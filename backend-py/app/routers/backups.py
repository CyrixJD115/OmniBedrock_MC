from __future__ import annotations

import asyncio
import logging
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse

from backend.app.core.config import settings as cfg
from backend.app.core.dependencies import backup_scheduler, backup_service, backup_settings_service, server_manager
from backend.app.core.permissions import BACKUPS_CREATE, BACKUPS_DELETE, BACKUPS_RESTORE, BACKUPS_VIEW, SETTINGS_EDIT
from backend.app.core.security import require_permission, verify_token, verify_token_query
from backend.app.models.server import ServerStatus
from backend.app.models.user import User
from backend.app.schemas.backup import (
    BackupCreateRequest,
    BackupListResponse,
    BackupScheduleConfig,
    BackupSettingsUpdate,
    TestCommandRequest,
)
from backend.app.services.audit_service import log_action
from backend.app.services.backup_service import BackupAlreadyRunning, _send_to_server, validate_pre_post_commands
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


@router.get("/worlds")
async def list_worlds(_u: User = Depends(verify_token)) -> list[str]:
    return backup_service.list_worlds()


@router.get("/")
async def list_backups(world: str | None = None, _u: User = Depends(verify_token)) -> BackupListResponse:
    backups = backup_service.list_backups(world)
    return BackupListResponse(backups=backups, total=len(backups))


@router.post("/create")
async def create_backup(req: BackupCreateRequest, user: User = Depends(require_permission(BACKUPS_CREATE))) -> dict:
    pre_post = backup_settings_service.load()["pre_post"] if req.run_hooks else {"before": [], "after": []}

    if req.run_hooks:
        valid, msg = validate_pre_post_commands(pre_post)
        if not valid:
            raise HTTPException(status_code=400, detail=msg)

    async def _discard(event: dict) -> None:
        pass

    try:
        result = await backup_service.run_backup(
            req.world,
            req.tag,
            full_backup=req.full_backup,
            zip_prefix=req.zip_prefix,
            export_folder=req.export_folder,
            compression=req.compression,
            include_items=req.include_items,
            pre_post=pre_post,
            dry_run=req.dry_run,
            notify=_discard,
            run_hooks=req.run_hooks,
        )
    except BackupAlreadyRunning:
        raise HTTPException(status_code=409, detail="A backup job is already running")
    log_action(user.username, "backup.run", result.get("filename") or req.world, category="backup")
    return result


@router.post("/restore/{world}/{filename}")
async def restore_backup(world: str, filename: str, _u: User = Depends(require_permission(BACKUPS_RESTORE))) -> dict:
    if not backup_service.restore_backup(world, filename):
        raise HTTPException(status_code=404, detail="Backup not found in trash")
    return {"success": True, "message": f"Restored {filename}"}


@router.post("/{world}/{filename}/restore-to-world")
async def restore_backup_to_world(
    world: str, filename: str, user: User = Depends(require_permission(BACKUPS_RESTORE)),
) -> dict:
    path = backup_service.get_backup_path(world, filename)
    if not path:
        raise HTTPException(status_code=404, detail="Backup not found")

    if server_manager.status == ServerStatus.running:
        logger = logging.getLogger("backup")
        logger.info("Stopping server before restoring world '%s' from backup '%s'", world, filename)
        await server_manager.stop()

    try:
        await backup_service.restore_to_world(world, filename)
    except FileNotFoundError as e:
        raise HTTPException(status_code=404, detail=str(e))

    log_action(user.username, "backup.restore_to_world", f"{world}/{filename}", category="backup")
    return {
        "success": True,
        "message": f"World '{world}' restored from '{filename}'. Server was stopped — start it manually when ready.",
    }


@router.get("/trash")
async def list_trash(world: str | None = None, _u: User = Depends(verify_token)) -> list[dict]:
    return backup_service.list_trash(world)


@router.delete("/{world}/{filename}")
async def delete_backup(world: str, filename: str, _u: User = Depends(require_permission(BACKUPS_DELETE))) -> dict:
    if not backup_service.delete_backup(world, filename):
        raise HTTPException(status_code=404, detail="Backup not found")
    return {"success": True, "message": f"Moved {filename} to trash"}


@router.get("/{world}/{filename}/download")
async def download_backup(world: str, filename: str, _u: User = Depends(verify_token_query)):
    path = backup_service.get_backup_path(world, filename)
    if not path:
        raise HTTPException(status_code=404, detail="Backup not found")
    return FileResponse(str(path), filename=filename)


@router.get("/settings")
async def get_settings(_u: User = Depends(verify_token)) -> dict:
    return backup_settings_service.load()


@router.put("/settings")
async def put_settings(
    payload: BackupSettingsUpdate,
    user: User = Depends(require_permission(SETTINGS_EDIT)),
) -> dict:
    backup_settings_service.save(manual=payload.manual, auto=payload.auto, pre_post=payload.pre_post)
    log_action(user.username, "backup.settings_update", "backup_settings.yaml", category="backup")
    return {"success": True}


@router.post("/test-command")
async def test_command(
    payload: TestCommandRequest,
    user: User = Depends(require_permission(BACKUPS_CREATE)),
) -> dict:
    entry = payload.entry
    log_action(user.username, "backup.test_command", f"{entry.type}:{entry.value}", category="backup")
    if entry.type == "send":
        lines: list[str] = []

        async def _collect(e: dict) -> None:
            if e.get("stream") == "server" and e.get("line"):
                lines.append(e["line"])

        await _send_to_server(str(entry.value), notify=_collect)
        output = f"Sent to console: {entry.value}"
        if lines:
            output += "\n" + "\n".join(lines)
        return {"kind": "send", "output": output, "exit_code": 0}
    if entry.type == "comment":
        return {"kind": "comment", "output": "Comment (ignored)", "exit_code": 0}
    if entry.type == "wait":
        return {"kind": "wait", "output": f"Wait directive: {entry.value}s (not executed in test)", "exit_code": 0}
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
async def list_folders(
    base: str = "",
    _u: User = Depends(require_permission(BACKUPS_VIEW)),
) -> dict:
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
async def update_scheduler_config(
    cfg_req: BackupScheduleConfig,
    user: User = Depends(require_permission(SETTINGS_EDIT)),
) -> dict:
    backup_settings_service.save(auto=cfg_req.model_dump(exclude_none=True))
    backup_scheduler.configure(
        enabled=cfg_req.enabled,
        interval_minutes=cfg_req.interval_minutes,
        keep_count=cfg_req.keep_count,
        worlds=cfg_req.worlds or [],
    )
    if cfg_req.enabled:
        await backup_scheduler.start()
    else:
        await backup_scheduler.stop()
    log_action(user.username, "backup.scheduler_update", f"enabled={cfg_req.enabled}", category="backup")
    return {"success": True}


@router.websocket("/ws")
async def backup_websocket(ws: WebSocket):
    try:
        await get_ws_handler().handle(ws)
    except WebSocketDisconnect:
        pass
