from __future__ import annotations

import asyncio
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from backend.app.core.config import settings


class BackupService:
    def __init__(self) -> None:
        self._backup_root = Path(settings.backups_dir) / "worlds"
        self._backup_root.mkdir(parents=True, exist_ok=True)
        self._worlds_dir = Path(settings.bedrock_server_dir) / "worlds"
        self._worlds_dir.mkdir(parents=True, exist_ok=True)

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

    async def create_backup(
        self,
        world: str,
        tag: str = "",
        full_backup: bool = True,
        compress: bool = True,
        include_items: list[str] | None = None,
        progress: asyncio.Queue[str] | None = None,
    ) -> str:
        world_path = self._worlds_dir / world
        if not world_path.exists():
            msg = f"World '{world}' not found"
            if progress:
                progress.put_nowait(msg)
            return msg

        backup_dir = self._backup_root / world
        backup_dir.mkdir(parents=True, exist_ok=True)

        timestamp = datetime.now().strftime("%Y-%m-%d-%H%M%S")
        tag_part = f"_{tag}" if tag else ""
        ext = ".zip"
        backup_name = f"{world}{tag_part}_{timestamp}{ext}"
        backup_path = backup_dir / backup_name

        compression = zipfile.ZIP_DEFLATED if compress else zipfile.ZIP_STORED

        if progress:
            progress.put_nowait(f"Creating backup: {backup_name}")

        loop = asyncio.get_event_loop()

        def _zip_work():
            with zipfile.ZipFile(str(backup_path), "w", compression) as zf:
                if full_backup:
                    for item in world_path.iterdir():
                        if item.is_dir():
                            for f in item.rglob("*"):
                                if f.is_file():
                                    arcname = f.relative_to(world_path.parent)
                                    zf.write(str(f), str(arcname))
                        else:
                            arcname = item.relative_to(world_path.parent)
                            zf.write(str(item), str(arcname))
                else:
                    items = include_items or []
                    for item_name in items:
                        item_path = world_path / item_name
                        if item_path.exists():
                            if item_path.is_dir():
                                for f in item_path.rglob("*"):
                                    if f.is_file():
                                        arcname = f.relative_to(world_path.parent)
                                        zf.write(str(f), str(arcname))
                            else:
                                arcname = item_path.relative_to(world_path.parent)
                                zf.write(str(item_path), str(arcname))

        await loop.run_in_executor(None, _zip_work)

        size = backup_path.stat().st_size
        msg = f"Backup created: {backup_name} ({size / 1024:.1f} KB)"
        if progress:
            progress.put_nowait(msg)
        return msg

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
