from __future__ import annotations

from fastapi import APIRouter, Depends
from pydantic import BaseModel

from backend.app.core.security import verify_token
from backend.app.core.config import settings

router = APIRouter(prefix="/settings", tags=["settings"])


class SettingsResponse(BaseModel):
    app_name: str
    debug: bool
    bedrock_server_dir: str
    backups_dir: str
    ini_dir: str
    logs_dir: str


@router.get("/")
async def get_settings(auth: str = Depends(verify_token)) -> SettingsResponse:
    return SettingsResponse(
        app_name=settings.app_name,
        debug=settings.debug,
        bedrock_server_dir=settings.bedrock_server_dir,
        backups_dir=settings.backups_dir,
        ini_dir=settings.ini_dir,
        logs_dir=settings.logs_dir,
    )
