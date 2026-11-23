//! Markdown rendering of a report, for sharing with pack authors or
//! pasting into an issue tracker.

use super::types::{CompatReport, Severity};
use std::fmt::Write;

fn sev(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "CRITICAL",
        Severity::Error => "ERROR",
        Severity::Warning => "WARNING",
        Severity::Info => "INFO",
    }
}

pub fn markdown(r: &CompatReport) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Add-on compatibility report\n");
    let _ = writeln!(s, "- Generated: {}", r.generated_at);
    if let Some(w) = &r.world {
        let _ = writeln!(s, "- World: {w}");
    }
    let _ = writeln!(
        s,
        "- Server: {}{}",
        r.server.version.as_deref().unwrap_or("unknown"),
        r.server
            .endstone_version
            .as_ref()
            .map(|e| format!(" · Endstone {e}"))
            .unwrap_or_default()
    );
    if !r.server.experiments.is_empty() {
        let _ = writeln!(s, "- Experiments: {}", r.server.experiments.join(", "));
    }
    for m in &r.script_api {
        let _ = writeln!(
            s,
            "- {}: stable ≤ {}{} ({})",
            m.module,
            m.max_stable.as_deref().unwrap_or("?"),
            if m.beta.is_empty() {
                String::new()
            } else {
                format!(", beta {}", m.beta.join("/"))
            },
            m.source
        );
    }
    let x = &r.summary;
    let _ = writeln!(
        s,
        "\n**{} critical · {} errors · {} warnings · {} info**: {} log events across {} files, {} packs affected.\n",
        x.critical, x.errors, x.warnings, x.info, x.events, x.files_scanned, x.packs_affected
    );
    if !r.timeline.is_empty() {
        let _ = writeln!(
            s,
            "## Timeline\n\n| Log | Server | Endstone | Lines | Errors | Warnings |\n|---|---|---|---:|---:|---:|"
        );
        for t in &r.timeline {
            let _ = writeln!(
                s,
                "| {}{} | {} | {} | {} | {} | {} |",
                t.label,
                if t.version_changed {
                    " ⟵ version change"
                } else {
                    ""
                },
                t.server_version.as_deref().unwrap_or("–"),
                t.endstone_version.as_deref().unwrap_or("–"),
                t.lines,
                t.errors,
                t.warnings
            );
        }
        s.push('\n');
    }
    let _ = writeln!(s, "## Issues\n");
    for i in &r.issues {
        let _ = writeln!(s, "### [{}] {}\n", sev(i.severity), i.title);
        if let Some(p) = &i.pack {
            let _ = writeln!(s, "- Pack: **{}** (`{}`, {})", p.name, p.folder, p.uuid);
        } else if let Some(h) = &i.hint {
            let _ = writeln!(s, "- Attributed to: {h}");
        }
        if i.count > 1 || i.source == super::types::Source::Log {
            let _ = writeln!(s, "- Occurrences: {}", i.count);
        }
        if let Some(f) = &i.first_seen {
            let _ = writeln!(
                s,
                "- First seen: {f}; last seen: {}",
                i.last_seen.as_deref().unwrap_or("–")
            );
        }
        if let Some(l) = &i.location {
            let _ = writeln!(
                s,
                "- Location: {} {}{}",
                l.function.as_deref().unwrap_or("<anonymous>"),
                l.file.as_deref().unwrap_or(""),
                l.line.map(|n| format!(":{n}")).unwrap_or_default()
            );
        }
        let _ = writeln!(s, "\n```\n{}\n```\n", i.detail);
        let _ = writeln!(s, "{}\n", i.explanation);
        let _ = writeln!(s, "**How to fix**\n");
        for f in &i.fixes {
            let _ = writeln!(s, "- {f}");
        }
        if !i.samples.is_empty() {
            let _ = writeln!(
                s,
                "\nSamples: {}",
                i.samples
                    .iter()
                    .map(|x| format!("`{x}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !i.docs.is_empty() {
            let _ = writeln!(
                s,
                "\nReferences: {}",
                i.docs
                    .iter()
                    .map(|d| format!("[{}]({})", d.title, d.url))
                    .collect::<Vec<_>>()
                    .join(" · ")
            );
        }
        s.push('\n');
    }
    s
}
