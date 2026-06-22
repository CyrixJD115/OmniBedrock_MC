#!/usr/bin/env python3
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

from backend.app import launcher

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
    venv_python = ROOT / ".venv" / "bin" / "python"
    if os.name == "nt":
        venv_python = ROOT / ".venv" / "Scripts" / "python.exe"
    if not venv_python.exists():
        venv_python = "python3"
    port = os.getenv("OMNI_PORT", "17754")
    cmd = [str(venv_python), "-m", "uvicorn", "backend.app.main:app", "--host", "0.0.0.0", "--port", port]
    cmd.append("--reload")
    cmd.append("--reload-exclude")
    cmd.append("bedrock_server/**")
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
        from backend.app.core.auth import reset_admin_store

        reset_admin_store()
        print("[reset] Admin user store cleared. A new admin + password will print on startup.")

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
