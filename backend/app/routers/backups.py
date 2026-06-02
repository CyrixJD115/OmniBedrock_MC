from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse

from backend.app.core.security import verify_token
from backend.app.schemas.backup import BackupCreateRequest, BackupScheduleConfig, BackupListResponse
from backend.app.services.backup_service import BackupService
from backend.app.managers.backup_scheduler import BackupScheduler

router = APIRouter(prefix="/backups", tags=["backups"])

_service = BackupService()
_scheduler = BackupScheduler()


@router.get("/worlds")
async def list_worlds(auth: str = Depends(verify_token)) -> list[str]:
    return _service.list_worlds()


@router.get("/")
async def list_backups(world: str | None = None, auth: str = Depends(verify_token)) -> BackupListResponse:
    backups = _service.list_backups(world)
    return BackupListResponse(backups=backups, total=len(backups))


@router.post("/create")
async def create_backup(req: BackupCreateRequest, auth: str = Depends(verify_token)) -> dict:
    msg = await _service.create_backup(
        world=req.world,
        tag=req.tag,
        full_backup=req.full_backup,
        compress=req.compress,
        include_items=req.include_items,
    )
    return {"success": True, "message": msg}


@router.delete("/{world}/{filename}")
async def delete_backup(world: str, filename: str, auth: str = Depends(verify_token)) -> dict:
    deleted = _service.delete_backup(world, filename)
    if not deleted:
        raise HTTPException(status_code=404, detail="Backup not found")
    return {"success": True, "message": f"Deleted {filename}"}


@router.get("/{world}/{filename}/download")
async def download_backup(world: str, filename: str, auth: str = Depends(verify_token)):
    path = _service.get_backup_path(world, filename)
    if not path:
        raise HTTPException(status_code=404, detail="Backup not found")
    return FileResponse(str(path), filename=filename)


@router.get("/scheduler")
async def get_scheduler_config(auth: str = Depends(verify_token)) -> dict:
    return {
        "enabled": _scheduler.enabled,
    }


@router.put("/scheduler")
async def update_scheduler_config(cfg: BackupScheduleConfig, auth: str = Depends(verify_token)) -> dict:
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
