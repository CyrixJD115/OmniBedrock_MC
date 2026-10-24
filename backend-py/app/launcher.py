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

_shutting_down = False


def spawn_kwargs() -> dict:
    if os.name == "nt":
        return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP, "stdin": subprocess.DEVNULL}
    return {"preexec_fn": os.setpgrp, "stdin": subprocess.DEVNULL}


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
                print("\r\033[K", end="", flush=True)
                print(f"Process exited with code {p.returncode}")
                _shutdown_all(processes)
