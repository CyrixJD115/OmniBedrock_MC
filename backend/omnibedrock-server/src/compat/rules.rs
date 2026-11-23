//! Log-event classification. Each rule maps one recognisable server
//! message to a stable rule id, a severity, a pack attribution hint and a
//! grouping key, so hundreds of thousands of repeats collapse into one issue.

use super::parse::{
    Level, balanced_bracket, identifiers, namespace, normalize, quoted, split_inline_frames,
};
use super::types::{Hint, Location, Severity};

#[derive(Clone, Debug)]
pub struct Classified {
    pub rule: &'static str,
    pub severity: Severity,
    pub category: &'static str,
    pub title: String,
    pub detail: String,
    pub key: String,
    pub hint: Option<Hint>,
    pub location: Option<Location>,
    /// A distinct value worth listing as a sample (e.g. each unknown block).
    pub sample: Option<String>,
}

/// Content-log channels BDS prefixes messages with (`[Json] …`).
const CONTENT_TAGS: &[&str] = &[
    "Json",
    "FeatureRegistry",
    "Recipes",
    "Item",
    "Items",
    "Blocks",
    "Block",
    "Molang",
    "Animation",
    "Animations",
    "Geometry",
    "Texture",
    "Textures",
    "Biome",
    "Biomes",
    "Spawning",
    "Entity",
    "Actor",
    "Loot",
    "LootTable",
    "Commands",
    "Rendering",
    "Particle",
    "Particles",
    "Sound",
    "Sounds",
    "UI",
    "Pack",
    "Packs",
    "Resource",
    "Behavior",
    "World",
    "Volumes",
    "Structure",
    "Structures",
    "Dimension",
    "FogDefinitions",
    "Camera",
    "Materials",
    "Trading",
    "Recipe",
    "Components",
    "Attachables",
    "Localization",
];

#[allow(clippy::too_many_arguments)]
fn mk(
    rule: &'static str,
    severity: Severity,
    category: &'static str,
    title: String,
    detail: String,
    hint: Option<Hint>,
    location: Option<Location>,
    group: &str,
) -> Classified {
    let hint_key = hint.as_ref().map(Hint::label).unwrap_or_default();
    let loc_key = location
        .as_ref()
        .map(|l| {
            format!(
                "{}@{}",
                l.function.as_deref().unwrap_or(""),
                l.file.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    Classified {
        key: format!("{rule}|{hint_key}|{group}|{loc_key}"),
        rule,
        severity,
        category,
        title,
        detail,
        hint,
        location,
        sample: None,
    }
}

/// Classify one assembled event (message plus any continuation frames).
pub fn classify(
    level: Level,
    msg: &str,
    frames: &[Location],
    plugins: &[String],
) -> Option<Classified> {
    let msg = msg.trim();
    if msg.is_empty() {
        return None;
    }
    if let Some(rest) = msg.strip_prefix("[Scripting] ") {
        return scripting(level, rest, frames);
    }
    if msg.contains("Traceback (most recent call last)") {
        let plugin = frames
            .iter()
            .find_map(|f| f.file.clone())
            .and_then(|f| plugin_from_path(&f, plugins));
        return Some(mk(
            "plugin.traceback",
            Severity::Error,
            "plugin",
            "Python traceback in an Endstone plugin".into(),
            msg.into(),
            plugin.map(Hint::Plugin),
            frames.first().cloned(),
            "",
        ));
    }
    if let Some(name) = msg
        .strip_prefix("Unable to register command '")
        .and_then(|r| r.split_once('\'').map(|(n, _)| n))
    {
        return Some(mk(
            "command.conflict",
            Severity::Warning,
            "commands",
            format!("Command /{name} registered twice"),
            msg.into(),
            None,
            None,
            name,
        ));
    }
    if msg.contains("[Watchdog]") || msg.to_ascii_lowercase().contains("watchdog") {
        let n = normalize(msg, true);
        return Some(mk(
            "script.watchdog",
            Severity::Warning,
            "performance",
            "Watchdog: slow or hanging tick".into(),
            n.clone(),
            None,
            None,
            &n,
        ));
    }
    if msg.contains("was not found and was ignored")
        || msg.contains("could not be found") && msg.contains("pack")
    {
        let n = normalize(msg, false);
        let hint = identifiers_or_uuid(msg);
        return Some(mk(
            "pack.missing",
            Severity::Error,
            "packs",
            "A world references a pack that is not installed".into(),
            msg.into(),
            hint,
            None,
            &n,
        ));
    }
    if let Some((tag, rest)) = balanced_bracket(msg) {
        if CONTENT_TAGS.contains(&tag) {
            return content(level, tag, rest);
        }
        if level >= Level::Warn && plugins.iter().any(|p| p.eq_ignore_ascii_case(tag)) {
            let n = normalize(rest, true);
            return Some(mk(
                "plugin.error",
                if level == Level::Error {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                "plugin",
                format!("Endstone plugin {tag} reported a problem"),
                rest.into(),
                Some(Hint::Plugin(tag.into())),
                None,
                &n,
            ));
        }
    }
    if level >= Level::Warn {
        let n = normalize(msg, true);
        let (rule, severity) = if level == Level::Error {
            ("general.error", Severity::Error)
        } else {
            ("general.warning", Severity::Warning)
        };
        let title = n.chars().take(90).collect::<String>();
        return Some(mk(
            rule,
            severity,
            "server",
            title,
            msg.into(),
            None,
            None,
            &n,
        ));
    }
    None
}

fn identifiers_or_uuid(msg: &str) -> Option<Hint> {
    let bytes = msg.as_bytes();
    (0..bytes.len().saturating_sub(35))
        .find(|&i| {
            bytes[i..i + 36].iter().enumerate().all(|(k, c)| match k {
                8 | 13 | 18 | 23 => *c == b'-',
                _ => c.is_ascii_hexdigit(),
            })
        })
        .map(|i| Hint::Uuid(msg[i..i + 36].to_ascii_lowercase()))
}

fn plugin_from_path(path: &str, plugins: &[String]) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    plugins
        .iter()
        .find(|p| lower.contains(&p.to_ascii_lowercase()))
        .cloned()
}

/// `[Scripting] [<pack>] <message>` and pack-less scripting notices.
fn scripting(level: Level, rest: &str, frames: &[Location]) -> Option<Classified> {
    if let Some(body) = rest.strip_prefix("Custom Command alias [") {
        let alias = body.split(']').next().unwrap_or_default();
        let full = rest
            .rsplit_once("full name [")
            .and_then(|(_, f)| f.split(']').next());
        let hint = full
            .and_then(namespace)
            .map(|ns| Hint::Namespace(ns.into()));
        return Some(mk(
            "command.alias_conflict",
            Severity::Warning,
            "commands",
            format!("Two packs both claim the /{alias} command alias"),
            rest.into(),
            hint,
            None,
            alias,
        ));
    }
    let (pack, body) = match balanced_bracket(rest) {
        Some((p, b)) if !matches!(p, "LOG" | "System" | "INFO" | "DEBUG") => (Some(p.trim()), b),
        Some((_, b)) => (None, b),
        None => (None, rest),
    };
    let (message, inline) = split_inline_frames(body);
    let message = message
        .strip_prefix("Unhandled promise rejection: ")
        .map(|m| m.trim())
        .unwrap_or(message);
    let has_error =
        message.contains("Error:") || message.contains("Error ") || message.starts_with("Error");
    if level < Level::Warn && !has_error {
        return None;
    }
    let location = inline.or_else(|| frames.first().cloned());
    let hint = pack.map(|p| Hint::Name(p.to_owned()));
    let lower = message.to_ascii_lowercase();
    let func = quoted(message).map(str::to_owned);
    let n = normalize(message, false);
    let c = if message.contains("InvalidEntityError") || lower.contains("entity being invalid") {
        let what = func.clone().unwrap_or_else(|| "an entity".into());
        mk(
            "script.invalid_entity",
            Severity::Error,
            "script",
            format!("Script uses a removed entity ({what})"),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("does not have required privileges") {
        mk(
            "script.privileges",
            Severity::Error,
            "script",
            "Script calls a restricted API from a before-event or early execution".into(),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("is not initialized") || lower.contains("early execution") {
        mk(
            "script.early_execution",
            Severity::Error,
            "script",
            "Script touches the world before it has loaded".into(),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if message.contains("SyntaxError") && lower.contains("could not find export") {
        let export = quoted(message).unwrap_or("?");
        mk(
            "script.missing_export",
            Severity::Critical,
            "script",
            format!("Script imports '{export}', which the server's API no longer exports"),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if message.contains("LocationOutOfWorldBoundariesError")
        || message.contains("LocationInUnloadedChunkError")
        || message.contains("UnloadedChunksError")
    {
        mk(
            "script.location_error",
            Severity::Error,
            "script",
            "Script reads a location outside the world or in an unloaded chunk".into(),
            message.into(),
            hint,
            location,
            &normalize(message.split(" which").next().unwrap_or(message), false),
        )
    } else if lower.contains("is not a function") {
        mk(
            "script.not_a_function",
            Severity::Error,
            "script",
            "Script calls an API that changed shape (not a function)".into(),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("of undefined")
        || lower.contains("of null")
        || lower.contains("cannot read property")
    {
        mk(
            "script.undefined_access",
            Severity::Error,
            "script",
            "Script reads a property of undefined".into(),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("failed to call function") {
        let what = func.clone().unwrap_or_else(|| "a native function".into());
        mk(
            "script.native_call_failed",
            Severity::Error,
            "script",
            format!("Native call {what}() failed"),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("failed to get property") || lower.contains("failed to set property") {
        let what = func.clone().unwrap_or_else(|| "a property".into());
        mk(
            "script.native_property_failed",
            Severity::Error,
            "script",
            format!("Native property '{what}' access failed"),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("module")
        && (lower.contains("not found")
            || lower.contains("unsupported")
            || lower.contains("could not")
            || lower.contains("invalid"))
    {
        mk(
            "manifest.module_unresolved",
            Severity::Critical,
            "manifest",
            "Script module version could not be loaded".into(),
            message.into(),
            hint,
            location,
            &n,
        )
    } else if lower.contains("spike") || lower.contains("hang") || lower.contains("too long") {
        mk(
            "script.watchdog",
            Severity::Warning,
            "performance",
            "Script is slowing the server tick".into(),
            message.into(),
            hint,
            location,
            &n,
        )
    } else {
        let short = n.chars().take(80).collect::<String>();
        mk(
            "script.runtime_error",
            if level == Level::Error || has_error {
                Severity::Error
            } else {
                Severity::Warning
            },
            "script",
            format!("Script error: {short}"),
            message.into(),
            hint,
            location,
            &n,
        )
    };
    Some(c)
}

/// The pack folder in a content-log path segment
/// (`worlds/<world>/behavior_packs/<folder>`).
fn folder_hint(s: &str) -> Option<Hint> {
    for marker in ["behavior_packs/", "resource_packs/"] {
        if let Some(i) = s.find(marker) {
            let rest = &s[i + marker.len()..];
            // Folder names may contain spaces: cut at the first "/" or " |".
            let end = [rest.find('/'), rest.find(" |")]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(rest.len());
            let folder = rest[..end].trim();
            if !folder.is_empty() {
                return Some(Hint::Folder(folder.to_owned()));
            }
        }
    }
    None
}

fn content(level: Level, tag: &str, rest: &str) -> Option<Classified> {
    if level < Level::Warn {
        return None;
    }
    let sev = if level == Level::Error {
        Severity::Error
    } else {
        Severity::Warning
    };
    if let Some(id) =
        rest.strip_prefix("Unknown block during Deferred BlockDescriptor resolution: ")
    {
        let id = id.trim();
        let ns = namespace(id).unwrap_or("?").to_owned();
        let mut c = mk(
            "content.unknown_block",
            Severity::Error,
            "content",
            format!("References blocks from '{ns}:' that no installed pack defines"),
            format!("Unknown block during Deferred BlockDescriptor resolution ({ns}:*)"),
            Some(Hint::Namespace(ns.clone())),
            None,
            &ns,
        );
        c.sample = Some(id.to_owned());
        return Some(c);
    }
    if tag == "FeatureRegistry" {
        let id = quoted(rest).unwrap_or_default().to_owned();
        let ns = namespace(&id).unwrap_or("?").to_owned();
        let (rule, title) = if rest.contains("No definition found") {
            (
                "content.missing_feature",
                format!("Feature rules point at '{ns}:' features that do not exist"),
            )
        } else if rest.contains("already exists") {
            (
                "content.duplicate_feature",
                format!("'{ns}:' features are defined twice"),
            )
        } else {
            (
                "content.feature",
                format!("Feature registry problem in '{ns}:'"),
            )
        };
        let mut c = mk(
            rule,
            sev,
            "content",
            title,
            normalize(rest, true),
            Some(Hint::Namespace(ns.clone())),
            None,
            &ns,
        );
        c.sample = Some(id);
        return Some(c);
    }
    if tag == "Recipes" && rest.contains("has the same ingredients as") {
        let ids = identifiers(rest);
        let own = ids
            .iter()
            .find(|i| !i.ends_with(".json"))
            .copied()
            .unwrap_or_default();
        let other = rest
            .split("same ingredients as ")
            .nth(1)
            .and_then(|r| identifiers(r).first().copied())
            .unwrap_or_default();
        let a = namespace(own).unwrap_or("?").to_owned();
        let b = namespace(other).unwrap_or("?").to_owned();
        let mut c = mk(
            "content.recipe_conflict",
            Severity::Warning,
            "content",
            format!("Recipes in '{a}:' collide with '{b}:'"),
            normalize(rest, true),
            Some(Hint::Namespace(a.clone())),
            None,
            &format!("{a}>{b}"),
        );
        c.sample = Some(format!("{own} vs {other}"));
        return Some(c);
    }
    if tag == "Item" && rest.contains("was created with the category set to") {
        let mut c = mk(
            "content.item_category",
            Severity::Info,
            "content",
            "Two catalog entries put the same item in different creative categories".into(),
            normalize(rest, true),
            folder_hint(rest),
            None,
            "",
        );
        c.sample = identifiers(rest).first().map(|s| (*s).to_owned());
        return Some(c);
    }
    if tag == "Blocks" && rest.contains("block permutations") {
        let n = rest
            .rsplit_once("has ")
            .map(|(_, n)| n.trim_end_matches(['.', ' ']).replace(" permutations", ""));
        let mut c = mk(
            "performance.block_permutations",
            Severity::Warning,
            "performance",
            "Too many custom block permutations".into(),
            rest.into(),
            None,
            None,
            "",
        );
        c.sample = n;
        return Some(c);
    }
    // Generic `<world> | <subsystem> | <pack path> | <id> | <component path…>`
    // or `<file> | <id> | <message>`: attribute by pack folder, else by the
    // first non-vanilla identifier's namespace; title from the message tail.
    let parts: Vec<&str> = rest.split(" | ").collect();
    let hint = folder_hint(rest).or_else(|| {
        identifiers(rest)
            .into_iter()
            .filter(|i| !i.starts_with("minecraft:") && !i.contains(".json"))
            .find_map(namespace)
            .map(|ns| Hint::Namespace(ns.to_owned()))
    });
    let (subject, message) = if tag == "Json" && parts.len() > 4 {
        // Component path after the object id, e.g. minecraft:breedable › breedsWith.
        (parts.get(1).copied().unwrap_or(tag), parts[4..].join(" › "))
    } else {
        (
            parts.first().copied().unwrap_or(tag),
            parts.last().copied().unwrap_or(rest).to_owned(),
        )
    };
    let group = normalize(&message, true);
    let short: String = group.chars().take(90).collect();
    let mut c = mk(
        match tag {
            "Json" => "content.json_schema",
            "Molang" => "content.molang",
            "Animation" | "Animations" => "content.animation",
            "Geometry" => "content.geometry",
            "Texture" | "Textures" => "content.texture",
            "Recipes" | "Recipe" => "content.recipe",
            _ => "content.other",
        },
        sev,
        "content",
        if tag == "Json" {
            format!("[Json] {subject}: {short}")
        } else {
            format!("[{tag}] {short}")
        },
        rest.into(),
        hint,
        None,
        &format!("{tag}|{group}"),
    );
    c.sample = parts
        .iter()
        .find(|p| identifiers(p).len() == 1 && !p.contains(".json"))
        .or(parts.first())
        .map(|s| (*s).to_owned());
    Some(c)
}
