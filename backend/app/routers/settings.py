from __future__ import annotations

from fastapi import APIRouter, Depends
from pydantic import BaseModel

from backend.app.core.config import settings as app_settings
from backend.app.core.dependencies import server_manager
from backend.app.core.permissions import SETTINGS_EDIT
from backend.app.core.security import require_permission, verify_token
from backend.app.models.user import User
from backend.app.services.audit_service import log_action

router = APIRouter(prefix="/settings", tags=["settings"])


class AppSettingsResponse(BaseModel):
    app_name: str
    debug: bool
    bedrock_server_dir: str
    backups_dir: str
    data_dir: str
    logs_dir: str


class ServerSettingsRequest(BaseModel):
    auto_restart: bool | None = None
    auto_restart_delay: int | None = None
    max_crashes: int | None = None
    stop_timeout: int | None = None
    kill_timeout: int | None = None


class ServerSettingsResponse(BaseModel):
    auto_restart: bool
    auto_restart_delay: int
    max_crashes: int
    stop_timeout: int
    kill_timeout: int
    crash_count: int


@router.get("/")
async def get_settings(_user: User = Depends(verify_token)) -> AppSettingsResponse:
    return AppSettingsResponse(
        app_name=app_settings.app_name,
        debug=app_settings.debug,
        bedrock_server_dir=app_settings.bedrock_server_dir,
        backups_dir=app_settings.backups_dir,
        data_dir=app_settings.data_dir,
        logs_dir=app_settings.logs_dir,
    )


@router.get("/server")
async def get_server_settings(_user: User = Depends(verify_token)) -> ServerSettingsResponse:
    return ServerSettingsResponse(
        auto_restart=server_manager.auto_restart,
        auto_restart_delay=server_manager._auto_restart_delay,
        max_crashes=server_manager._max_crashes,
        stop_timeout=server_manager._stop_timeout,
        kill_timeout=server_manager._kill_timeout,
        crash_count=server_manager.crash_count,
    )


@router.put("/server")
async def update_server_settings(
    req: ServerSettingsRequest,
    user: User = Depends(require_permission(SETTINGS_EDIT)),
) -> dict:
    if req.auto_restart is not None or req.auto_restart_delay is not None or req.max_crashes is not None:
        server_manager.set_auto_restart(
            enabled=req.auto_restart if req.auto_restart is not None else server_manager.auto_restart,
            delay=req.auto_restart_delay,
            max_crashes=req.max_crashes,
        )
    if req.stop_timeout is not None or req.kill_timeout is not None:
        server_manager.set_grace_periods(
            stop_timeout=req.stop_timeout if req.stop_timeout is not None else server_manager._stop_timeout,
            kill_timeout=req.kill_timeout if req.kill_timeout is not None else server_manager._kill_timeout,
        )
    log_action(user.username, "settings.update", category="settings")
    return {"success": True, "message": "Server settings updated"}
