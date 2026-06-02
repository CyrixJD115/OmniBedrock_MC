from __future__ import annotations

from pydantic import BaseModel


class BackupCreateRequest(BaseModel):
    world: str
    tag: str = ""
    full_backup: bool = True
    compress: bool = True
    include_items: list[str] | None = None


class BackupScheduleConfig(BaseModel):
    enabled: bool = False
    interval_minutes: int = 30
    keep_count: int = 10
    worlds: list[str] | None = None


class BackupListResponse(BaseModel):
    backups: list[dict]
    total: int
