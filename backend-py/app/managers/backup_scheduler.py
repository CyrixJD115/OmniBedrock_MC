from __future__ import annotations

import asyncio
import logging

from backend.app.services.backup_service import BackupService, validate_pre_post_commands
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

    def configure(
        self,
        enabled: bool,
        interval_minutes: int = 30,
        keep_count: int = 10,
        worlds: list[str] | None = None,
    ) -> None:
        self._enabled = enabled
        self._interval = interval_minutes
        self._keep = keep_count
        self._worlds = worlds or []

    async def start(self) -> None:
        if not self._enabled:
            return
        if self._task and not self._task.done():
            return
        settings = self._settings_service.load()
        valid, msg = validate_pre_post_commands(settings["pre_post"])
        if not valid:
            logger.warning("Backup scheduler not started: %s", msg)
            self._enabled = False
            return
        self._task = asyncio.create_task(self._run())
        logger.info("Backup scheduler started (interval=%d min, keep=%d)", self._interval, self._keep)

    async def start_if_enabled(self) -> None:
        settings = self._settings_service.load()
        auto = settings.get("auto", {})
        if auto.get("enabled", False):
            self.configure(
                enabled=True,
                interval_minutes=auto.get("interval_minutes", 30),
                keep_count=auto.get("keep_count", 10),
                worlds=auto.get("worlds"),
            )
            await self.start()

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

        valid, msg = validate_pre_post_commands(pre_post)
        if not valid:
            logger.warning("Auto-backup skipped: %s", msg)
            return

        worlds = self._worlds or self._backup_service.list_worlds()
        for world in worlds:
            async def _discard(e: dict) -> None:
                pass
            try:
                await self._backup_service.run_backup(
                    world,
                    tag="auto",
                    full_backup=auto.get("full_backup", True),
                    zip_prefix="auto_backup",
                    export_folder=auto.get("export_folder", ""),
                    compression=auto.get("compression", "deflate"),
                    include_items=auto.get("include_items") or None,
                    pre_post=pre_post,
                    dry_run=False,
                    notify=_discard,
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
