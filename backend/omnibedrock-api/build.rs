//! Ensures the embedded panel UI exists (and is fresh) before the crate
//! compiles.
//!
//! `include_dir!` bakes `frontend/build` into the binary at compile time, so
//! this script builds the UI when it is missing or older than `frontend/src`.
//! On a real checkout (`frontend/src` exists) a failed UI build is a HARD
//! ERROR: silently bundling a placeholder has shipped "working" binaries
//! whose UI was a stub. The placeholder fallback remains only for
//! source-less trees (Docker cargo stage, packaged crates). Set
//! `OMNI_SKIP_FRONTEND_BUILD=1` to never invoke npm and accept the
//! placeholder.
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let frontend = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crate lives two levels below the workspace root")
        .join("frontend");
    let build_dir = frontend.join("build");
    let index = build_dir.join("index.html");
    println!("cargo:rerun-if-changed={}", build_dir.display());
    println!("cargo:rerun-if-changed={}", frontend.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        frontend.join("package.json").display()
    );
    println!("cargo:rerun-if-env-changed=OMNI_SKIP_FRONTEND_BUILD");

    let sources_exist = frontend.join("src").exists();
    let skip = std::env::var_os("OMNI_SKIP_FRONTEND_BUILD").is_some();
    let stale = sources_exist && index.exists() && newer_than(&frontend.join("src"), &index);
    if sources_exist && !skip && (stale || !index.exists()) {
        if stale {
            println!("cargo:warning=frontend/src is newer than frontend/build; rebuilding the UI");
        } else {
            println!("cargo:warning=frontend/build is missing; building the UI in frontend/");
        }
        if !frontend.join("node_modules").exists() {
            // Fresh clone: vite is not installed, so `npm run build` would
            // fail on the missing binary before compiling a single file.
            println!("cargo:warning=node_modules is missing; running `npm ci` in frontend/");
            let installed = if frontend.join("package-lock.json").exists() {
                Command::new("npm")
                    .args(["ci", "--no-audit", "--no-fund"])
                    .current_dir(&frontend)
                    .status()
            } else {
                Command::new("npm")
                    .args(["install", "--no-audit", "--no-fund"])
                    .current_dir(&frontend)
                    .status()
            };
            match installed {
                Ok(status) if status.success() => {}
                _ => hard_fail(
                    "`npm ci` failed while installing frontend dependencies",
                    &frontend,
                ),
            }
        }
        // vite.config.ts is developer-local (gitignored, like the CI
        // workflow's own copy step). Without it vite falls back to root mode
        // and fails hunting for an index.html that a SvelteKit project never
        // has — so seed it from the tracked example first.
        let vite_config = frontend.join("vite.config.ts");
        let vite_example = frontend.join("vite.config.example.ts");
        if !vite_config.exists() && vite_example.exists() {
            println!("cargo:warning=vite.config.ts missing; copying vite.config.example.ts");
            let _ = std::fs::copy(&vite_example, &vite_config);
        }
        let built = Command::new("npm")
            .args(["run", "build"])
            .current_dir(&frontend)
            .status();
        match built {
            Ok(status) if status.success() => {}
            _ => hard_fail("`npm run build` failed in frontend/", &frontend),
        }
    }
    if !index.exists() {
        let _ = std::fs::create_dir_all(&build_dir);
        let _ = std::fs::write(index, PLACEHOLDER);
    }
}

/// Refuse to ship a placeholder when real UI sources are present. A cargo
/// warning scrolls past unseen; a red build cannot be missed.
fn hard_fail(what: &str, frontend: &Path) -> ! {
    eprintln!(
        "\nerror: {what}.\n\
         Refusing to bundle a placeholder UI over real frontend sources.\n\
         Fix the error above (is Node.js/npm installed?), or run \
         `npm ci && npm run build` in {frontend_display}/ yourself and retry,\n\
         or set OMNI_SKIP_FRONTEND_BUILD=1 to explicitly accept a \
         placeholder UI.\n",
        frontend_display = frontend.display()
    );
    std::process::exit(1)
}

/// Is any file under `dir` newer than `marker`? Cheap mtime walk; build
/// scripts have no dependency budget for this.
fn newer_than(dir: &Path, marker: &Path) -> bool {
    let marker_time = match marker.metadata().and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    fn walk(dir: &Path, marker_time: std::time::SystemTime) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if walk(&path, marker_time) {
                    return true;
                }
            } else if meta.modified().is_ok_and(|t| t > marker_time) {
                return true;
            }
        }
        false
    }
    walk(dir, marker_time)
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en" class="dark">
<head><meta charset="utf-8"><title>OmniBedrock</title></head>
<body style="background:#030810;color:#c8d3de;font-family:sans-serif;display:grid;place-items:center;height:100vh;margin:0">
  <div style="text-align:center;max-width:34rem">
    <h1 style="color:#fff">OmniBedrock API is running</h1>
    <p>The web UI was not embedded in this binary. Build it with
    <code style="color:#22d3ee">npm run build</code> in <code style="color:#22d3ee">frontend/</code>
    and rebuild the binary to bundle it.</p>
    <p>The API is served under <code style="color:#22d3ee">/api/v1</code>.</p>
  </div>
</body>
</html>
"#;
