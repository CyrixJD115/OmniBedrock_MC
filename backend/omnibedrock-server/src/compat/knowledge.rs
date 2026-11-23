//! What each rule means and how to fix it. Explanations are grounded in
//! Microsoft Learn and Bedrock Wiki pages (verified links below); the panel
//! adds pack-specific lines (declared module versions, source excerpts) when
//! it builds the report.

use super::types::DocLink;

pub struct Knowledge {
    pub explanation: &'static str,
    pub fixes: &'static [&'static str],
    pub docs: &'static [DocLink],
}

const V2: DocLink = DocLink {
    title: "Scripting V2 overview (1.x → 2.0 changes)",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/documents/scripting/v2-overview",
};
const VERSIONING: DocLink = DocLink {
    title: "Script module versioning",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/documents/scripting/versioning",
};
const CHANGELOG: DocLink = DocLink {
    title: "@minecraft/server changelog",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/scriptapi/minecraft/server/changelog",
};
const INVALID: DocLink = DocLink {
    title: "InvalidEntityError",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/scriptapi/minecraft/server/invalidentityerror",
};
const MANIFEST: DocLink = DocLink {
    title: "manifest.json reference",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/reference/content/addonsreference/examples/addonmanifest",
};
const EXPERIMENTS: DocLink = DocLink {
    title: "Experimental features (Beta APIs)",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/documents/experimentalfeaturestoggle",
};
const CMDS_MS: DocLink = DocLink {
    title: "Script custom commands",
    url: "https://learn.microsoft.com/en-us/minecraft/creator/documents/scripting/custom-commands",
};
const CMDS_WIKI: DocLink = DocLink {
    title: "Custom commands (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/scripting/custom-commands",
};
const TROUBLE: DocLink = DocLink {
    title: "Troubleshooting & content log (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/guide/troubleshooting#content-log",
};
const JSON: DocLink = DocLink {
    title: "Understanding JSON (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/guide/understanding-json",
};
const NAMESPACES: DocLink = DocLink {
    title: "Namespaces (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/concepts/namespaces",
};
const FEATURES: DocLink = DocLink {
    title: "Feature types (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/world-generation/feature-types",
};
const RECIPES: DocLink = DocLink {
    title: "Recipes (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/loot/recipes",
};
const BLOCK_STATES: DocLink = DocLink {
    title: "Block states (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/blocks/block-states",
};
const SCRIPTING: DocLink = DocLink {
    title: "Intro to scripting (Bedrock Wiki)",
    url: "https://wiki.bedrock.dev/scripting/scripting-intro",
};
const ENDSTONE: DocLink = DocLink {
    title: "Endstone documentation",
    url: "https://endstone.dev/latest/",
};

pub fn lookup(rule: &str) -> Knowledge {
    match rule {
        "script.invalid_entity" => Knowledge {
            explanation: "The pack's script keeps a reference to an entity after it was removed, despawned or unloaded, then reads it again (often every tick). Since @minecraft/server 2.0.0, getComponent / getComponents / hasComponent, EntityComponent.entity and EntityInventoryComponent.container throw InvalidEntityError on an invalid entity instead of returning undefined. A pack written for 1.x semantics but declaring a 2.x module floods the log this way.",
            fixes: &[
                "Pack author: check `entity.isValid` (a property in 2.x, a method in 1.x) before every access, and drop stale references from maps/arrays when entities are removed.",
                "Update the pack to a release built for the server's script API version, or ask the author for one.",
                "If the pack is not essential, disable it on this world until it is fixed: these errors run every tick and cost CPU and disk.",
            ],
            docs: &[INVALID, V2, CHANGELOG],
        },
        "script.native_call_failed" | "script.native_property_failed" => Knowledge {
            explanation: "A native API call threw. Usual causes: the object is no longer valid, the component does not exist on that entity/block (getComponent returned undefined), the call happened in read-only (before-event) mode, or the method's arguments changed between module versions.",
            fixes: &[
                "Pack author: guard the call (isValid, check the component is present) and wrap per-entity work in try/catch so one bad entity does not abort the whole loop.",
                "Compare the pack's declared @minecraft/server version with the changelog for that method.",
                "Update or temporarily disable the pack if the errors repeat every tick.",
            ],
            docs: &[CHANGELOG, V2],
        },
        "script.privileges" => Knowledge {
            explanation: "Before-events and early execution run in a restricted mode in which world-mutating APIs are not allowed. The script calls one of them there.",
            fixes: &[
                "Pack author: defer the mutation with `system.run(() => …)` from the before-event handler.",
                "Move world access out of `system.beforeEvents.startup` into `world.afterEvents.worldLoad`.",
            ],
            docs: &[V2],
        },
        "script.early_execution" => Knowledge {
            explanation: "Since 2.0.0 scripts start in early execution: most of the `world` object throws until the world has loaded. Code at module top level (or a getter it calls) touches the world too early.",
            fixes: &[
                "Pack author: run world-dependent setup in `world.afterEvents.worldLoad` (formerly worldInitialize) or inside `system.run`.",
                "Use a pack release targeting the server's script API version.",
            ],
            docs: &[V2],
        },
        "script.missing_export" => Knowledge {
            explanation: "The script imports a name that @minecraft/server does not export at the declared version, so the importing module fails to load entirely. Example: `Vector3` is a TypeScript interface, not a runtime class; old packs that construct it break.",
            fixes: &[
                "Pack author: remove the import and use a plain `{ x, y, z }` object (or @minecraft/math helpers bundled into the pack).",
                "Update the pack to a release built for the server's script API version.",
            ],
            docs: &[CHANGELOG, VERSIONING],
        },
        "script.not_a_function" => Knowledge {
            explanation: "The script calls something that is no longer a function at the declared module version. The classic 2.0.0 case is `isValid()`, now a read-only property, plus removed methods such as runCommandAsync.",
            fixes: &[
                "Pack author: replace `x.isValid()` with `x.isValid`, `runCommandAsync` with `runCommand` (or system.runJob), and check the changelog for other renames.",
                "Or keep the pack on a 1.x module version, which the server still supports.",
            ],
            docs: &[V2, CHANGELOG],
        },
        "script.location_error" => Knowledge {
            explanation: "The script reads or writes a block/location below or above the dimension's height limits, or in a chunk that is not loaded. Typical for code that scans around players or machines without bounds checks.",
            fixes: &[
                "Pack author: clamp Y to dimension.heightRange and check dimension.isChunkLoaded(location) (or catch LocationInUnloadedChunkError) before getBlock.",
                "Usually low impact unless the count keeps climbing.",
            ],
            docs: &[SCRIPTING, CHANGELOG],
        },
        "content.recipe" => Knowledge {
            explanation: "A recipe file failed validation, usually because an ingredient or result references an item that does not exist (missing pack, renamed item, or an item that needs an experiment).",
            fixes: &[
                "Check that every item named in the recipe exists (install the pack defining it).",
                "Update the pack that ships the recipe.",
            ],
            docs: &[RECIPES, NAMESPACES],
        },
        "script.undefined_access" => Knowledge {
            explanation: "The script reads a property of undefined/null. It usually expected a component or object the API no longer returns (or never returned for that entity type).",
            fixes: &[
                "Pack author: null-check API results (getComponent, getBlock, getEntity…) before use.",
                "Update the pack.",
            ],
            docs: &[SCRIPTING, CHANGELOG],
        },
        "script.runtime_error" => Knowledge {
            explanation: "A script in this pack throws. The message and first stack frame show where.",
            fixes: &[
                "Check whether the pack has an update for this server version.",
                "Report the message and stack frame to the pack author.",
                "Disable the pack on this world if it floods the log.",
            ],
            docs: &[SCRIPTING, TROUBLE],
        },
        "script.watchdog" => Knowledge {
            explanation: "Script or server work is taking long enough to trip the watchdog: ticks are delayed and players see lag.",
            fixes: &[
                "Find the heaviest pack via the per-pack counts and disable it to compare.",
                "Pack author: spread work over ticks with system.runJob.",
            ],
            docs: &[SCRIPTING],
        },
        "manifest.module_unresolved" | "manifest.module_too_new" => Knowledge {
            explanation: "The pack depends on a script module version this server build does not provide, so its scripts do not load at all. Stable versions are backward compatible: a server supports every stable version up to its own maximum.",
            fixes: &[
                "Update the server (Version switcher) to a build that ships the requested module version, or",
                "Install a pack release targeting an older module version, or lower the version in manifest.json only if the script does not use newer APIs.",
            ],
            docs: &[VERSIONING, MANIFEST],
        },
        "manifest.beta_mismatch" => Knowledge {
            explanation: "Beta script modules are tied to one exact game build. A pack pinned to another build's beta (or to an unversioned 'beta') breaks after every server update.",
            fixes: &[
                "Use the pack release made for this exact server build, or",
                "Change the dependency to the beta version this server provides (shown above) if the scripts are compatible, and keep the Beta APIs experiment on.",
            ],
            docs: &[VERSIONING, EXPERIMENTS],
        },
        "manifest.beta_floating" => Knowledge {
            explanation: "The dependency version is the bare string \"beta\", so the pack gets whichever beta API the server build ships. Beta APIs change without notice between builds, so the pack can break on any server update even though nothing in the pack changed.",
            fixes: &[
                "After each server update, check this pack's script errors first.",
                "Pack author: pin a concrete version (stable if possible) so breakage is explicit.",
            ],
            docs: &[VERSIONING, EXPERIMENTS],
        },
        "manifest.beta_experiment" => Knowledge {
            explanation: "Beta script modules only load in worlds with the 'Beta APIs' experiment enabled.",
            fixes: &[
                "Enable Beta APIs on the world (re-export it from a client with the toggle on), or use a pack release that only needs stable modules.",
            ],
            docs: &[EXPERIMENTS, VERSIONING],
        },
        "manifest.engine_too_new" => Knowledge {
            explanation: "min_engine_version is newer than the server. The pack was built for a newer game and may use content or components this server does not know.",
            fixes: &[
                "Update the server to at least that version, or install an older release of the pack.",
            ],
            docs: &[MANIFEST],
        },
        "manifest.legacy_engine" => Knowledge {
            explanation: "The pack targets a much older engine. Content still loads in compatibility mode, but components whose schema changed since then produce [Json] errors and some behaviour silently changes.",
            fixes: &[
                "Look for a newer release of the pack.",
                "Check this pack's content-log issues below. They show exactly which components failed.",
            ],
            docs: &[MANIFEST, JSON],
        },
        "manifest.legacy_scripts" => Knowledge {
            explanation: "The pack uses the 1.x script API. The server still runs it, but with 1.x semantics. Mixing 1.x and 2.x packs is fine. This is informational.",
            fixes: &[
                "No action needed unless the pack misbehaves; prefer updated releases when available.",
            ],
            docs: &[V2, VERSIONING],
        },
        "manifest.missing_dependency" | "pack.missing" => Knowledge {
            explanation: "A pack (or the world's pack list) references a pack uuid that is not installed. The dependent pack then loads partially or not at all.",
            fixes: &[
                "Install the missing pack (usually the matching resource or behavior half of the add-on).",
                "Or remove the stale reference from the world's pack list on the Active tab.",
            ],
            docs: &[MANIFEST],
        },
        "manifest.duplicate_uuid" => Knowledge {
            explanation: "Two installed packs share a header uuid. BDS loads only one of them, and which one depends on load order.",
            fixes: &[
                "Delete the older copy, or give one a fresh uuid (Details → Randomize UUID) if both are genuinely different packs.",
            ],
            docs: &[MANIFEST],
        },
        "manifest.invalid" => Knowledge {
            explanation: "manifest.json is missing or not valid JSON, so the pack cannot load.",
            fixes: &["Fix the manifest in the Details tab editor, or reinstall the pack."],
            docs: &[MANIFEST, JSON],
        },
        "manifest.script_eval" => Knowledge {
            explanation: "The pack declares the script_eval capability (it runs eval / Function()). It is allowed but worth knowing about on a public server.",
            fixes: &["No action needed if you trust the pack."],
            docs: &[MANIFEST],
        },
        "scan.is_valid_call" | "scan.removed_api" | "scan.vector3_import" => Knowledge {
            explanation: "Static scan: the pack's JavaScript uses an API form that the module version it declares no longer has. These lines will throw when reached.",
            fixes: &[
                "Pack author: migrate the listed calls (see the V2 overview), or declare a 1.x module version if the pack was never ported.",
            ],
            docs: &[V2, CHANGELOG],
        },
        "scan.no_validity_guards" => Knowledge {
            explanation: "Static scan: the pack reads entity components in many places but almost never checks isValid. With 2.x semantics every access to a removed entity throws. This is the usual cause of an InvalidEntityError flood.",
            fixes: &[
                "Pack author: add `if (!entity.isValid) return;` guards in tick loops and event handlers.",
            ],
            docs: &[INVALID, V2],
        },
        "command.alias_conflict" | "command.conflict" => Knowledge {
            explanation: "Two packs/plugins register the same command name. Only the first keeps the short alias; the other is reachable only by its namespaced name.",
            fixes: &[
                "Use the full namespaced name shown in the message, or remove one of the packs. Harmless otherwise.",
            ],
            docs: &[CMDS_MS, CMDS_WIKI],
        },
        "content.unknown_block" => Knowledge {
            explanation: "Some content (features, loot, recipes, block lists) references blocks under a namespace that no loaded pack defines. Typically an add-on depends on another add-on that is not installed, or a pack was removed but packs that reference it were not.",
            fixes: &[
                "Install the add-on that owns this namespace, or remove the pack that references it (see 'referenced by').",
                "Check the namespace spelling. Identifiers are case-sensitive.",
            ],
            docs: &[NAMESPACES, BLOCK_STATES],
        },
        "content.missing_feature" => Knowledge {
            explanation: "Feature rules place features whose definition files do not exist. The pack shipped incomplete, or two versions of it are mixed.",
            fixes: &[
                "Reinstall a clean copy of the pack (delete the old folder first).",
                "Make sure only one version of the add-on is installed.",
            ],
            docs: &[FEATURES],
        },
        "content.duplicate_feature" => Knowledge {
            explanation: "The same feature identifier is defined twice: two copies of one add-on, or two add-ons sharing a namespace.",
            fixes: &["Remove the duplicate pack copy."],
            docs: &[FEATURES, NAMESPACES],
        },
        "content.recipe_conflict" => Knowledge {
            explanation: "Two recipes take identical ingredients but produce different items, so one of them is unreachable from the crafting table.",
            fixes: &[
                "Usually harmless. Remove one of the packs, or edit one recipe's pattern, if the shadowed item matters.",
            ],
            docs: &[RECIPES],
        },
        "content.item_category" => Knowledge {
            explanation: "Two creative-catalog entries place the same item in different categories. Cosmetic only.",
            fixes: &["No action needed."],
            docs: &[TROUBLE],
        },
        "performance.block_permutations" => Knowledge {
            explanation: "Custom blocks multiply by their states into permutations. Past 65,536, the game warns that performance degrades (memory, world load and join time).",
            fixes: &[
                "Remove block-heavy add-ons you do not need (decoration and furniture packs are usual suspects).",
                "Ensure `block-network-ids-are-hashes=true` stays set.",
            ],
            docs: &[BLOCK_STATES],
        },
        "content.json_schema"
        | "content.molang"
        | "content.animation"
        | "content.geometry"
        | "content.texture"
        | "content.other"
        | "content.feature" => Knowledge {
            explanation: "Content-log error: a pack file failed validation. The path shows pack | object | component chain. Usually the pack targets an older format_version whose schema changed.",
            fixes: &[
                "Update the pack.",
                "Or edit the file at the component path shown (Files tab) to match the current schema.",
            ],
            docs: &[TROUBLE, JSON],
        },
        "plugin.traceback" | "plugin.error" => Knowledge {
            explanation: "An Endstone plugin raised an error. Plugins are built against a specific Endstone API. After an Endstone update, plugins that are not updated commonly break.",
            fixes: &[
                "Update the plugin to a build for this Endstone version.",
                "Or pin Endstone to the version the plugin supports (Version switcher).",
            ],
            docs: &[ENDSTONE],
        },
        "housekeeping.content_logs" => Knowledge {
            explanation: "BDS writes a ContentLog file per start when content-log-file-enabled=true. With noisy packs these grow to hundreds of MB each and are never rotated.",
            fixes: &[
                "Delete old content logs (button below). The newest is kept.",
                "Set content-log-file-enabled=false in Properties once you are done debugging.",
            ],
            docs: &[TROUBLE],
        },
        "housekeeping.log_flood" => Knowledge {
            explanation: "One repeating error is writing hundreds of thousands of lines per day: disk churn, slower log search, and CPU spent formatting errors every tick.",
            fixes: &[
                "Fix or disable the pack behind the top issue. This is the single highest-impact change.",
            ],
            docs: &[TROUBLE],
        },
        _ => Knowledge {
            explanation: "The server reported this at warning/error level. It did not match a known pattern.",
            fixes: &["Search the message on Bedrock Wiki / the pack's support page."],
            docs: &[TROUBLE],
        },
    }
}
