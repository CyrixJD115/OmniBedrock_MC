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
