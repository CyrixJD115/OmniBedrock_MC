#!/usr/bin/env python3
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

from importlib.util import module_from_spec, spec_from_file_location

_launcher_spec = spec_from_file_location("omni_legacy_launcher", Path(__file__).resolve().parent / "backend-py" / "app" / "launcher.py")
assert _launcher_spec and _launcher_spec.loader
launcher = module_from_spec(_launcher_spec)
_launcher_spec.loader.exec_module(launcher)

FRONTEND_DIR = Path(__file__).resolve().parent / "frontend"
VITE_CONFIG = FRONTEND_DIR / "vite.config.ts"
VITE_CONFIG_EXAMPLE = FRONTEND_DIR / "vite.config.example.ts"
ROOT = Path(__file__).resolve().parent


def ensure_vite_config():
    if not VITE_CONFIG_EXAMPLE.exists():
        return
    if not VITE_CONFIG.exists():
        shutil.copy2(VITE_CONFIG_EXAMPLE, VITE_CONFIG)
        print(f"[frontend] Created {VITE_CONFIG.name} from {VITE_CONFIG_EXAMPLE.name}")
        print(f"[frontend] Edit {VITE_CONFIG.name} to customize ports, hosts, etc.")


def start_backend() -> subprocess.Popen:
    binary = ROOT / "target" / "debug" / ("omnibedrock.exe" if os.name == "nt" else "omnibedrock")
    subprocess.run(["cargo", "build", "--locked"], cwd=ROOT, check=True)
    cmd = [str(binary)]
    print(f"[backend] Starting: {' '.join(cmd)}")
    return subprocess.Popen(
        cmd,
        cwd=str(ROOT),
        stdout=sys.stdout,
        stderr=sys.stderr,
        **launcher.spawn_kwargs(),
    )


def start_frontend() -> subprocess.Popen | None:
    cmd = ["npm", "run", "dev"]
    node = shutil.which("node")
    if not node:
        print("[frontend] Node.js not found. Skipping frontend.")
        return None
    print(f"[frontend] Starting: npm run dev (in {FRONTEND_DIR})")
    return subprocess.Popen(
        cmd,
        cwd=str(FRONTEND_DIR),
        stdout=sys.stdout,
        stderr=sys.stderr,
        **launcher.spawn_kwargs(),
    )


def main():
    parser = argparse.ArgumentParser(description="Launch OmniBedrock MC servers (dev mode)")
    parser.add_argument("--backend", action="store_true", help="Run only the backend")
    parser.add_argument("--frontend", action="store_true", help="Run only the frontend")
    parser.add_argument(
        "--reset-admin",
        action="store_true",
        help="Clear the admin user store so a fresh admin is generated on next startup",
    )
    args = parser.parse_args()

    if args.reset_admin:
        parser.error("--reset-admin was removed: reset the selected account through the users API, or restore your users.yaml backup")

    run_backend = args.backend or not args.frontend
    run_frontend = args.frontend or not args.backend

    processes: list[subprocess.Popen] = []
    if run_backend:
        processes.append(start_backend())
    if run_frontend:
        ensure_vite_config()
        p = start_frontend()
        if p:
            processes.append(p)

    if not processes:
        print("No servers to run.")
        return

    launcher.run("OmniBedrock MC Panel (dev)", processes)


if __name__ == "__main__":
    main()
