#!/usr/bin/env bash
# Package the panel as a single self-contained binary. The web UI is built
# automatically (frontend/build) and embedded by the Rust build script.
#
# FEATURES="desktop" also builds in the `--desktop` Tauri window mode; the
# resulting binary then needs webkit2gtk-4.1 at runtime (Linux).
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
FEATURES=${FEATURES:-}
if [[ -n "$FEATURES" ]]; then
  cargo build --release --locked --features "$FEATURES"
else
  cargo build --release --locked
fi
mkdir -p dist/omnibedrock
cp target/release/omnibedrock README.md LICENSE .env.example dist/omnibedrock/
cp deploy/config.example.yaml deploy/omnibedrock.service dist/omnibedrock/
tar -czf dist/omnibedrock-linux.tar.gz -C dist omnibedrock
