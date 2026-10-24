from __future__ import annotations

import asyncio
import logging
import os
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


def validate_pre_post_commands(pre_post: dict) -> tuple[bool, str]:
    before = pre_post.get("before", [])
    after = pre_post.get("after", [])
    has_save_hold = any(
        e.get("type") == "send" and e.get("value") == "save hold"
        for e in before
    )
    has_save_resume = any(
        e.get("type") == "send" and e.get("value") == "save resume"
        for e in after
    )
    if not has_save_hold and not has_save_resume:
        return False, "Before commands must include 'save hold' (send type) and After commands must include 'save resume' (send type)"
    if not has_save_hold:
        return False, "Before commands must include 'save hold' (send type)"
    if not has_save_resume:
        return False, "After commands must include 'save resume' (send type)"
    return True, ""


async def _send_to_server(cmd: str, notify: NotifyFn | None = None) -> None:
    from backend.app.core.dependencies import server_manager

    # Subscribe to stdout BEFORE queuing the command to avoid missing output
    q: asyncio.Queue[str] | None = None
    if notify is not None:
        q = server_manager.subscribe_stdout()

    await server_manager.send_command(cmd)
    await asyncio.sleep(0)  # yield so _writer_loop can pick up the command

    if q is not None:
        try:
            # Wait up to 5s for first line of server response
            try:
                line = await asyncio.wait_for(q.get(), timeout=5.0)
                await notify({"type": "output", "stream": "server", "line": line})
            except asyncio.TimeoutError:
                return  # no output within 5s
            # Keep reading until 1s gap signals end of output
            while True:
                try:
                    line = await asyncio.wait_for(q.get(), timeout=1.0)
                    await notify({"type": "output", "stream": "server", "line": line})
                except asyncio.TimeoutError:
                    break
        finally:
            server_manager.unsubscribe_stdout(q)


NotifyFn = Callable[[dict], Awaitable[None]]


class BackupService:
    def __init__(self) -> None:
        self._backup_root = Path(settings.backups_dir) / "worlds"
        self._backup_root.mkdir(parents=True, exist_ok=True)
        self._worlds_dir = Path(settings.bedrock_server_dir) / "worlds"
        self._worlds_dir.mkdir(parents=True, exist_ok=True)
        self._active = False
        self._subscribers: set[Callable[[dict], Awaitable[None]]] = set()

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
                        "filename": f.name,
                        "world": d.name,
                        "size_bytes": stat.st_size,
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
                        "filename": f.name,
                        "world": d.parent.name,
                        "size_bytes": stat.st_size,
                        "modified": datetime.fromtimestamp(stat.st_mtime, tz=timezone.utc).isoformat(),
                    })
        return sorted(items, key=lambda x: x["modified"], reverse=True)

    def get_backup_path(self, world: str, filename: str) -> Path | None:
        path = self._backup_root / world / filename
        return path if path.exists() else None

    async def restore_to_world(self, world: str, filename: str) -> None:
        backup_path = self._backup_root / world / filename
        if not backup_path.exists():
            raise FileNotFoundError(f"Backup '{filename}' for world '{world}' not found")
        world_path = self._worlds_dir / world
        if not world_path.exists():
            raise FileNotFoundError(f"World directory '{world}' not found")

        backup_dir = world_path.parent / f"{world}.bak.{datetime.now().strftime('%Y%m%d-%H%M%S')}"
        logger.info("Saving current world to %s", backup_dir)
        loop = asyncio.get_event_loop()

        def _restore():
            shutil.move(str(world_path), str(backup_dir))
            with zipfile.ZipFile(str(backup_path), "r") as zf:
                zf.extractall(str(self._worlds_dir))
            os.chmod(str(world_path), 0o755)

        await loop.run_in_executor(None, _restore)
        logger.info("Restored world '%s' from backup '%s'", world, filename)

    async def _create_zip(
        self,
        world: str,
        backup_path: Path,
        full_backup: bool,
        compression: str,
        include_items: list[str] | None,
    ) -> None:
        world_path = self._worlds_dir / world
        if not world_path.exists():
            raise FileNotFoundError(f"World '{world}' not found")
        zip_compression = zipfile.ZIP_DEFLATED if compression == "deflate" else zipfile.ZIP_STORED
        loop = asyncio.get_event_loop()

        def _zip_work():
            with zipfile.ZipFile(str(backup_path), "w", zip_compression) as zf:
                items = list(world_path.iterdir()) if full_backup else [
                    world_path / n for n in (include_items or [])
                ]
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

    async def _broadcast(self, event: dict) -> None:
        for sub in list(self._subscribers):
            try:
                await sub(event)
            except Exception:
                logger.debug("backup subscriber dropped", exc_info=True)

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

        async def _notify(event: dict) -> None:
            await notify(event)
            await self._broadcast(event)

        try:
            await _notify({"type": "status", "phase": "pre", "message": "Starting backup"})
            await _notify({"type": "progress", "percent": 5, "message": "pre-commands"})
            if run_hooks and pre_post:
                await self._run_phase("pre", pre_post.get("before", []), dry_run, _notify)

            if not dry_run:
                await _notify({"type": "status", "phase": "zip", "message": "Creating archive"})
                await _notify({"type": "progress", "percent": 40, "message": "zipping"})
                backup_dir = self._resolve_backup_dir(world, export_folder)
                backup_dir.mkdir(parents=True, exist_ok=True)
                timestamp = datetime.now().strftime("%Y-%m-%d-%H%M%S")
                tag_part = f"_{tag}" if tag else ""
                filename = f"{zip_prefix}{tag_part}_{world}_{timestamp}.zip"
                backup_path = backup_dir / filename
                await self._create_zip(world, backup_path, full_backup, compression, include_items)
                size = backup_path.stat().st_size
                await _notify({"type": "output", "stream": "zip",
                               "line": f"Created {filename} ({size / 1024:.1f} KB)"})
            else:
                await _notify({"type": "status", "phase": "zip", "message": "Dry-run: no archive written"})

            await _notify({"type": "progress", "percent": 85, "message": "post-commands"})
            if run_hooks and pre_post:
                await self._run_phase("post", pre_post.get("after", []), dry_run, _notify)

            await _notify({"type": "progress", "percent": 100, "message": "done"})
            await _notify({"type": "done", "success": True, "filename": filename,
                           "message": "Backup complete"})
            return {"success": True, "filename": filename, "message": "Backup complete"}
        except Exception as e:
            logger.exception("Backup failed")
            await _notify({"type": "error", "message": str(e)})
            await _notify({"type": "done", "success": False, "filename": None, "message": str(e)})
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
            await notify({"type": "output", "stream": phase, "line": f"[send] {value}"})
            await _send_to_server(str(value), notify=notify)
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

    async def create_backup(
        self,
        world: str,
        tag: str = "",
        full_backup: bool = True,
        compress: bool = True,
        include_items: list[str] | None = None,
        progress: asyncio.Queue | None = None,
    ) -> str:
        async def _discard(event: dict) -> None:
            if progress is not None:
                progress.put_nowait(event.get("message", ""))

        result = await self.run_backup(
            world, tag, full_backup=full_backup, zip_prefix="manual_backup",
            export_folder="", compression="deflate" if compress else "store",
            include_items=include_items, pre_post={"before": [], "after": []},
            dry_run=False, notify=_discard, run_hooks=False,
        )
        return result["message"]
