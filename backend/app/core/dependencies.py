from __future__ import annotations

from backend.app.managers.backup_scheduler import BackupScheduler
from backend.app.services.backup_service import BackupService
from backend.app.services.backup_settings_service import BackupSettingsService
from backend.app.services.server_manager import ServerManager

server_manager = ServerManager()
backup_service = BackupService()
backup_settings_service = BackupSettingsService()
backup_scheduler = BackupScheduler(backup_service, backup_settings_service)
