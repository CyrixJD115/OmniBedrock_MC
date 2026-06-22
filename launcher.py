"""
Shared launcher logic for dev.py and start.py.
"""

import os
import select
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent

ARM_MIN_DELAY = 5
ARM_TIMEOUT = 10

_arm_state = {"armed": False, "time": 0.0}
_shutting_down = False


def spawn_kwargs() -> dict:
    if os.name == "nt":
        return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP, "stdin": subprocess.DEVNULL}
    return {"preexec_fn": os.setpgrp, "stdin": subprocess.DEVNULL}


def server_is_running() -> bool:
    lock_file = ROOT / "backend" / "data" / "console_lock_state.yaml"
    try:
        import yaml
        if lock_file.exists():
            data = yaml.safe_load(lock_file.read_text())
            return data.get("console", {}).get("state") == "locked"
    except Exception:
        pass
    return False


def _shutdown_all(processes: list[subprocess.Popen]) -> None:
    global _shutting_down
    if _shutting_down:
        return
    _shutting_down = True
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


def _sigint_handler(*_) -> None:
    if _shutting_down:
        return
    print("\n  Ctrl+C is disabled. Type 'quit' + Enter to shut down.")


def _handle_request(processes: list[subprocess.Popen]) -> None:
    if _shutting_down:
        return
    if server_is_running():
        now = time.time()
        arm = _arm_state

        if not arm["armed"]:
            arm["armed"] = True
            arm["time"] = now
            print()
            print("!" * 60)
            print("  WARNING: Minecraft server is still running!")
            print("  Shutdown ARMED. Type 'quit' again to confirm.")
            print(f"  Wait {ARM_MIN_DELAY}s\u2013{ARM_TIMEOUT}s between attempts. Sooner cancels.")
            print("!" * 60)
            return

        elapsed = now - arm["time"]
        if elapsed < ARM_MIN_DELAY:
            arm["armed"] = False
            print("Shutdown cancelled (too fast \u2014 protection triggered).")
            return
        if elapsed > ARM_TIMEOUT:
            arm["armed"] = False
            print("Shutdown cancelled (timed out).")
            return

        arm["armed"] = False

    _shutdown_all(processes)


def run(title: str, processes: list[subprocess.Popen]) -> None:
    signal.signal(signal.SIGINT, _sigint_handler)
    signal.signal(signal.SIGTERM, lambda *_: _shutdown_all(processes))

    print(f"\n{title} running. Type 'quit' + Enter to shut down.")

    stdin_ok = True
    while True:
        if stdin_ok:
            try:
                r, _, _ = select.select([sys.stdin], [], [], 0.5)
            except (ValueError, OSError):
                stdin_ok = False
                time.sleep(0.5)
                continue

            if r:
                line = sys.stdin.readline()
                if not line:
                    stdin_ok = False
                    continue
                if line.strip().lower() in ("quit", "q", "exit", "shutdown"):
                    _handle_request(processes)
        else:
            time.sleep(0.5)

        for p in processes:
            if p and p.poll() is not None:
                print(f"Process exited with code {p.returncode}")
                _shutdown_all(processes)
