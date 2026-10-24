//! Ensures the embedded panel UI exists before the crate compiles.
//!
//! `include_dir!` bakes `frontend/build` into the binary at compile time, so
//! this script runs `npm run build` when the folder is missing (fresh clone,
//! CI without a prior frontend step) and falls back to a placeholder page if
//! npm is unavailable, keeping `cargo build` working everywhere. Set
//! `OMNI_SKIP_FRONTEND_BUILD=1` to never invoke npm.
use std::path::PathBuf;
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
    println!("cargo:rerun-if-env-changed=OMNI_SKIP_FRONTEND_BUILD");
    if !index.exists() && std::env::var_os("OMNI_SKIP_FRONTEND_BUILD").is_none() {
        println!("cargo:warning=frontend/build is missing; running `npm run build` in frontend/");
        let status = Command::new("npm")
            .args(["run", "build"])
            .current_dir(&frontend)
            .status();
        match status {
            Ok(status) if status.success() => {}
            _ => println!(
                "cargo:warning=frontend build failed; bundling a placeholder UI. \
                 Run `npm run build` in frontend/ and rebuild to embed the panel."
            ),
        }
    }
    if !index.exists() {
        let _ = std::fs::create_dir_all(&build_dir);
        let _ = std::fs::write(index, PLACEHOLDER);
    }
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
