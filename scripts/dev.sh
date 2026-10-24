#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
backend_pid=''
frontend_pid=''
cleanup() {
  trap - EXIT INT TERM
  if [[ -n "$frontend_pid" ]]; then kill "$frontend_pid" 2>/dev/null || true; fi
  if [[ -n "$backend_pid" ]]; then kill "$backend_pid" 2>/dev/null || true; fi
  wait || true
}
trap cleanup EXIT INT TERM
# The dev UI comes up first so Vite warms while the backend compiles.
if [[ ! -f frontend/vite.config.ts ]]; then cp frontend/vite.config.example.ts frontend/vite.config.ts; fi
if [[ ! -d frontend/node_modules ]]; then
  echo "[dev] installing frontend dependencies..."
  (cd frontend && npm ci --no-audit --no-fund)
fi
(cd frontend && exec npm run dev) &
frontend_pid=$!
echo "[dev] building backend (debug)..."
cargo build --locked
echo "[dev] starting backend on :${OMNI_PORT:-17754}"
./target/debug/omnibedrock &
backend_pid=$!
wait -n "$backend_pid" "$frontend_pid"
