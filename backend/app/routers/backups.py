from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse

from backend.app.core.security import verify_token
from backend.app.managers.backup_scheduler import BackupScheduler
from backend.app.models.user import User
from backend.app.schemas.backup import BackupCreateRequest, BackupListResponse, BackupScheduleConfig
from backend.app.services.backup_service import BackupService

router = APIRouter(prefix="/backups", tags=["backups"])

_service = BackupService()
_scheduler = BackupScheduler()


@router.get("/worlds")
async def list_worlds(_user: User = Depends(verify_token)) -> list[str]:
    return _service.list_worlds()


@router.get("/")
async def list_backups(world: str | None = None, _user: User = Depends(verify_token)) -> BackupListResponse:
    backups = _service.list_backups(world)
    return BackupListResponse(backups=backups, total=len(backups))


@router.post("/create")
async def create_backup(req: BackupCreateRequest, _user: User = Depends(verify_token)) -> dict:
    msg = await _service.create_backup(
        world=req.world,
        tag=req.tag,
        full_backup=req.full_backup,
        compress=req.compress,
        include_items=req.include_items,
    )
    return {"success": True, "message": msg}


@router.post("/restore/{world}/{filename}")
async def restore_backup(world: str, filename: str, _user: User = Depends(verify_token)) -> dict:
    restored = _service.restore_backup(world, filename)
    if not restored:
        raise HTTPException(status_code=404, detail="Backup not found in trash")
    return {"success": True, "message": f"Restored {filename}"}


@router.get("/trash")
async def list_trash(world: str | None = None, _user: User = Depends(verify_token)) -> list[dict]:
    return _service.list_trash(world)


@router.delete("/{world}/{filename}")
async def delete_backup(world: str, filename: str, _user: User = Depends(verify_token)) -> dict:
    deleted = _service.delete_backup(world, filename)
    if not deleted:
        raise HTTPException(status_code=404, detail="Backup not found")
    return {"success": True, "message": f"Moved {filename} to trash"}


@router.get("/{world}/{filename}/download")
async def download_backup(world: str, filename: str, _user: User = Depends(verify_token)):
    path = _service.get_backup_path(world, filename)
    if not path:
        raise HTTPException(status_code=404, detail="Backup not found")
    return FileResponse(str(path), filename=filename)


@router.get("/scheduler")
async def get_scheduler_config(_user: User = Depends(verify_token)) -> dict:
    return {
        "enabled": _scheduler.enabled,
    }


@router.put("/scheduler")
async def update_scheduler_config(cfg: BackupScheduleConfig, _user: User = Depends(verify_token)) -> dict:
    _scheduler.configure(
        enabled=cfg.enabled,
        interval_minutes=cfg.interval_minutes,
        keep_count=cfg.keep_count,
        worlds=cfg.worlds,
    )
    if cfg.enabled:
        await _scheduler.start()
    else:
        await _scheduler.stop()
    return {"success": True, "message": "Scheduler updated"}
