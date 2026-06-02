from __future__ import annotations

import asyncio
import json
import logging

from backend.app.core.config import settings
from backend.app.services.backup_service import BackupService

logger = logging.getLogger("backup_scheduler")


class BackupScheduler:
    def __init__(self) -> None:
        self._task: asyncio.Task | None = None
        self._enabled = False
        self._interval = 30
        self._keep = 10
        self._worlds: list[str] = []
        self._backup_service = BackupService()

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
        self._task = asyncio.create_task(self._run())
        logger.info("Backup scheduler started (interval=%d min, keep=%d)", self._interval, self._keep)

    async def stop(self) -> None:
        if self._task and not self._task.done():
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
        self._enabled = False
        logger.info("Backup scheduler stopped")

    async def _run(self) -> None:
        while self._enabled:
            try:
                await asyncio.sleep(self._interval * 60)
                if not self._enabled:
                    break
                worlds = self._worlds or self._backup_service.list_worlds()
                for world in worlds:
                    await self._backup_service.create_backup(world, tag="auto")
                self._cleanup_old()
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error("Auto-backup error: %s", e)

    def _cleanup_old(self) -> None:
        try:
            all_backups = self._backup_service.list_backups()
            backups_by_world: dict[str, list[dict]] = {}
            for b in all_backups:
                backups_by_world.setdefault(b["world"], []).append(b)
            for world, backups in backups_by_world.items():
                if len(backups) > self._keep:
                    to_delete = backups[self._keep:]
                    for b in to_delete:
                        self._backup_service.delete_backup(b["world"], b["filename"])
                        logger.info("Deleted old backup: %s/%s", b["world"], b["filename"])
        except Exception as e:
            logger.error("Cleanup error: %s", e)
