#!/usr/bin/env python3
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FRONTEND_DIR = ROOT / "frontend"
VITE_CONFIG = FRONTEND_DIR / "vite.config.ts"
VITE_CONFIG_EXAMPLE = FRONTEND_DIR / "vite.config.example.ts"
LOCK_FILE = ROOT / "backend" / "data" / "console_lock_state.yaml"

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
    print(f"[backend] Starting: {' '.join(cmd)}")
    return subprocess.Popen(
        cmd,
        cwd=str(ROOT),
        stdout=sys.stdout,
        stderr=sys.stderr,
    )


def ensure_vite_config():
    if not VITE_CONFIG_EXAMPLE.exists():
        return
    if not VITE_CONFIG.exists():
        shutil.copy2(VITE_CONFIG_EXAMPLE, VITE_CONFIG)
        print(f"[frontend] Created {VITE_CONFIG.name} from {VITE_CONFIG_EXAMPLE.name}")
        print(f"[frontend] Edit {VITE_CONFIG.name} to customize ports, hosts, etc.")


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


_arm_state = {"armed": False, "time": 0.0}
_ARM_MIN_DELAY = 5
_ARM_TIMEOUT = 10
_shutting_down = False


def _shutdown_all() -> None:
    global _shutting_down
    if _shutting_down:
        return
    _shutting_down = True
    signal.signal(signal.SIGINT, signal.default_int_handler)
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


def cleanup(*_) -> None:
    if _shutting_down:
        return

    if _server_is_running():
        now = time.time()
        arm = _arm_state

        if not arm["armed"]:
            arm["armed"] = True
            arm["time"] = now
            print("\n" + "!" * 60)
            print("  WARNING: Minecraft server is still running!")
            print("  Shutdown is ARMED. Press Ctrl+C again to confirm.")
            print(f"  Wait at least {_ARM_MIN_DELAY}s between presses. Pressing sooner cancels.")
            print(f"  Arm expires in {_ARM_TIMEOUT}s.")
            print("!" * 60 + "\n")
            signal.signal(signal.SIGINT, signal.default_int_handler)
            return

        elapsed = now - arm["time"]
        if elapsed < _ARM_MIN_DELAY:
            arm["armed"] = False
            signal.signal(signal.SIGINT, cleanup)
            print("Shutdown cancelled (too fast — protection triggered).")
            return
        if elapsed > _ARM_TIMEOUT:
            arm["armed"] = False
            signal.signal(signal.SIGINT, cleanup)
            print("Shutdown cancelled (timed out).")
            return

        arm["armed"] = False

    _shutdown_all()


def main():
    signal.signal(signal.SIGINT, cleanup)
    signal.signal(signal.SIGTERM, cleanup)

    processes.append(start_backend())
    ensure_vite_config()
    p = start_frontend()
    if p:
        processes.append(p)

    try:
        while True:
            time.sleep(1)
            for p in processes:
                if p and p.poll() is not None:
                    print(f"Process exited with code {p.returncode}")
                    _shutdown_all()
    except KeyboardInterrupt:
        cleanup()


if __name__ == "__main__":
    main()
