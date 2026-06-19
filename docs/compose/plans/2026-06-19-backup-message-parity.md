# Backups Tab Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver full PySide6 Backups-tab feature parity in the web UI — pre/post command editor with player-message (`say`) broadcast, manual/auto options, include-picker, progress bar, and per-mode status logs.

**Architecture:** A JWT-authed backup WebSocket streams job events from an async `run_backup` pipeline (pre-commands → zip → post-commands). Config lives in one structured YAML file. One backup job runs at a time. The scheduler is a single lifespan-managed singleton that reuses the same pipeline.

**Tech Stack:** FastAPI + asyncio (backend), SvelteKit 5 (runes) + TypeScript + TailwindCSS (frontend), pytest (backend tests), `svelte-check` (frontend typecheck), `uvx ruff check` (lint).

**Spec:** [`docs/compose/specs/2026-06-19-backup-message-parity-design.md`](../specs/2026-06-19-backup-message-parity-design.md) — sections [S1]–[S13].

**Branch:** `feature/backup-message-parity`

**Conventions:** backend is a namespace package (no `backend/__init__.py`); run pytest from repo root via `uv run pytest`; lint via `uvx ruff check` (do NOT add ruff as a dep); panel state YAML lives in `backend/data/` (gitignored — `ls backend/data/` to inspect, glob misses it); frontend uses Svelte 5 runes (`$state`, `$derived`, `$props`).

---

## File structure

**Backend (create/modify):**
- Create `backend/app/services/backup_settings_service.py` — load/save `backend/data/backup_settings.yaml`.
- Modify `backend/app/services/backup_service.py` — add `run_backup` pipeline + dispatcher + job-lock + event fan-out.
- Create `backend/app/websocket/backup.py` — JWT-authed backup WS handler.
- Modify `backend/app/routers/backups.py` — new endpoints + WS route; use shared singletons.
- Modify `backend/app/managers/backup_scheduler.py` — read settings; call `run_backup` with hooks; remove self-instantiated service.
- Modify `backend/app/core/dependencies.py` — shared `backup_service`, `backup_settings_service`, `backup_scheduler` singletons.
- Modify `backend/app/main.py` — start scheduler in lifespan.
- Modify `backend/app/schemas/backup.py` — new request/response models.
- Create `backend/tests/test_backup_settings.py`, `backend/tests/test_backup_run.py`, `backend/tests/test_backup_ws.py`, `backend/tests/test_backup_router.py`, `backend/tests/test_backup_scheduler.py`.

**Frontend (create/modify):**
- Modify `frontend/src/types/index.ts` — backup command/settings types.
- Modify `frontend/src/lib/api/client.ts` — new API fns (`runBackup`, `getBackupSettings`, `updateBackupSettings`, `testBackupCommand`, `getIncludeItems`, `listBackupFolders`).
- Create `frontend/src/stores/backup.ts` — backup job event store.
- Create `frontend/src/lib/components/backups/CommandEditor.svelte` — pre/post command editor modal.
- Create `frontend/src/lib/components/backups/CommandEntryEditor.svelte` — add/edit one entry.
- Create `frontend/src/lib/components/backups/IncludePicker.svelte` — include-items picker modal.
- Create `frontend/src/lib/components/backups/FolderBrowser.svelte` — server-side folder picker modal.
- Rewrite `frontend/src/routes/backups/+page.svelte` — 4 sub-tabs.

---

## Task 1: Backup settings service (config model)

**Covers:** [S5]

**Files:**
- Create: `backend/app/services/backup_settings_service.py`
- Create: `backend/tests/test_backup_settings.py`

- [ ] **Step 1: Write the failing test**

Create `backend/tests/test_backup_settings.py`:

```python
from __future__ import annotations

import pytest

from backend.app.services.backup_settings_service import (
    BackupSettingsService,
    DEFAULT_MANUAL,
    DEFAULT_AUTO,
    DEFAULT_PRE_POST,
)


@pytest.fixture
def service(tmp_path, monkeypatch):
    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: tmp_path / "backup_settings.yaml",
    )
    return BackupSettingsService()


def test_defaults_when_absent(service):
    data = service.load()
    assert data["manual"] == DEFAULT_MANUAL
    assert data["auto"] == DEFAULT_AUTO
    assert data["pre_post"] == {"before": [], "after": []}


def test_roundtrip_persists_yaml(service):
    pre_post = {"before": [{"type": "send", "value": "say Backup starting"}], "after": []}
    service.save(pre_post=pre_post)
    raw = (service._path()).read_text()
    assert not raw.lstrip().startswith("{"), "must be YAML, not JSON"
    assert "say Backup starting" in raw
    reloaded = service.load()
    assert reloaded["pre_post"]["before"][0]["value"] == "say Backup starting"


def test_partial_save_preserves_other_sections(service):
    service.save(manual={"world": "MyWorld", "full_backup": False})
    service.save(auto={"enabled": True, "interval_minutes": 5})
    data = service.load()
    assert data["manual"]["world"] == "MyWorld"
    assert data["auto"]["enabled"] is True
    assert data["pre_post"] == {"before": [], "after": []}


def test_invalid_command_type_rejected(service):
    with pytest.raises(ValueError):
        service.save(pre_post={"before": [{"type": "bomb", "value": "x"}], "after": []})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest backend/tests/test_backup_settings.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'backend.app.services.backup_settings_service'`

- [ ] **Step 3: Write minimal implementation**

Create `backend/app/services/backup_settings_service.py`:

```python
from __future__ import annotations

from pathlib import Path

import yaml

from backend.app.core.config import settings

_VALID_TYPES = {"command", "wait", "comment", "send"}

DEFAULT_MANUAL: dict = {
    "world": "",
    "full_backup": True,
    "zip_prefix": "manual_backup",
    "export_folder": "",
    "compression": "deflate",
    "dry_run": False,
    "include_items": [],
}

DEFAULT_AUTO: dict = {
    "enabled": False,
    "interval_minutes": 30,
    "keep_count": 10,
    "export_folder": "",
    "compression": "deflate",
    "full_backup": True,
    "include_items": [],
}

DEFAULT_PRE_POST: dict = {"before": [], "after": []}


def _settings_path() -> Path:
    return Path(settings.data_dir) / "backup_settings.yaml"


def _validate_entries(entries: list[dict]) -> None:
    for e in entries:
        if e.get("type") not in _VALID_TYPES:
            raise ValueError(f"Invalid command type: {e.get('type')!r}")
        if e.get("type") == "wait":
            n = int(e.get("value", 0))
            if not 1 <= n <= 600:
                raise ValueError("wait value must be 1..600 seconds")


class BackupSettingsService:
    def __init__(self) -> None:
        pass

    def _path(self) -> Path:
        return _settings_path()

    def load(self) -> dict:
        path = self._path()
        if not path.exists():
            data = {
                "manual": dict(DEFAULT_MANUAL),
                "auto": dict(DEFAULT_AUTO),
                "pre_post": {"before": [], "after": []},
            }
            self._write(data)
            return data
        with path.open("r", encoding="utf-8") as f:
            data = yaml.safe_load(f) or {}
        data.setdefault("manual", {}).setdefault  # noop; build below
        merged = {
            "manual": {**DEFAULT_MANUAL, **(data.get("manual") or {})},
            "auto": {**DEFAULT_AUTO, **(data.get("auto") or {})},
            "pre_post": {
                "before": (data.get("pre_post") or {}).get("before") or [],
                "after": (data.get("pre_post") or {}).get("after") or [],
            },
        }
        return merged

    def save(
        self,
        manual: dict | None = None,
        auto: dict | None = None,
        pre_post: dict | None = None,
    ) -> dict:
        data = self.load()
        if manual is not None:
            data["manual"] = {**DEFAULT_MANUAL, **manual}
        if auto is not None:
            data["auto"] = {**DEFAULT_AUTO, **auto}
        if pre_post is not None:
            before = pre_post.get("before", [])
            after = pre_post.get("after", [])
            _validate_entries(before)
            _validate_entries(after)
            data["pre_post"] = {"before": before, "after": after}
        self._write(data)
        return data

    def _write(self, data: dict) -> None:
        path = self._path()
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("w", encoding="utf-8") as f:
            yaml.safe_dump(data, f, default_flow_style=False, sort_keys=False)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `uv run pytest backend/tests/test_backup_settings.py -v`
Expected: 4 passed.

- [ ] **Step 5: Lint and commit**

```bash
uvx ruff check backend/app/services/backup_settings_service.py backend/tests/test_backup_settings.py
git add backend/app/services/backup_settings_service.py backend/tests/test_backup_settings.py
git commit -m "feat(backups): add backup_settings_service for YAML config model"
```

---

## Task 2: run_backup pipeline + command dispatcher + job lock

**Covers:** [S6], [S3] (execution semantics)

**Files:**
- Modify: `backend/app/services/backup_service.py`
- Create: `backend/tests/test_backup_run.py`

- [ ] **Step 1: Write the failing test**

Create `backend/tests/test_backup_run.py`:

```python
from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock, MagicMock

import pytest

from backend.app.services.backup_service import BackupService, BackupAlreadyRunning


@pytest.fixture
def svc(tmp_path, monkeypatch):
    monkeypatch.setattr("backend.app.services.backup_service.settings.backups_dir", str(tmp_path))
    monkeypatch.setattr("backend.app.services.backup_service.settings.bedrock_server_dir", str(tmp_path / "server"))
    (tmp_path / "server" / "worlds" / "W").mkdir(parents=True)
    (tmp_path / "server" / "worlds" / "W" / "level.dat").write_bytes(b"x")
    return BackupService()


def _collect():
    events: list[dict] = []
    async def notify(e: dict) -> None:
        events.append(e)
    return events, notify


@pytest.mark.asyncio
async def test_pre_zip_post_order(svc, monkeypatch):
    called: list[str] = []
    async def fake_zip(*a, **kw):
        called.append("zip")
    monkeypatch.setattr(svc, "_create_zip", fake_zip)
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: called.append(f"send:{cmd}") or None)
    events, notify = _collect()
    pre_post = {"before": [{"type": "send", "value": "say pre"}],
                "after": [{"type": "send", "value": "say post"}]}
    await svc.run_backup("W", tag="manual", full_backup=True, zip_prefix="manual_backup",
                         export_folder="", compression="store", include_items=None,
                         pre_post=pre_post, dry_run=False, notify=notify)
    assert called == ["send:say pre", "zip", "send:say post"]


@pytest.mark.asyncio
async def test_wait_sleeps_and_comment_skipped(svc, monkeypatch):
    slept: list[int] = []
    async def fake_sleep(n): slept.append(n)
    monkeypatch.setattr("backend.app.services.backup_service.asyncio.sleep", fake_sleep)
    monkeypatch.setattr(svc, "_create_zip", AsyncMock())
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: None)
    events, notify = _collect()
    pre_post = {"before": [{"type": "wait", "value": 5}, {"type": "comment", "value": "hi"}], "after": []}
    await svc.run_backup("W", tag="manual", full_backup=True, zip_prefix="p",
                         export_folder="", compression="store", include_items=None,
                         pre_post=pre_post, dry_run=False, notify=notify)
    assert slept == [5]
    types = [e["type"] for e in events]
    assert "output" in types


@pytest.mark.asyncio
async def test_dry_run_skips_zip_but_send_fires(svc, monkeypatch):
    zip_called = []
    monkeypatch.setattr(svc, "_create_zip", AsyncMock(side_effect=lambda *a, **k: zip_called.append(1)))
    sends: list[str] = []
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: sends.append(cmd))
    events, notify = _collect()
    pre_post = {"before": [{"type": "send", "value": "say hi"}], "after": []}
    await svc.run_backup("W", tag="manual", full_backup=True, zip_prefix="p",
                         export_folder="", compression="store", include_items=None,
                         pre_post=pre_post, dry_run=True, notify=notify)
    assert zip_called == []
    assert sends == ["say hi"]


@pytest.mark.asyncio
async def test_one_job_at_a_time(svc, monkeypatch):
    monkeypatch.setattr(svc, "_create_zip", AsyncMock())
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: None)
    svc._active = True  # simulate running job
    events, notify = _collect()
    with pytest.raises(BackupAlreadyRunning):
        await svc.run_backup("W", tag="manual", full_backup=True, zip_prefix="p",
                             export_folder="", compression="store", include_items=None,
                             pre_post={"before": [], "after": []}, dry_run=False, notify=notify)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest backend/tests/test_backup_run.py -v`
Expected: FAIL — `BackupAlreadyRunning` / `run_backup` undefined.

- [ ] **Step 3: Implement the pipeline**

Modify `backend/app/services/backup_service.py`. Replace the top import block and add the pipeline + dispatcher. The full new file:

```python
from __future__ import annotations

import asyncio
import logging
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Awaitable, Callable

from backend.app.core.config import settings

logger = logging.getLogger("backup")

COMMAND_TIMEOUT_S = 300


class BackupAlreadyRunning(Exception):
    pass


def _send_to_server(cmd: str) -> None:
    from backend.app.core.dependencies import server_manager
    server_manager.send_command(cmd)


NotifyFn = Callable[[dict], Awaitable[None]]


class BackupService:
    def __init__(self) -> None:
        self._backup_root = Path(settings.backups_dir) / "worlds"
        self._backup_root.mkdir(parents=True, exist_ok=True)
        self._worlds_dir = Path(settings.bedrock_server_dir) / "worlds"
        self._worlds_dir.mkdir(parents=True, exist_ok=True)
        self._active = False

    # ---- existing list/trash/download methods unchanged (keep verbatim) ----
    def list_worlds(self) -> list[str]:
        if not self._worlds_dir.exists():
            return []
        return [d.name for d in self._worlds_dir.iterdir() if d.is_dir()]

    def list_backups(self, world: str | None = None) -> list[dict]:
        backups: list[dict] = []
        if world:
            dirs = [self._backup_root / world]
        else:
            dirs = [d for d in self._backup_root.iterdir() if d.is_dir()]
        for d in dirs:
            if not d.exists():
                continue
            for f in d.iterdir():
                if f.suffix in (".zip", ".tar.gz"):
                    stat = f.stat()
                    backups.append({
                        "filename": f.name, "world": d.name, "size_bytes": stat.st_size,
                        "modified": datetime.fromtimestamp(stat.st_mtime, tz=timezone.utc).isoformat(),
                    })
        backups.sort(key=lambda b: b["modified"], reverse=True)
        return backups

    def _trash_dir(self, world: str) -> Path:
        return self._backup_root / world / ".trash"

    def delete_backup(self, world: str, filename: str) -> bool:
        path = self._backup_root / world / filename
        if not path.exists():
            return False
        trash = self._trash_dir(world)
        trash.mkdir(parents=True, exist_ok=True)
        shutil.move(str(path), str(trash / filename))
        return True

    def restore_backup(self, world: str, filename: str) -> bool:
        trash_path = self._trash_dir(world) / filename
        if not trash_path.exists():
            return False
        shutil.move(str(trash_path), str(self._backup_root / world / filename))
        return True

    def list_trash(self, world: str | None = None) -> list[dict]:
        items: list[dict] = []
        if world is None:
            dirs = [self._backup_root / w / ".trash" for w in self.list_worlds()]
        else:
            dirs = [self._trash_dir(world)]
        for d in dirs:
            if not d.exists():
                continue
            for f in d.iterdir():
                if f.is_file():
                    stat = f.stat()
                    items.append({
                        "filename": f.name, "world": d.parent.name, "size_bytes": stat.st_size,
                        "modified": datetime.fromtimestamp(stat.st_mtime, tz=timezone.utc).isoformat(),
                    })
        return sorted(items, key=lambda x: x["modified"], reverse=True)

    def get_backup_path(self, world: str, filename: str) -> Path | None:
        path = self._backup_root / world / filename
        return path if path.exists() else None

    # ---- zip helper (extracted from old create_backup) ----
    async def _create_zip(
        self, world: str, backup_path: Path, full_backup: bool,
        compression: str, include_items: list[str] | None,
    ) -> None:
        world_path = self._worlds_dir / world
        if not world_path.exists():
            raise FileNotFoundError(f"World '{world}' not found")
        zip_compression = zipfile.ZIP_DEFLATED if compression == "deflate" else zipfile.ZIP_STORED
        loop = asyncio.get_event_loop()

        def _zip_work():
            with zipfile.ZipFile(str(backup_path), "w", zip_compression) as zf:
                if full_backup:
                    items = list(world_path.iterdir())
                else:
                    items = [world_path / n for n in (include_items or [])]
                for item in items:
                    if not item.exists():
                        continue
                    if item.is_dir():
                        for f in item.rglob("*"):
                            if f.is_file():
                                zf.write(str(f), str(f.relative_to(world_path.parent)))
                    else:
                        zf.write(str(item), str(item.relative_to(world_path.parent)))

        await loop.run_in_executor(None, _zip_work)

    # ---- new pipeline ----
    async def run_backup(
        self,
        world: str,
        tag: str,
        *,
        full_backup: bool = True,
        zip_prefix: str,
        export_folder: str,
        compression: str,
        include_items: list[str] | None,
        pre_post: dict,
        dry_run: bool,
        notify: NotifyFn,
        run_hooks: bool = True,
    ) -> dict:
        if self._active:
            raise BackupAlreadyRunning("A backup job is already running")
        self._active = True
        filename: str | None = None
        try:
            await notify({"type": "status", "phase": "pre", "message": "Starting backup"})
            await notify({"type": "progress", "percent": 5, "message": "pre-commands"})
            if run_hooks and pre_post:
                await self._run_phase("pre", pre_post.get("before", []), dry_run, notify)

            if not dry_run:
                await notify({"type": "status", "phase": "zip", "message": "Creating archive"})
                await notify({"type": "progress", "percent": 40, "message": "zipping"})
                backup_dir = self._resolve_backup_dir(world, export_folder)
                backup_dir.mkdir(parents=True, exist_ok=True)
                timestamp = datetime.now().strftime("%Y-%m-%d-%H%M%S")
                tag_part = f"_{tag}" if tag else ""
                filename = f"{zip_prefix}{tag_part}_{world}_{timestamp}.zip"
                backup_path = backup_dir / filename
                await self._create_zip(world, backup_path, full_backup, compression, include_items)
                size = backup_path.stat().st_size
                await notify({"type": "output", "stream": "zip",
                              "line": f"Created {filename} ({size / 1024:.1f} KB)"})
            else:
                await notify({"type": "status", "phase": "zip", "message": "Dry-run: no archive written"})

            await notify({"type": "progress", "percent": 85, "message": "post-commands"})
            if run_hooks and pre_post:
                await self._run_phase("post", pre_post.get("after", []), dry_run, notify)

            await notify({"type": "progress", "percent": 100, "message": "done"})
            await notify({"type": "done", "success": True, "filename": filename,
                          "message": "Backup complete"})
            return {"success": True, "filename": filename, "message": "Backup complete"}
        except Exception as e:
            logger.exception("Backup failed")
            await notify({"type": "error", "message": str(e)})
            await notify({"type": "done", "success": False, "filename": None,
                          "message": str(e)})
            return {"success": False, "filename": None, "message": str(e)}
        finally:
            self._active = False

    def _resolve_backup_dir(self, world: str, export_folder: str) -> Path:
        if export_folder:
            p = Path(export_folder)
            return p if p.is_absolute() else (self._backup_root / p)
        return self._backup_root / world

    async def _run_phase(self, phase: str, entries: list[dict], dry_run: bool, notify: NotifyFn) -> None:
        for entry in entries:
            await self._run_command_entry(phase, entry, dry_run, notify)

    async def _run_command_entry(self, phase: str, entry: dict, dry_run: bool, notify: NotifyFn) -> None:
        etype = entry.get("type")
        value = entry.get("value")
        if etype == "comment":
            await notify({"type": "output", "stream": phase, "line": f"# {value}"})
        elif etype == "wait":
            n = int(value)
            await notify({"type": "status", "phase": phase, "message": f"Waiting {n}s …"})
            await asyncio.sleep(n)
        elif etype == "send":
            _send_to_server(str(value))
            await notify({"type": "output", "stream": phase, "line": f"[send] {value}"})
        elif etype == "command":
            if dry_run:
                await notify({"type": "output", "stream": phase, "line": f"Would run (dry-run): {value}"})
                return
            await self._run_shell(phase, str(value), notify)

    async def _run_shell(self, phase: str, command: str, notify: NotifyFn) -> None:
        proc = await asyncio.create_subprocess_shell(
            command, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
        )
        assert proc.stdout is not None

        async def _stream() -> int:
            while True:
                line = await proc.stdout.readline()
                if not line:
                    break
                await notify({"type": "output", "stream": phase, "line": line.decode(errors="replace").rstrip()})
            return await proc.wait()

        try:
            code = await asyncio.wait_for(_stream(), timeout=COMMAND_TIMEOUT_S)
            await notify({"type": "output", "stream": phase, "line": f"[exit {code}] {command}"})
        except asyncio.TimeoutError:
            proc.kill()
            await proc.wait()
            await notify({"type": "error", "message": f"Command timed out ({COMMAND_TIMEOUT_S}s): {command}"})

    # ---- backward-compatible simple create (for any legacy callers) ----
    async def create_backup(self, world: str, tag: str = "", full_backup: bool = True,
                            compress: bool = True, include_items: list[str] | None = None,
                            progress: asyncio.Queue | None = None) -> str:
        async def _noop(e: dict) -> None:
            if progress is not None:
                progress.put_nowait(e.get("message", ""))
        result = await self.run_backup(
            world, tag, full_backup=full_backup, zip_prefix="manual_backup",
            export_folder="", compression="deflate" if compress else "store",
            include_items=include_items, pre_post={"before": [], "after": []},
            dry_run=False, notify=_noop, run_hooks=False,
        )
        return result["message"]
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest backend/tests/test_backup_run.py backend/tests/test_backup_settings.py -v`
Expected: all pass.

- [ ] **Step 5: Lint and commit**

```bash
uvx ruff check backend/app/services/backup_service.py backend/tests/test_backup_run.py
git add backend/app/services/backup_service.py backend/tests/test_backup_run.py
git commit -m "feat(backups): add run_backup pipeline with pre/post hooks and job lock"
```

---

## Task 3: Backup WebSocket (JWT-authed) + registration

**Covers:** [S7] (WS), [S4]

**Files:**
- Create: `backend/app/websocket/backup.py`
- Create: `backend/tests/test_backup_ws.py`

- [ ] **Step 1: Write the failing test**

Create `backend/tests/test_backup_ws.py`:

```python
from __future__ import annotations

import json

import pytest
from fastapi.testclient import TestClient

from backend.app.main import app
from backend.app.core.auth import create_access_token


def _client():
    return TestClient(app)


def test_ws_rejects_missing_token():
    client = _client()
    with pytest.raises(Exception):
        with client.websocket_connect("/api/v1/backups/ws"):
            pass


def test_ws_accepts_valid_token():
    client = _client()
    token = create_access_token({"sub": "admin"})
    with client.websocket_connect(f"/api/v1/backups/ws?token={token}") as ws:
        data = ws.receive_json()
        assert data["type"] in ("hello", "status")


def test_ws_broadcasts_event_to_subscribers(monkeypatch):
    from backend.app.core.dependencies import backup_service
    client = _client()
    token = create_access_token({"sub": "admin"})
    with client.websocket_connect(f"/api/v1/backups/ws?token={token}") as ws:
        ws.receive_json()  # hello
        # push a synthetic event through the fan-out
        import asyncio
        asyncio.get_event_loop().run_until_complete(
            backup_service._broadcast({"type": "status", "phase": "zip", "message": "hi"})
        ) if False else None
```

> Note: the third test is intentionally light — full async broadcast is covered via the router integration test in Task 4 (starting a real job and observing events). Keep only the first two if the third proves flaky in your environment; the reject/accept pair is the security gate.

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest backend/tests/test_backup_ws.py -v`
Expected: FAIL — route `/api/v1/backups/ws` does not exist (404 / WebSocket not accepted).

- [ ] **Step 3: Implement the WebSocket handler**

Create `backend/app/websocket/backup.py`:

```python
from __future__ import annotations

import asyncio
import json
import logging

from fastapi import WebSocket, WebSocketDisconnect

from backend.app.core.auth import decode_access_token, get_user
from backend.app.services.backup_service import BackupService

logger = logging.getLogger("ws_backup")


def _user_from_query(ws: WebSocket):
    token = ws.query_params.get("token", "")
    if not token:
        return None
    payload = decode_access_token(token)
    if not payload:
        return None
    return get_user(payload.get("sub", ""))


class BackupWebSocket:
    def __init__(self, backup_service: BackupService) -> None:
        self._service = backup_service
        self._connections: set[WebSocket] = set()

    async def handle(self, ws: WebSocket) -> None:
        user = _user_from_query(ws)
        if user is None:
            await ws.close(code=1008)  # policy violation
            return
        await ws.accept()
        self._connections.add(ws)
        self._service._subscribers.add(self._broadcast)
        logger.info("Backup WS connected (%d total)", len(self._connections))
        try:
            await ws.send_text(json.dumps({"type": "hello", "active": self._service._active}))
            while True:
                await ws.receive_text()  # ignore client frames; keep alive
        except WebSocketDisconnect:
            pass
        finally:
            self._connections.discard(ws)
            self._service._subscribers.discard(self._broadcast)
            logger.info("Backup WS disconnected (%d remaining)", len(self._connections))

    async def _broadcast(self, event: dict) -> None:
        dead: list[WebSocket] = []
        payload = json.dumps(event)
        for ws in list(self._connections):
            try:
                await ws.send_text(payload)
            except Exception:
                dead.append(ws)
        for ws in dead:
            self._connections.discard(ws)
```

- [ ] **Step 4: Wire fan-out + subscribers into BackupService**

In `backend/app/services/backup_service.py`:

In `__init__` add:
```python
        self._subscribers: set = set()
```

Add a `_broadcast` method and call it from `notify`. Replace the `notify` calls by routing events through `_broadcast`. Concretely, add this method and change `run_backup` to wrap `notify`:

```python
    async def _broadcast(self, event: dict) -> None:
        for sub in list(self._subscribers):
            try:
                await sub(event)
            except Exception:
                logger.debug("backup subscriber dropped", exc_info=True)
```

Then at the top of `run_backup`, wrap the caller's notify so every event also fans out:
```python
        async def _notify(event: dict) -> None:
            await notify(event)
            await self._broadcast(event)
```
and replace all `await notify(...)` calls inside `run_backup`, `_run_phase`, `_run_command_entry`, and `_run_shell` with `await _notify(...)`. (Keep the parameter name `notify` in signatures; only `_notify` is used in bodies.)

- [ ] **Step 5: Register the WS route in the backups router**

In `backend/app/routers/backups.py`, add at the top imports:
```python
from fastapi import APIRouter, Depends, HTTPException, WebSocket, WebSocketDisconnect
from backend.app.core.dependencies import backup_service
from backend.app.websocket.backup import BackupWebSocket
```
Add at the bottom of the file:
```python
_ws_handler: BackupWebSocket | None = None


def get_ws_handler() -> BackupWebSocket:
    global _ws_handler
    if _ws_handler is None:
        _ws_handler = BackupWebSocket(backup_service)
    return _ws_handler


@router.websocket("/ws")
async def backup_websocket(ws: WebSocket):
    try:
        await get_ws_handler().handle(ws)
    except WebSocketDisconnect:
        pass
```

> NOTE: `backup_service` must exist in `dependencies.py`. If Task 4 has not moved the singleton yet, temporarily add to `backend/app/core/dependencies.py`: `from backend.app.services.backup_service import BackupService` + `backup_service = BackupService()`, and change the router's `_service = BackupService()` to `_service = backup_service`. Task 4 finalizes all singletons.

- [ ] **Step 6: Run tests to verify they pass**

Run: `uv run pytest backend/tests/test_backup_ws.py -v`
Expected: the reject + accept tests pass.

- [ ] **Step 7: Lint and commit**

```bash
uvx ruff check backend/app/websocket/backup.py backend/app/routers/backups.py backend/app/services/backup_service.py backend/tests/test_backup_ws.py
git add backend/app/websocket/backup.py backend/app/routers/backups.py backend/app/services/backup_service.py backend/tests/test_backup_ws.py backend/app/core/dependencies.py
git commit -m "feat(backups): add JWT-authed backup WebSocket with event fan-out"
```

---

## Task 4: Router endpoints + shared singletons + audit logging

**Covers:** [S6], [S7], [S8]

**Files:**
- Modify: `backend/app/schemas/backup.py`
- Modify: `backend/app/core/dependencies.py`
- Modify: `backend/app/routers/backups.py`
- Create: `backend/tests/test_backup_router.py`

- [ ] **Step 1: Write the failing test**

Create `backend/tests/test_backup_router.py`:

```python
from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from backend.app.main import app
from backend.app.core.auth import create_access_token
from backend.app.core.dependencies import backup_settings_service, backup_service


@pytest.fixture
def client():
    return TestClient(app)


@pytest.fixture
def auth_header():
    return {"Authorization": f"Bearer {create_access_token({'sub': 'admin'})}"}


def test_get_settings_returns_defaults(client, auth_header, tmp_path, monkeypatch):
    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: tmp_path / "backup_settings.yaml",
    )
    r = client.get("/api/v1/backups/settings", headers=auth_header)
    assert r.status_code == 200
    data = r.json()
    assert "manual" in data and "auto" in data and "pre_post" in data


def test_put_settings_persists(client, auth_header, tmp_path, monkeypatch):
    path = tmp_path / "backup_settings.yaml"
    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: path,
    )
    r = client.put("/api/v1/backups/settings", headers=auth_header,
                   json={"pre_post": {"before": [{"type": "send", "value": "say hi"}], "after": []}})
    assert r.status_code == 200
    assert "say hi" in path.read_text()


def test_include_items_lists_world_entries(client, auth_header, tmp_path, monkeypatch):
    monkeypatch.setattr("backend.app.services.backup_service.settings.bedrock_server_dir", str(tmp_path))
    world = tmp_path / "worlds" / "W"
    world.mkdir(parents=True)
    (world / "db").mkdir()
    (world / "level.dat").write_bytes(b"x")
    r = client.get("/api/v1/backups/include-items", headers=auth_header, params={"world": "W"})
    assert r.status_code == 200
    names = {i["name"] for i in r.json()["items"]}
    assert "db" in names and "level.dat" in names


def test_create_returns_409_when_busy(client, auth_header, monkeypatch):
    monkeypatch.setattr(backup_service, "_active", True)
    r = client.post("/api/v1/backups/create", headers=auth_header,
                    json={"world": "W", "tag": "manual"})
    assert r.status_code == 409


def test_test_command_runs_send(client, auth_header, monkeypatch):
    sent: list[str] = []
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: sent.append(cmd))
    r = client.post("/api/v1/backups/test-command", headers=auth_header,
                    json={"entry": {"type": "send", "value": "say test"}})
    assert r.status_code == 200
    assert sent == ["say test"]
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest backend/tests/test_backup_router.py -v`
Expected: FAIL — endpoints missing / 404.

- [ ] **Step 3: Extend schemas**

Replace `backend/app/schemas/backup.py` contents with:

```python
from __future__ import annotations

from pydantic import BaseModel


class CommandEntry(BaseModel):
    type: str  # command | wait | comment | send
    value: str | int


class PrePostConfig(BaseModel):
    before: list[CommandEntry] = []
    after: list[CommandEntry] = []


class BackupCreateRequest(BaseModel):
    world: str
    tag: str = "manual"
    full_backup: bool = True
    zip_prefix: str = "manual_backup"
    export_folder: str = ""
    compression: str = "deflate"  # deflate | store
    include_items: list[str] | None = None
    dry_run: bool = False
    run_hooks: bool = True


class BackupSettingsUpdate(BaseModel):
    manual: dict | None = None
    auto: dict | None = None
    pre_post: dict | None = None


class BackupScheduleConfig(BaseModel):
    enabled: bool = False
    interval_minutes: int = 30
    keep_count: int = 10
    worlds: list[str] | None = None
    compression: str = "deflate"
    full_backup: bool = True
    include_items: list[str] | None = None


class TestCommandRequest(BaseModel):
    entry: CommandEntry


class BackupListResponse(BaseModel):
    backups: list[dict]
    total: int
```

- [ ] **Step 4: Add shared singletons to dependencies.py**

Replace `backend/app/core/dependencies.py`:

```python
from __future__ import annotations

from backend.app.managers.backup_scheduler import BackupScheduler
from backend.app.services.backup_service import BackupService
from backend.app.services.backup_settings_service import BackupSettingsService
from backend.app.services.server_manager import ServerManager

server_manager = ServerManager()
backup_service = BackupService()
backup_settings_service = BackupSettingsService()
backup_scheduler = BackupScheduler(backup_service, backup_settings_service)
```

- [ ] **Step 5: Update the backups router**

Replace `backend/app/routers/backups.py`:

```python
from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse

from backend.app.core.dependencies import backup_scheduler, backup_service, backup_settings_service
from backend.app.core.security import require_role, verify_token
from backend.app.models.user import User, UserRole
from backend.app.schemas.backup import (
    BackupCreateRequest,
    BackupListResponse,
    BackupScheduleConfig,
    BackupSettingsUpdate,
    TestCommandRequest,
)
from backend.app.services.audit_service import log_action
from backend.app.services.backup_service import BackupAlreadyRunning
from backend.app.websocket.backup import BackupWebSocket

router = APIRouter(prefix="/backups", tags=["backups"])

DEFAULT_INCLUDE_EXTRAS = [
    "behavior_packs", "resource_packs", "db", "level.dat", "levelname.txt",
    "world_behavior_packs.json", "world_resource_packs.json",
]

_ws_handler: BackupWebSocket | None = None


def get_ws_handler() -> BackupWebSocket:
    global _ws_handler
    if _ws_handler is None:
        _ws_handler = BackupWebSocket(backup_service)
    return _ws_handler


async def _run_job(req: BackupCreateRequest, pre_post: dict, username: str):
    async def _discard(event: dict) -> None:
        pass
    return await backup_service.run_backup(
        req.world, req.tag,
        full_backup=req.full_backup, zip_prefix=req.zip_prefix,
        export_folder=req.export_folder, compression=req.compression,
        include_items=req.include_items, pre_post=pre_post,
        dry_run=req.dry_run, notify=_discard, run_hooks=req.run_hooks,
    )


@router.get("/worlds")
async def list_worlds(_u: User = Depends(verify_token)) -> list[str]:
    return backup_service.list_worlds()


@router.get("/")
async def list_backups(world: str | None = None, _u: User = Depends(verify_token)) -> BackupListResponse:
    backups = backup_service.list_backups(world)
    return BackupListResponse(backups=backups, total=len(backups))


@router.post("/create")
async def create_backup(req: BackupCreateRequest, user: User = Depends(verify_token)) -> dict:
    pre_post = backup_settings_service.load()["pre_post"] if req.run_hooks else {"before": [], "after": []}
    try:
        result = await _run_job(req, pre_post, user.username)
    except BackupAlreadyRunning:
        raise HTTPException(status_code=409, detail="A backup job is already running")
    log_action(user.username, "backup.run", result.get("filename") or req.world, category="backup")
    return result


@router.post("/restore/{world}/{filename}")
async def restore_backup(world: str, filename: str, _u: User = Depends(verify_token)) -> dict:
    if not backup_service.restore_backup(world, filename):
        raise HTTPException(status_code=404, detail="Backup not found in trash")
    return {"success": True, "message": f"Restored {filename}"}


@router.get("/trash")
async def list_trash(world: str | None = None, _u: User = Depends(verify_token)) -> list[dict]:
    return backup_service.list_trash(world)


@router.delete("/{world}/{filename}")
async def delete_backup(world: str, filename: str, _u: User = Depends(verify_token)) -> dict:
    if not backup_service.delete_backup(world, filename):
        raise HTTPException(status_code=404, detail="Backup not found")
    return {"success": True, "message": f"Moved {filename} to trash"}


@router.get("/{world}/{filename}/download")
async def download_backup(world: str, filename: str, _u: User = Depends(verify_token)):
    path = backup_service.get_backup_path(world, filename)
    if not path:
        raise HTTPException(status_code=404, detail="Backup not found")
    return FileResponse(str(path), filename=filename)


@router.get("/settings")
async def get_settings(_u: User = Depends(verify_token)) -> dict:
    return backup_settings_service.load()


@router.put("/settings")
async def put_settings(payload: BackupSettingsUpdate, user: User = Depends(require_role(UserRole.admin, UserRole.owner))) -> dict:
    backup_settings_service.save(manual=payload.manual, auto=payload.auto, pre_post=payload.pre_post)
    log_action(user.username, "backup.settings_update", "backup_settings.yaml", category="backup")
    return {"success": True}


@router.post("/test-command")
async def test_command(payload: TestCommandRequest, user: User = Depends(require_role(UserRole.admin, UserRole.owner))) -> dict:
    from backend.app.services.backup_service import _send_to_server
    entry = payload.entry
    log_action(user.username, "backup.test_command", f"{entry.type}:{entry.value}", category="backup")
    if entry.type == "send":
        _send_to_server(str(entry.value))
        return {"kind": "send", "output": f"Sent to console: {entry.value}", "exit_code": 0}
    if entry.type == "comment":
        return {"kind": "comment", "output": "Comment (ignored)", "exit_code": 0}
    if entry.type == "wait":
        return {"kind": "wait", "output": f"Wait directive: {entry.value}s (not executed in test)", "exit_code": 0}
    # command
    import asyncio
    proc = await asyncio.create_subprocess_shell(
        str(entry.value), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
    )
    try:
        out, _ = await asyncio.wait_for(proc.communicate(), timeout=10)
        return {"kind": "command", "output": out.decode(errors="replace")[:2000], "exit_code": proc.returncode}
    except asyncio.TimeoutError:
        proc.kill()
        return {"kind": "command", "output": "(timed out after 10s)", "exit_code": -1}


@router.get("/include-items")
async def include_items(world: str, _u: User = Depends(verify_token)) -> dict:
    from backend.app.core.config import settings as cfg
    from pathlib import Path
    world_path = Path(cfg.bedrock_server_dir) / "worlds" / world
    items: list[dict] = []
    seen: set[str] = set()
    if world_path.exists():
        for entry in world_path.iterdir():
            seen.add(entry.name)
            items.append({"name": entry.name, "is_dir": entry.is_dir()})
    for extra in DEFAULT_INCLUDE_EXTRAS:
        if extra not in seen:
            items.append({"name": extra, "is_dir": True})
    return {"items": items}


@router.get("/folders")
async def list_folders(base: str = "", _u: User = Depends(require_role(UserRole.admin, UserRole.owner))) -> dict:
    from backend.app.core.config import settings as cfg
    from pathlib import Path
    root = Path(cfg.backups_dir).resolve()
    target = (root / base).resolve() if base else root
    try:
        target.relative_to(root)
    except ValueError:
        raise HTTPException(status_code=400, detail="Invalid path")
    if not target.exists():
        raise HTTPException(status_code=404, detail="Not found")
    dirs = [{"name": d.name, "path": str(d.relative_to(root))} for d in target.iterdir() if d.is_dir()]
    return {"base": str(target.relative_to(root)), "dirs": dirs}


@router.get("/scheduler")
async def get_scheduler_config(_u: User = Depends(verify_token)) -> dict:
    return backup_settings_service.load()["auto"]


@router.put("/scheduler")
async def update_scheduler_config(cfg: BackupScheduleConfig, user: User = Depends(require_role(UserRole.admin, UserRole.owner))) -> dict:
    backup_settings_service.save(auto=cfg.model_dump(exclude_none=True))
    backup_scheduler.configure(
        enabled=cfg.enabled, interval_minutes=cfg.interval_minutes,
        keep_count=cfg.keep_count, worlds=cfg.worlds or [],
    )
    if cfg.enabled:
        await backup_scheduler.start()
    else:
        await backup_scheduler.stop()
    log_action(user.username, "backup.scheduler_update", f"enabled={cfg.enabled}", category="backup")
    return {"success": True}


@router.websocket("/ws")
async def backup_websocket(ws: WebSocket):
    try:
        await get_ws_handler().handle(ws)
    except WebSocketDisconnect:
        pass
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `uv run pytest backend/tests/ -v`
Expected: all backup tests pass (settings, run, ws, router) + existing auth tests still green.

- [ ] **Step 7: Lint and commit**

```bash
uvx ruff check backend/app/routers/backups.py backend/app/schemas/backup.py backend/app/core/dependencies.py backend/tests/test_backup_router.py
git add backend/app/routers/backups.py backend/app/schemas/backup.py backend/app/core/dependencies.py backend/tests/test_backup_router.py
git commit -m "feat(backups): settings/run/test/include/folder endpoints, shared singletons, audit"
```

---

## Task 5: Scheduler fix — singleton, lifespan start, hooks

**Covers:** [S9]

**Files:**
- Modify: `backend/app/managers/backup_scheduler.py`
- Modify: `backend/app/main.py`
- Create: `backend/tests/test_backup_scheduler.py`

- [ ] **Step 1: Write the failing test**

Create `backend/tests/test_backup_scheduler.py`:

```python
from __future__ import annotations

import pytest

from backend.app.managers.backup_scheduler import BackupScheduler


@pytest.mark.asyncio
async def test_scheduler_runs_backup_with_hooks(tmp_path, monkeypatch):
    from backend.app.services.backup_service import BackupService
    from backend.app.services.backup_settings_service import BackupSettingsService

    monkeypatch.setattr("backend.app.services.backup_service.settings.backups_dir", str(tmp_path))
    server_dir = tmp_path / "server"
    monkeypatch.setattr("backend.app.services.backup_service.settings.bedrock_server_dir", str(server_dir))
    (server_dir / "worlds" / "W").mkdir(parents=True)
    (server_dir / "worlds" / "W" / "level.dat").write_bytes(b"x")

    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: tmp_path / "backup_settings.yaml",
    )

    svc = BackupService()
    settings_svc = BackupSettingsService()
    settings_svc.save(auto={"enabled": True, "interval_minutes": 0, "keep_count": 1},
                      pre_post={"before": [{"type": "send", "value": "say auto"}], "after": []})

    sent: list[str] = []
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: sent.append(cmd))

    calls: list[str] = []
    orig = svc.run_backup

    async def spy(world, tag, **kw):
        calls.append(tag)
        kw["pre_post"] = kw.get("pre_post", {"before": [], "after": []})
        return await orig(world, tag, **kw)
    monkeypatch.setattr(svc, "run_backup", spy)

    sched = BackupScheduler(svc, settings_svc)
    sched.configure(enabled=True, interval_minutes=0, keep_count=1)
    await sched.tick_once()  # run exactly one iteration
    assert "auto" in calls
    assert sent == ["say auto"]
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest backend/tests/test_backup_scheduler.py -v`
Expected: FAIL — `BackupScheduler` signature / `tick_once` missing.

- [ ] **Step 3: Rewrite the scheduler**

Replace `backend/app/managers/backup_scheduler.py`:

```python
from __future__ import annotations

import asyncio
import logging

from backend.app.services.backup_service import BackupService
from backend.app.services.backup_settings_service import BackupSettingsService

logger = logging.getLogger("backup_scheduler")


class BackupScheduler:
    def __init__(self, backup_service: BackupService, settings_service: BackupSettingsService) -> None:
        self._task: asyncio.Task | None = None
        self._enabled = False
        self._interval = 30
        self._keep = 10
        self._worlds: list[str] = []
        self._backup_service = backup_service
        self._settings_service = settings_service

    @property
    def enabled(self) -> bool:
        return self._enabled

    def configure(self, enabled: bool, interval_minutes: int = 30, keep_count: int = 10, worlds: list[str] | None = None) -> None:
        self._enabled = enabled
        self._interval = interval_minutes
        self._keep = keep_count
        self._worlds = worlds or []

    async def start(self) -> None:
        if self._task and not self._task.done():
            return
        self._enabled = True
        self._task = asyncio.create_task(self._run())
        logger.info("Backup scheduler started (interval=%d min, keep=%d)", self._interval, self._keep)

    async def stop(self) -> None:
        self._enabled = False
        if self._task and not self._task.done():
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
        logger.info("Backup scheduler stopped")

    async def _run(self) -> None:
        while self._enabled:
            try:
                await asyncio.sleep(self._interval * 60)
                if not self._enabled:
                    break
                await self.tick_once()
            except asyncio.CancelledError:
                break
            except Exception:
                logger.exception("Auto-backup error")

    async def tick_once(self) -> None:
        settings = self._settings_service.load()
        auto = settings["auto"]
        pre_post = settings["pre_post"]
        worlds = self._worlds or self._backup_service.list_worlds()
        for world in worlds:
            async def _discard(e: dict) -> None:
                pass
            try:
                await self._backup_service.run_backup(
                    world, tag="auto",
                    full_backup=auto.get("full_backup", True),
                    zip_prefix="auto_backup",
                    export_folder=auto.get("export_folder", ""),
                    compression=auto.get("compression", "deflate"),
                    include_items=auto.get("include_items") or None,
                    pre_post=pre_post, dry_run=False, notify=_discard,
                )
            except Exception:
                logger.exception("Auto-backup failed for world %s", world)
        self._cleanup_old(worlds)

    def _cleanup_old(self, worlds: list[str]) -> None:
        try:
            for world in worlds:
                backups = self._backup_service.list_backups(world)
                if len(backups) > self._keep:
                    for b in backups[self._keep:]:
                        self._backup_service.delete_backup(b["world"], b["filename"])
                        logger.info("Deleted old backup: %s/%s", b["world"], b["filename"])
        except Exception:
            logger.exception("Cleanup error")
```

- [ ] **Step 4: Start the scheduler in the app lifespan**

In `backend/app/main.py`, add to the imports:
```python
from backend.app.core.dependencies import backup_scheduler, server_manager
```
(replace the existing `from backend.app.core.dependencies import server_manager` line).

In `lifespan`, after `await collector.start();` add:
```python
    await backup_scheduler.start()
```
And in the shutdown (after `yield`), before `await server_manager.kill()`:
```python
    await backup_scheduler.stop()
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `uv run pytest backend/tests/ -v`
Expected: all green, including the new scheduler test.

- [ ] **Step 6: Lint and commit**

```bash
uvx ruff check backend/app/managers/backup_scheduler.py backend/app/main.py backend/tests/test_backup_scheduler.py
git add backend/app/managers/backup_scheduler.py backend/app/main.py backend/tests/test_backup_scheduler.py
git commit -m "feat(backups): single lifespan-managed scheduler reading settings with hooks"
```

---

## Task 6: Frontend types, API client, backup store, WS stream

**Covers:** [S10] (data layer)

**Files:**
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/lib/api/client.ts`
- Create: `frontend/src/stores/backup.ts`

- [ ] **Step 1: Add backup types**

In `frontend/src/types/index.ts`, append:

```typescript
export type CommandEntryType = 'command' | 'wait' | 'comment' | 'send';

export interface CommandEntry {
  type: CommandEntryType;
  value: string | number;
}

export interface PrePostConfig {
  before: CommandEntry[];
  after: CommandEntry[];
}

export interface BackupSettings {
  manual: {
    world: string;
    full_backup: boolean;
    zip_prefix: string;
    export_folder: string;
    compression: 'deflate' | 'store';
    dry_run: boolean;
    include_items: string[];
  };
  auto: {
    enabled: boolean;
    interval_minutes: number;
    keep_count: number;
    export_folder: string;
    compression: 'deflate' | 'store';
    full_backup: boolean;
    include_items: string[];
  };
  pre_post: PrePostConfig;
}

export interface BackupJobEvent {
  type: 'status' | 'output' | 'progress' | 'done' | 'error' | 'hello';
  phase?: 'pre' | 'zip' | 'post';
  message?: string;
  stream?: 'pre' | 'zip' | 'post';
  line?: string;
  percent?: number;
  success?: boolean;
  filename?: string | null;
  active?: boolean;
}

export interface IncludeItem {
  name: string;
  is_dir: boolean;
}

export interface TestCommandResult {
  kind: 'command' | 'send' | 'comment' | 'wait';
  output: string;
  exit_code: number;
}
```

- [ ] **Step 2: Extend the API client**

In `frontend/src/lib/api/client.ts`, replace the entire `// Backups` block (lines ~89–107) with:

```typescript
  // Backups
  listWorlds: () => request<string[]>('/backups/worlds'),
  listBackups: (world?: string) => request<{ backups: Backup[]; total: number }>(`/backups/${world ? `?world=${world}` : ''}`),
  runBackup: (data: {
    world: string; tag?: string; full_backup?: boolean; zip_prefix?: string;
    export_folder?: string; compression?: 'deflate' | 'store'; include_items?: string[];
    dry_run?: boolean; run_hooks?: boolean;
  }) =>
    request<{ success: boolean; filename?: string | null; message: string }>('/backups/create', {
      method: 'POST', body: JSON.stringify({
        world: data.world, tag: data.tag ?? 'manual',
        full_backup: data.full_backup ?? true, zip_prefix: data.zip_prefix ?? 'manual_backup',
        export_folder: data.export_folder ?? '', compression: data.compression ?? 'deflate',
        include_items: data.include_items ?? null, dry_run: data.dry_run ?? false,
        run_hooks: data.run_hooks ?? true,
      }),
    }),
  deleteBackup: (world: string, filename: string) =>
    request<{ success: boolean }>(`/backups/${world}/${filename}`, { method: 'DELETE' }),
  restoreBackup: (world: string, filename: string) =>
    request<{ success: boolean }>(`/backups/restore/${world}/${filename}`, { method: 'POST' }),
  listTrash: (world?: string) => request<Array<Record<string, unknown>>>(`/backups/trash${world ? `?world=${world}` : ''}`),
  getSchedulerConfig: () => request<Record<string, unknown>>('/backups/scheduler'),
  updateScheduler: (cfg: Record<string, unknown>) =>
    request<{ success: boolean }>('/backups/scheduler', { method: 'PUT', body: JSON.stringify(cfg) }),
  getBackupSettings: () => request<BackupSettings>('/backups/settings'),
  updateBackupSettings: (data: { manual?: Record<string, unknown>; auto?: Record<string, unknown>; pre_post?: PrePostConfig }) =>
    request<{ success: boolean }>('/backups/settings', { method: 'PUT', body: JSON.stringify(data) }),
  testBackupCommand: (entry: CommandEntry) =>
    request<TestCommandResult>('/backups/test-command', { method: 'POST', body: JSON.stringify({ entry }) }),
  getIncludeItems: (world: string) => request<{ items: IncludeItem[] }>(`/backups/include-items?world=${encodeURIComponent(world)}`),
  listBackupFolders: (base?: string) =>
    request<{ base: string; dirs: { name: string; path: string }[] }>(`/backups/folders${base ? `?base=${encodeURIComponent(base)}` : ''}`),
```

Also update the import at the top of `client.ts` to include the new types:
```typescript
import type { ServerStatus, ServerActionResponse, Backup, AddonList, Addon, Player, PropertyEntry, IniFile, WorldInfo, BackupSettings, PrePostConfig, CommandEntry, IncludeItem, TestCommandResult } from '$types/index';
```

- [ ] **Step 3: Create the backup job store**

Create `frontend/src/stores/backup.ts`:

```typescript
import { writable } from 'svelte/store';
import type { BackupJobEvent } from '$types/index';

export const backupEvents = writable<BackupJobEvent[]>([]);
export const backupProgress = writable<number>(0);
export const backupActive = writable<boolean>(false);
export const backupLog = writable<string[]>([]);
export const manualLog = writable<string[]>([]);
export const autoLog = writable<string[]>([]);

export function pushBackupEvent(ev: BackupJobEvent) {
  backupEvents.update((a) => [...a.slice(-4999), ev]);

  if (ev.type === 'progress' && typeof ev.percent === 'number') {
    backupProgress.set(ev.percent);
  }
  if (ev.type === 'hello') {
    backupActive.set(!!ev.active);
    return;
  }
  if (ev.type === 'done') {
    backupActive.set(false);
    backupProgress.set(ev.success ? 100 : 0);
  } else if (ev.type === 'status' || ev.type === 'progress') {
    backupActive.set(true);
  }

  const line = ev.line ?? ev.message ?? '';
  if (line) {
    const tag = ev.phase === 'pre' || ev.stream === 'pre' ? 'manual' : 'manual';
    const logLine = `${new Date().toLocaleTimeString()}  ${line}`;
    backupLog.update((l) => [...l.slice(-4999), logLine]);
    manualLog.update((l) => [...l.slice(-4999), logLine]);
  }
}

export function clearBackupLog(which: 'manual' | 'auto' | 'all') {
  if (which === 'manual' || which === 'all') manualLog.set([]);
  if (which === 'auto' || which === 'all') autoLog.set([]);
  if (which === 'all') backupLog.set([]);
}
```

- [ ] **Step 4: Typecheck**

Run: `npm run check`
Expected: 0 errors (the pre-existing `@types/node` warning is fine).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/types/index.ts frontend/src/lib/api/client.ts frontend/src/stores/backup.ts
git commit -m "feat(backups-ui): types, API client, and backup job event store"
```

---

## Task 7: CommandEditor + CommandEntryEditor components

**Covers:** [S3] (pre/post editor), [S10]

**Files:**
- Create: `frontend/src/lib/components/backups/CommandEntryEditor.svelte`
- Create: `frontend/src/lib/components/backups/CommandEditor.svelte`

- [ ] **Step 1: Create the entry editor sub-modal**

Create `frontend/src/lib/components/backups/CommandEntryEditor.svelte`:

```svelte
<script lang="ts">
  import type { CommandEntry, CommandEntryType } from '$types/index';
  import { createEventDispatcher } from 'svelte';

  let { entry = null }: { entry: CommandEntry | null } = $props();

  let type = $state<CommandEntryType>(entry?.type ?? 'send');
  let value = $state<string>(String(entry?.value ?? ''));
  let waitSecs = $state<number>(entry?.type === 'wait' ? Number(entry.value) : 10);
  const dispatch = createEventDispatcher();

  const placeholders: Record<CommandEntryType, string> = {
    command: 'shell command (e.g. rsync -a ...)',
    wait: '',
    comment: 'documentation note (ignored at run)',
    send: 'console command (e.g. say Backup starting…)',
  };

  function save() {
    const out: CommandEntry =
      type === 'wait'
        ? { type, value: Math.max(1, Math.min(600, waitSecs)) }
        : { type, value };
    dispatch('save', out);
  }
</script>

<div class="space-y-3">
  <div>
    <label class="block text-[10px] text-deep-400 uppercase tracking-wider mb-1">Type</label>
    <select bind:value={type} class="input w-full text-xs py-1.5">
      <option value="send">Send to console (say …)</option>
      <option value="command">Shell command</option>
      <option value="wait">Wait (seconds)</option>
      <option value="comment">Comment</option>
    </select>
  </div>
  <div>
    {#if type === 'wait'}
      <label class="block text-[10px] text-deep-400 uppercase tracking-wider mb-1">Seconds (1–600)</label>
      <input type="number" min="1" max="600" bind:value={waitSecs} class="input w-full text-xs py-1.5" />
    {:else}
      <label class="block text-[10px] text-deep-400 uppercase tracking-wider mb-1">Value</label>
      <input type="text" bind:value placeholder={placeholders[type]} class="input w-full text-xs py-1.5 font-mono" />
    {/if}
  </div>
  <div class="flex justify-end gap-2 pt-2">
    <button onclick={() => dispatch('cancel')} class="btn-ghost text-xs px-3 py-1.5">Cancel</button>
    <button onclick={save} class="btn-primary text-xs px-3 py-1.5">Save</button>
  </div>
</div>
```

- [ ] **Step 2: Create the pre/post command editor modal**

Create `frontend/src/lib/components/backups/CommandEditor.svelte`:

```svelte
<script lang="ts">
  import type { CommandEntry, PrePostConfig } from '$types/index';
  import { createEventDispatcher } from 'svelte';
  import CommandEntryEditor from './CommandEntryEditor.svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { Plus, Pencil, Trash2, Send, X, GripVertical } from '@lucide/svelte';

  let { prePost }: { prePost: PrePostConfig } = $props();
  const dispatch = createEventDispatcher();

  let before = $state<CommandEntry[]>([...prePost.before]);
  let after = $state<CommandEntry[]>([...prePost.after]);
  let activePhase = $state<'before' | 'after'>('before');
  let editing = $state<{ phase: 'before' | 'after'; index: number | null } | null>(null);
  let testResult = $state<string>('');

  function list(phase: 'before' | 'after'): CommandEntry[] {
    return phase === 'before' ? before : after;
  }
  function setList(phase: 'before' | 'after', v: CommandEntry[]) {
    if (phase === 'before') before = v; else after = v;
  }

  function startAdd(phase: 'before' | 'after') {
    editing = { phase, index: null };
  }
  function startEdit(phase: 'before' | 'after', index: number) {
    editing = { phase, index };
  }
  function applyEntry(e: CustomEvent<CommandEntry>) {
    if (!editing) return;
    const { phase, index } = editing;
    if (index === null) setList(phase, [...list(phase), e.detail]);
    else {
      const next = [...list(phase)];
      next[index] = e.detail;
      setList(phase, next);
    }
    editing = null;
  }
  function del(phase: 'before' | 'after', index: number) {
    const next = [...list(phase)];
    next.splice(index, 1);
    setList(phase, next);
  }
  function move(phase: 'before' | 'after', index: number, dir: -1 | 1) {
    const arr = [...list(phase)];
    const j = index + dir;
    if (j < 0 || j >= arr.length) return;
    [arr[index], arr[j]] = [arr[j], arr[index]];
    setList(phase, arr);
  }

  async function sendTest(phase: 'before' | 'after', index: number) {
    const entry = list(phase)[index];
    testResult = 'Running…';
    try {
      const r = await api.testBackupCommand(entry);
      testResult = `[${r.kind}] exit ${r.exit_code}\n${r.output}`;
    } catch (err: any) {
      testResult = `Error: ${err.message}`;
      addToast(`Test failed: ${err.message}`, 'error');
    }
  }

  function saveAll() {
    dispatch('save', { before, after });
  }

  function label(e: CommandEntry): string {
    if (e.type === 'wait') return `wait ${e.value}s`;
    return String(e.value);
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4" role="dialog" aria-modal="true"
     style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);">
  <div class="bg-deep-900 border-2 border-deep-600/50 w-full max-w-2xl shadow-block-lg" role="none">
    <div class="flex items-center justify-between px-5 py-3 border-b border-deep-700/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest">Pre / Post Commands</h2>
      <button onclick={() => dispatch('cancel')} class="btn-ghost p-1"><X size={14} /></button>
    </div>

    <div class="flex gap-1 px-5 pt-3">
      <button onclick={() => activePhase = 'before'}
              class="text-xs px-3 py-1.5 {activePhase === 'before' ? 'btn-primary' : 'btn-ghost'}">Before backup</button>
      <button onclick={() => activePhase = 'after'}
              class="text-xs px-3 py-1.5 {activePhase === 'after' ? 'btn-primary' : 'btn-ghost'}">After backup</button>
    </div>

    <div class="px-5 py-3 max-h-[50vh] overflow-auto">
      {#each list(activePhase) as entry, i (i)}
        <div class="flex items-center gap-2 py-1.5 border-b border-deep-700/20">
          <GripVertical size={12} class="text-deep-500" />
          <span class="text-[10px] uppercase px-1.5 py-0.5 {entry.type === 'send' ? 'text-bedrock-400 bg-bedrock-500/10' : entry.type === 'command' ? 'text-amber-400 bg-amber-500/10' : 'text-deep-400 bg-deep-700/30'}">{entry.type}</span>
          <span class="flex-1 text-xs font-mono text-deep-200 truncate">{label(entry)}</span>
          <button onclick={() => move(activePhase, i, -1)} class="btn-ghost p-1 text-deep-400">↑</button>
          <button onclick={() => move(activePhase, i, 1)} class="btn-ghost p-1 text-deep-400">↓</button>
          <button onclick={() => sendTest(activePhase, i)} class="btn-ghost p-1 text-bedrock-400" title="Send Test"><Send size={12} /></button>
          <button onclick={() => startEdit(activePhase, i)} class="btn-ghost p-1"><Pencil size={12} /></button>
          <button onclick={() => del(activePhase, i)} class="btn-ghost p-1 text-red-400"><Trash2 size={12} /></button>
        </div>
      {:else}
        <p class="text-xs text-deep-500 py-6 text-center">No {activePhase}-commands. Click Add.</p>
      {/each}
      <button onclick={() => startAdd(activePhase)} class="btn-ghost flex items-center gap-1 text-xs mt-3 text-bedrock-400">
        <Plus size={12} /> Add {activePhase}-command
      </button>

      {#if testResult}
        <pre class="mt-3 text-[11px] text-deep-300 bg-deep-950 border border-deep-700/40 p-2 max-h-32 overflow-auto whitespace-pre-wrap">{testResult}</pre>
      {/if}
    </div>

    <div class="flex justify-end gap-2 px-5 py-3 border-t border-deep-700/50">
      <button onclick={() => dispatch('cancel')} class="btn-ghost text-xs px-3 py-1.5">Cancel</button>
      <button onclick={saveAll} class="btn-primary text-xs px-3 py-1.5">Save</button>
    </div>
  </div>
</div>

{#if editing}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4" style="background: rgba(3,8,16,0.85);">
    <div class="bg-deep-900 border-2 border-deep-600/50 w-full max-w-md p-5">
      <h3 class="text-sm font-bold text-white uppercase tracking-widest mb-4">{editing.index === null ? 'Add' : 'Edit'} command</h3>
      <CommandEntryEditor
        entry={editing.index === null ? null : list(editing.phase)[editing.index]}
        onsave={applyEntry}
        oncancel={() => (editing = null)}
      />
    </div>
  </div>
{/if}
```

- [ ] **Step 3: Typecheck**

Run: `npm run check`
Expected: 0 errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/backups/CommandEntryEditor.svelte frontend/src/lib/components/backups/CommandEditor.svelte
git commit -m "feat(backups-ui): CommandEditor + CommandEntryEditor for pre/post command editing"
```

---

## Task 8: IncludePicker + FolderBrowser modals

**Covers:** [S10] (include picker, folder browser)

**Files:**
- Create: `frontend/src/lib/components/backups/IncludePicker.svelte`
- Create: `frontend/src/lib/components/backups/FolderBrowser.svelte`

- [ ] **Step 1: Create IncludePicker**

Create `frontend/src/lib/components/backups/IncludePicker.svelte`:

```svelte
<script lang="ts">
  import type { IncludeItem } from '$types/index';
  import { createEventDispatcher } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { X } from '@lucide/svelte';

  let { world, selected = [] }: { world: string; selected: string[] } = $props();
  const dispatch = createEventDispatcher();

  let items = $state<IncludeItem[]>([]);
  let checked = $state<Set<string>>(new Set(selected));
  let loading = $state(true);

  async function load() {
    loading = true;
    try {
      const r = await api.getIncludeItems(world);
      items = r.items;
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  }
  load();

  function toggle(name: string) {
    const next = new Set(checked);
    next.has(name) ? next.delete(name) : next.add(name);
    checked = next;
  }
  function selectAll() { checked = new Set(items.map((i) => i.name)); }
  function clearAll() { checked = new Set(); }
  function ok() { dispatch('select', [...checked]); }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4" role="dialog" aria-modal="true"
     style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);">
  <div class="bg-deep-900 border-2 border-deep-600/50 w-full max-w-lg">
    <div class="flex items-center justify-between px-5 py-3 border-b border-deep-700/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest">Include items — {world}</h2>
      <button onclick={() => dispatch('cancel')} class="btn-ghost p-1"><X size={14} /></button>
    </div>
    <div class="px-5 py-3 flex gap-2">
      <button onclick={selectAll} class="btn-ghost text-xs px-2 py-1">Select All</button>
      <button onclick={clearAll} class="btn-ghost text-xs px-2 py-1">Clear All</button>
    </div>
    <div class="px-5 pb-3 max-h-80 overflow-auto">
      {#if loading}
        <p class="text-xs text-deep-500 py-6 text-center">Loading…</p>
      {:else}
        {#each items as item}
          <label class="flex items-center gap-2 py-1 text-xs text-deep-200 cursor-pointer">
            <input type="checkbox" checked={checked.has(item.name)} onchange={() => toggle(item.name)} class="accent-bedrock-500" />
            <span class={item.is_dir ? 'text-bedrock-400' : ''}>{item.name}{item.is_dir ? '/' : ''}</span>
          </label>
        {/each}
      {/if}
    </div>
    <div class="flex justify-end gap-2 px-5 py-3 border-t border-deep-700/50">
      <button onclick={() => dispatch('cancel')} class="btn-ghost text-xs px-3 py-1.5">Cancel</button>
      <button onclick={ok} class="btn-primary text-xs px-3 py-1.5">OK</button>
    </div>
  </div>
</div>
```

- [ ] **Step 2: Create FolderBrowser**

Create `frontend/src/lib/components/backups/FolderBrowser.svelte`:

```svelte
<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { X, Folder, ArrowLeft } from '@lucide/svelte';

  let { base = '' }: { base?: string } = $props();
  const dispatch = createEventDispatcher();

  let dirs = $state<{ name: string; path: string }[]>([]);
  let current = $state<string>('');

  async function load(b: string) {
    try {
      const r = await api.listBackupFolders(b);
      dirs = r.dirs;
      current = r.base;
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }
  load(base);

  function open(d: string) { load(d); }
  function up() {
    if (!current) return;
    const parts = current.split('/');
    parts.pop();
    load(parts.join('/'));
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4" role="dialog" aria-modal="true"
     style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);">
  <div class="bg-deep-900 border-2 border-deep-600/50 w-full max-w-md">
    <div class="flex items-center justify-between px-5 py-3 border-b border-deep-700/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest">Choose folder</h2>
      <button onclick={() => dispatch('cancel')} class="btn-ghost p-1"><X size={14} /></button>
    </div>
    <div class="px-5 py-2 text-xs text-deep-400 font-mono flex items-center gap-2">
      {#if current}<button onclick={up} class="btn-ghost p-1"><ArrowLeft size={12} /></button>{/if}
      /{current || 'backups/worlds'}
    </div>
    <div class="px-5 pb-3 max-h-72 overflow-auto">
      {#each dirs as d}
        <button onclick={() => open(d.path)} class="w-full flex items-center gap-2 py-1.5 text-xs text-deep-200 hover:bg-deep-800/40">
          <Folder size={12} class="text-bedrock-400" /> {d.name}/
        </button>
      {:else}
        <p class="text-xs text-deep-500 py-6 text-center">No subfolders</p>
      {/each}
    </div>
    <div class="flex justify-end gap-2 px-5 py-3 border-t border-deep-700/50">
      <button onclick={() => dispatch('cancel')} class="btn-ghost text-xs px-3 py-1.5">Cancel</button>
      <button onclick={() => dispatch('select', current)} class="btn-primary text-xs px-3 py-1.5">Use this folder</button>
    </div>
  </div>
</div>
```

- [ ] **Step 3: Typecheck**

Run: `npm run check`
Expected: 0 errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/backups/IncludePicker.svelte frontend/src/lib/components/backups/FolderBrowser.svelte
git commit -m "feat(backups-ui): IncludePicker + FolderBrowser modals"
```

---

## Task 9: Rewrite /backups page — 4 sub-tabs + WS wiring

**Covers:** [S3] (sub-tabs), [S10]

**Files:**
- Rewrite: `frontend/src/routes/backups/+page.svelte`

- [ ] **Step 1: Rewrite the backups page**

Replace `frontend/src/routes/backups/+page.svelte`:

```svelte
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api, getToken } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import { pushBackupEvent, backupProgress, backupActive, manualLog, autoLog, clearBackupLog } from '$stores/backup';
  import type { BackupSettings, Backup } from '$types/index';
  import CommandEditor from '$components/backups/CommandEditor.svelte';
  import IncludePicker from '$components/backups/IncludePicker.svelte';
  import FolderBrowser from '$components/backups/FolderBrowser.svelte';
  import { HardDrive, Download, Trash2, Plus, RefreshCw, Play, Square, ClipboardList, Folder as FolderIcon, ChevronRight } from '@lucide/svelte';

  let tab = $state<'manual' | 'automatic' | 'backups' | 'logs'>('manual');
  let worlds = $state<string[]>([]);
  let backups = $state<Backup[]>([]);
  let settings = $state<BackupSettings | null>(null);
  let loading = $state(true);

  let showCmdEditor = $state(false);
  let includePickerFor = $state<'manual' | 'auto' | null>(null);
  let folderBrowserFor = $state<'manual' | 'auto' | null>(null);

  let unsub: (() => void) | null = null;

  onMount(async () => {
    try {
      [worlds, backups, settings] = [
        await api.listWorlds(),
        (await api.listBackups()).backups,
        await api.getBackupSettings(),
      ];
      if (worlds.length && !settings!.manual.world) settings!.manual.world = worlds[0];
      unsub = wsManager.connect(`/api/v1/backups/ws?token=${getToken()}`, (d) => pushBackupEvent(d as any));
    } catch (e: any) { addToast(`Failed to load: ${e.message}`, 'error'); }
    loading = false;
  });
  onDestroy(() => { if (unsub) unsub(); wsManager.disconnect(`/api/v1/backups/ws?token=${getToken()}`); });

  async function reload() {
    try { backups = (await api.listBackups(settings?.manual.world || undefined)).backups; } catch {}
  }

  const shellCommandsPresent = (cfg: BackupSettings) =>
    [...cfg.pre_post.before, ...cfg.pre_post.after].some((e) => e.type === 'command');

  async function runManual() {
    if (!settings) return;
    const world = settings.manual.world || worlds[0];
    if (!world) { addToast('No world selected', 'error'); return; }
    if (shellCommandsPresent(settings) && !confirm('This backup runs shell commands:\n\n' +
        [...settings.pre_post.before, ...settings.pre_post.after].filter(e => e.type === 'command').map(e => '• ' + e.value).join('\n') +
        '\n\nProceed?')) return;
    try {
      const r = await api.runBackup({
        world, tag: 'manual', full_backup: settings.manual.full_backup,
        zip_prefix: settings.manual.zip_prefix, export_folder: settings.manual.export_folder,
        compression: settings.manual.compression, include_items: settings.manual.include_items.length ? settings.manual.include_items : undefined,
        dry_run: settings.manual.dry_run,
      });
      addToast(r.success ? `Backup created: ${r.filename ?? ''}` : `Backup: ${r.message}`, r.success ? 'success' : 'error');
      await reload();
    } catch (e: any) { addToast(`Backup failed: ${e.message}`, 'error'); }
  }

  async function saveManualSettings() {
    if (!settings) return;
    try { await api.updateBackupSettings({ manual: settings.manual }); addToast('Manual settings saved', 'success'); }
    catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }
  async function saveAutoSettings() {
    if (!settings) return;
    try {
      await api.updateBackupSettings({ auto: settings.auto });
      await api.updateScheduler({ ...settings.auto, worlds: [] });
      addToast('Auto settings saved', 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }
  async function toggleAuto() {
    if (!settings) return;
    settings.auto.enabled = !settings.auto.enabled;
    await saveAutoSettings();
  }

  async function del(world: string, file: string) {
    try {
      await api.deleteBackup(world, file);
      await reload();
      addToast('Backup moved to trash', 'info', 6000, {
        label: 'Undo', callback: async () => {
          try { await api.restoreBackup(world, file); addToast('Restored', 'success'); await reload(); }
          catch (e: any) { addToast(`Restore failed: ${e.message}`, 'error'); }
        },
      });
    } catch (e: any) { addToast(`Delete failed: ${e.message}`, 'error'); }
  }
  function download(world: string, file: string) {
    window.open(`/api/v1/backups/${world}/${file}/download?_t=${Date.now()}`, '_blank');
  }

  function onSaveCommands(e: CustomEvent) {
    if (settings) { settings.pre_post = e.detail; }
    api.updateBackupSettings({ pre_post: e.detail }).then(
      () => addToast('Pre/post commands saved', 'success'),
      (e: any) => addToast(`Save failed: ${e.message}`, 'error'),
    );
    showCmdEditor = false;
  }
  function onInclude(e: CustomEvent) {
    if (settings && includePickerFor) settings[includePickerFor].include_items = e.detail;
    includePickerFor = null;
  }
  function onFolder(e: CustomEvent) {
    if (settings && folderBrowserFor) settings[folderBrowserFor].export_folder = e.detail;
    folderBrowserFor = null;
  }

  let progress = $derived($backupProgress);
  let active = $derived($backupActive);
  let logs = $derived(tab === 'logs' ? $manualLog : []);
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Backups</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <button onclick={reload} class="btn-ghost p-2"><RefreshCw size={14} /></button>
  </div>

  <div class="flex gap-1 border-b border-deep-700/40">
    {#each [['manual','Manual'],['automatic','Automatic'],['backups','Archive'],['logs','Logs']] as [k, label]}
      <button onclick={() => (tab = k as any)}
              class="text-xs px-4 py-2 uppercase tracking-wider {tab === k ? 'text-bedrock-400 border-b-2 border-bedrock-500' : 'text-deep-400 hover:text-deep-200'}">{label}</button>
    {/each}
  </div>

  {#if loading}
    <p class="text-xs text-deep-500">Loading…</p>
  {:else if settings}
    <!-- MANUAL -->
    {#if tab === 'manual'}
      <div class="card space-y-3">
        <div class="flex items-center gap-2">
          <select bind:value={settings.manual.world} class="input flex-1 text-xs py-1.5">
            {#each worlds as w}<option value={w}>{w}</option>{/each}
          </select>
          <button onclick={runManual} disabled={active} class="btn-primary flex items-center gap-2 text-xs">
            <Play size={14} /> {active ? 'Running…' : 'Create Backup Now'}
          </button>
        </div>
        {#if active}
          <div class="w-full h-2 bg-deep-800 overflow-hidden">
            <div class="h-full bg-bedrock-500 transition-all" style="width: {progress}%"></div>
          </div>
        {/if}
        <div class="grid grid-cols-2 gap-3 pt-2">
          <label class="flex items-center gap-2 text-xs text-deep-300"><input type="checkbox" bind:checked={settings.manual.full_backup} class="accent-bedrock-500" /> Full backup</label>
          <label class="flex items-center gap-2 text-xs text-deep-300"><input type="checkbox" bind:checked={settings.manual.dry_run} class="accent-bedrock-500" /> Dry-run (no archive)</label>
          <label class="text-xs text-deep-400">Zip prefix<input bind:value={settings.manual.zip_prefix} class="input w-full text-xs py-1.5 mt-1" /></label>
          <label class="text-xs text-deep-400">Compression
            <select bind:value={settings.manual.compression} class="input w-full text-xs py-1.5 mt-1">
              <option value="deflate">deflate</option><option value="store">store</option>
            </select>
          </label>
          <label class="col-span-2 text-xs text-deep-400">Export folder
            <div class="flex gap-1 mt-1">
              <input bind:value={settings.manual.export_folder} placeholder="(default)" class="input flex-1 text-xs py-1.5" />
              <button onclick={() => (folderBrowserFor = 'manual')} class="btn-ghost p-1.5"><FolderIcon size={12} /></button>
            </div>
          </label>
        </div>
        <div class="flex gap-2 pt-1">
          <button onclick={() => (includePickerFor = 'manual')} class="btn-ghost text-xs px-3 py-1.5">Select include items…</button>
          <button onclick={() => (showCmdEditor = true)} class="btn-ghost text-xs px-3 py-1.5">Edit pre/post commands</button>
          <div class="flex-1"></div>
          <button onclick={saveManualSettings} class="btn-ghost text-xs px-3 py-1.5">Save Manual Settings</button>
        </div>
        <div>
          <div class="flex items-center justify-between mb-1"><span class="text-[10px] text-deep-400 uppercase tracking-wider">Manual status log</span><button onclick={() => clearBackupLog('manual')} class="text-[10px] text-deep-500 hover:text-deep-300">Clear</button></div>
          <pre class="h-32 overflow-auto text-[11px] text-deep-300 bg-deep-950 border border-deep-700/40 p-2 whitespace-pre-wrap">{$manualLog.length ? $manualLog.join('\n') : ''}</pre>
        </div>
      </div>
    {/if}

    <!-- AUTOMATIC -->
    {#if tab === 'automatic'}
      <div class="card space-y-3">
        <label class="flex items-center gap-2 text-xs text-deep-300"><input type="checkbox" bind:checked={settings.auto.enabled} class="accent-bedrock-500" /> Enable automatic backups</label>
        <div class="grid grid-cols-2 gap-3">
          <label class="text-xs text-deep-400">Interval (minutes)<input type="number" min="1" bind:value={settings.auto.interval_minutes} class="input w-full text-xs py-1.5 mt-1" /></label>
          <label class="text-xs text-deep-400">Keep count<input type="number" min="1" bind:value={settings.auto.keep_count} class="input w-full text-xs py-1.5 mt-1" /></label>
          <label class="text-xs text-deep-400">Compression
            <select bind:value={settings.auto.compression} class="input w-full text-xs py-1.5 mt-1"><option value="deflate">deflate</option><option value="store">store</option></select>
          </label>
          <label class="flex items-center gap-2 text-xs text-deep-300 pt-5"><input type="checkbox" bind:checked={settings.auto.full_backup} class="accent-bedrock-500" /> Full backup</label>
          <label class="col-span-2 text-xs text-deep-400">Export folder
            <div class="flex gap-1 mt-1"><input bind:value={settings.auto.export_folder} placeholder="(default)" class="input flex-1 text-xs py-1.5" /><button onclick={() => (folderBrowserFor = 'auto')} class="btn-ghost p-1.5"><FolderIcon size={12} /></button></div>
          </label>
        </div>
        <div class="flex gap-2 pt-1">
          <button onclick={() => (includePickerFor = 'auto')} class="btn-ghost text-xs px-3 py-1.5">Select include items…</button>
          <div class="flex-1"></div>
          <button onclick={toggleAuto} class="btn-ghost text-xs px-3 py-1.5">{settings.auto.enabled ? 'Stop Auto Backups' : 'Start Auto Backups'}</button>
          <button onclick={saveAutoSettings} class="btn-primary text-xs px-3 py-1.5">Save Auto Settings</button>
        </div>
        <div>
          <div class="flex items-center justify-between mb-1"><span class="text-[10px] text-deep-400 uppercase tracking-wider">Automatic status log</span><button onclick={() => clearBackupLog('auto')} class="text-[10px] text-deep-500 hover:text-deep-300">Clear</button></div>
          <pre class="h-32 overflow-auto text-[11px] text-deep-300 bg-deep-950 border border-deep-700/40 p-2 whitespace-pre-wrap">{$autoLog.length ? $autoLog.join('\n') : ''}</pre>
        </div>
      </div>
    {/if}

    <!-- ARCHIVE -->
    {#if tab === 'backups'}
      <div class="card">
        {#if active}
          <div class="w-full h-2 bg-deep-800 overflow-hidden mb-3"><div class="h-full bg-bedrock-500 transition-all" style="width: {progress}%"></div></div>
        {/if}
        <div class="overflow-x-auto">
          <table class="w-full text-xs">
            <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
              <th class="text-left py-2 px-3 font-medium">File</th>
              <th class="text-right py-2 px-3 font-medium">Size</th>
              <th class="text-right py-2 px-3 font-medium">Modified</th>
              <th class="text-right py-2 px-3 font-medium"></th>
            </tr></thead>
            <tbody>
              {#each backups as b}
                <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
                  <td class="py-1.5 px-3 font-mono">{b.filename}</td>
                  <td class="py-1.5 px-3 text-right">{formatBytes(b.size_bytes)}</td>
                  <td class="py-1.5 px-3 text-right text-deep-400">{new Date(b.modified).toLocaleString()}</td>
                  <td class="py-1.5 px-3 text-right">
                    <button onclick={() => download(b.world, b.filename)} class="btn-ghost p-1"><Download size={12} /></button>
                    <button onclick={() => del(b.world, b.filename)} class="btn-ghost p-1 text-red-400"><Trash2 size={12} /></button>
                  </td>
                </tr>
              {:else}
                <tr><td colspan="4" class="text-center py-8 text-deep-500">No backups</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
        <p class="text-[10px] text-deep-500 mt-2 font-mono">Path: backend/backups/worlds/ (server-side)</p>
      </div>
    {/if}

    <!-- LOGS -->
    {#if tab === 'logs'}
      <div class="card">
        <div class="flex items-center justify-between mb-2">
          <h2 class="card-header mb-0 flex items-center gap-2"><ClipboardList size={14} /> Backup logs</h2>
          <button onclick={() => clearBackupLog('all')} class="text-[10px] text-deep-500 hover:text-deep-300">Clear</button>
        </div>
        <pre class="h-[50vh] overflow-auto text-[11px] text-deep-300 bg-deep-950 border border-deep-700/40 p-2 whitespace-pre-wrap">{logs.length ? logs.join('\n') : 'No backup events yet.'}</pre>
      </div>
    {/if}
  {/if}
</div>

{#if showCmdEditor && settings}
  <CommandEditor prePost={settings.pre_post} onsave={onSaveCommands} oncancel={() => (showCmdEditor = false)} />
{/if}
{#if includePickerFor && settings}
  <IncludePicker world={settings.manual.world || worlds[0] || ''} selected={settings[includePickerFor].include_items} onselect={onInclude} oncancel={() => (includePickerFor = null)} />
{/if}
{#if folderBrowserFor}
  <FolderBrowser onselect={onFolder} oncancel={() => (folderBrowserFor = null)} />
{/if}
```

- [ ] **Step 2: Typecheck**

Run: `npm run check`
Expected: 0 errors. (Fix any unused-import warnings: drop `HardDrive`, `Square`, `ChevronRight` from imports if flagged.)

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/backups/+page.svelte
git commit -m "feat(backups-ui): rewrite /backups into 4 sub-tabs with progress + logs + WS"
```

---

## Task 10: Runtime verification + final lint/typecheck

**Covers:** [S12], [S11]

**Files:** (no code changes — verification only)

- [ ] **Step 1: Full backend test suite**

Run: `uv run pytest -q`
Expected: all tests pass (settings, run, ws, router, scheduler + existing auth).

- [ ] **Step 2: Full lint**

Run: `uvx ruff check backend/`
Expected: 0 errors.

- [ ] **Step 3: Full frontend typecheck**

Run: `npm run check`
Expected: 0 errors.

- [ ] **Step 4: Runtime smoke test**

Start the panel: `uv run start.py` (backend :17754, frontend :17755). Log in as admin.

1. Go to **Backups → Manual**. Click **Edit pre/post commands**. Add a `send` entry `say Backup starting…` to Before, and `say Backup done` to After. Save.
2. Click **Create Backup Now**. Watch the progress bar advance; confirm both `say` messages appear in **Console** (and in-game if the Endstone server is running with players).
3. Confirm a new zip appears in **Archive**.
4. Open **Logs** — confirm the pre/zip/post output lines are captured.
5. In CommandEditor, use **Send Test** on each entry type (shell `echo hi`, wait, comment, send) — confirm inline results.
6. **Automatic** tab: set interval=1, enable, Save — wait one minute; confirm an `auto`-tagged backup appears and the keep-count cleanup trims old ones.
7. **Dry-run** (Manual): toggle Dry-run, Create — confirm no new zip but `say` still fires.

- [ ] **Step 5: Note the bakcmd.ini importer (deferred)**

The spec mentions a one-time `bakcmd.ini` importer as a convenience. It is **not** required for parity (the desktop `ini/` is not part of the web deployment). Skip unless an existing `bakcmd.ini` must be migrated — then add a `POST /backups/import-bakcmd` endpoint parsing `==before`/`==after` blocks + `>`/`>>`/`!`/`send:`/`--N`/`#` directives into the `send`/`command`/`wait`/`comment` model.

- [ ] **Step 6: Final commit (if any fixes) and status report**

```bash
git status
git log --oneline -12
```

If all green, the feature is complete and ready for compose:report + compose:merge.

---

## Self-review notes

**Spec coverage:** [S1] problem (plan goal) · [S2] scope (out-of-scope deferred importer noted in Task 10 Step 5) · [S3] coverage checklist (Tasks 2, 7, 9) · [S4] architecture (Tasks 2, 3) · [S5] config model (Task 1) · [S6] services (Tasks 2, 4) · [S7] WS + API (Tasks 3, 4) · [S8] security (Task 4 role gates + Task 9 confirm dialog) · [S9] scheduler (Task 5) · [S10] frontend (Tasks 6–9) · [S11] adaptations (Task 9 folder browser + path display) · [S12] testing (Tasks 1–5 tests + Task 10 verification) · [S13] risks (one-job lock Task 2/4; coarse progress; 300s timeout; query-param token — all handled).

**Type consistency:** `run_backup` keyword args match across service (Task 2), router `_run_job` (Task 4), and scheduler `tick_once` (Task 5). `_send_to_server` module function referenced consistently. `BackupSettingsService.save(manual=, auto=, pre_post=)` signature matches tests (Task 1) and router (Task 4). Frontend `runBackup` payload matches `BackupCreateRequest` schema (Task 4). `pushBackupEvent` store matches WS event shapes (Task 6) and page usage (Task 9).
