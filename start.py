#!/usr/bin/env python3
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

ROOT = Path(__file__).resolve().parent
FRONTEND_DIR = ROOT / "frontend"
VITE_CONFIG = FRONTEND_DIR / "vite.config.ts"
VITE_CONFIG_EXAMPLE = FRONTEND_DIR / "vite.config.example.ts"


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


def ensure_vite_config():
    if not VITE_CONFIG_EXAMPLE.exists():
        return
    if not VITE_CONFIG.exists():
        shutil.copy2(VITE_CONFIG_EXAMPLE, VITE_CONFIG)
        print(f"[frontend] Created {VITE_CONFIG.name} from {VITE_CONFIG_EXAMPLE.name}")
        print(f"[frontend] Edit {VITE_CONFIG.name} to customize ports, hosts, etc.")


def ensure_dependencies():
    pkg = FRONTEND_DIR / "package.json"
    lock = FRONTEND_DIR / "node_modules" / ".package-lock.json"
    if not pkg.exists():
        return
    need_install = False
    if not (FRONTEND_DIR / "node_modules").exists():
        need_install = True
    elif lock.exists() and pkg.stat().st_mtime > lock.stat().st_mtime:
        need_install = True
    if need_install:
        print("[frontend] Installing dependencies (npm install)...")
        result = subprocess.run(["npm", "install"], cwd=str(FRONTEND_DIR))
        if result.returncode != 0:
            print("[frontend] npm install failed!")
            sys.exit(1)
        print("[frontend] Dependencies installed.")
    svelte_kit = FRONTEND_DIR / ".svelte-kit"
    if svelte_kit.exists():
        shutil.rmtree(svelte_kit)
        print("[frontend] Cleared stale .svelte-kit cache.")
    print("[frontend] Syncing SvelteKit...")
    subprocess.run(["npx", "svelte-kit", "sync"], cwd=str(FRONTEND_DIR), check=True)


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
    processes: list[subprocess.Popen] = []
    processes.append(start_backend())
    ensure_vite_config()
    ensure_dependencies()
    p = start_frontend()
    if p:
        processes.append(p)
    launcher.run("OmniBedrock MC Panel", processes)


if __name__ == "__main__":
    main()
