#!/usr/bin/env python3
import argparse
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

FRONTEND_DIR = Path(__file__).resolve().parent / "frontend"
VITE_CONFIG = FRONTEND_DIR / "vite.config.ts"
VITE_CONFIG_EXAMPLE = FRONTEND_DIR / "vite.config.example.ts"
LOCK_FILE = Path(__file__).resolve().parent / "backend" / "data" / "console_lock_state.yaml"


def ensure_vite_config():
    if not VITE_CONFIG_EXAMPLE.exists():
        return
    if not VITE_CONFIG.exists():
        shutil.copy2(VITE_CONFIG_EXAMPLE, VITE_CONFIG)
        print(f"[frontend] Created {VITE_CONFIG.name} from {VITE_CONFIG_EXAMPLE.name}")
        print(f"[frontend] Edit {VITE_CONFIG.name} to customize ports, hosts, etc.")


ROOT = Path(__file__).resolve().parent

processes: list[subprocess.Popen] = []


def _server_is_running() -> bool:
    try:
        import yaml
        if LOCK_FILE.exists():
            data = yaml.safe_load(LOCK_FILE.read_text())
            return data.get("console", {}).get("state") == "locked"
    except Exception:
        pass
    return False


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
    )


def start_frontend() -> subprocess.Popen:
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
    )


_shutting_down = False


def cleanup(*_):
    global _shutting_down
    if _shutting_down:
        return
    _shutting_down = True

    if _server_is_running():
        print("\n" + "!" * 60)
        print("  WARNING: Minecraft server is still running!")
        print("  The panel will try to stop it gracefully.")
        print("  Press Ctrl+C again within 3 seconds to force quit.")
        print("!" * 60 + "\n")

        def force_quit(*_):
            print("\nForce quitting...")
            for p in processes:
                if p and p.poll() is None:
                    p.kill()
            sys.exit(1)

        old_handler = signal.signal(signal.SIGINT, force_quit)
        signal.signal(signal.SIGTERM, force_quit)
        try:
            time.sleep(3)
        except KeyboardInterrupt:
            pass
        signal.signal(signal.SIGINT, old_handler)

    print("\nShutting down...")
    for p in processes:
        if p and p.poll() is None:
            p.terminate()
    for p in processes:
        if p:
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
    sys.exit(0)


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

    signal.signal(signal.SIGINT, cleanup)
    signal.signal(signal.SIGTERM, cleanup)

    run_backend = args.backend or not args.frontend
    run_frontend = args.frontend or not args.backend

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

    try:
        while True:
            time.sleep(1)
            for p in processes:
                if p and p.poll() is not None:
                    print(f"Process exited with code {p.returncode}")
                    cleanup()
    except KeyboardInterrupt:
        cleanup()


if __name__ == "__main__":
    main()
