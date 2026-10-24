from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import PlainTextResponse

from backend.app.core.permissions import PROPERTIES_EDIT
from backend.app.core.security import require_permission, verify_token
from backend.app.models.user import User
from backend.app.services.audit_service import log_action
from backend.app.services.properties_service import PropertiesService
from backend.app.utils.minecraft_helpers import validate_property

router = APIRouter(prefix="/properties", tags=["properties"])

_service = PropertiesService()


@router.get("/")
async def get_properties(_user: User = Depends(verify_token)) -> list[dict]:
    return [e.model_dump() for e in _service.load()]


@router.get("/raw")
async def get_properties_raw(_user: User = Depends(verify_token)) -> PlainTextResponse:
    return PlainTextResponse(_service.get_raw())


@router.put("/raw")
async def save_properties_raw(body: dict, user: User = Depends(require_permission(PROPERTIES_EDIT))):
    text = body.get("text", "")
    _service.save_raw(text)
    log_action(user.username, "properties.raw_update", category="properties")
    return {"success": True, "message": "Properties saved"}


@router.put("/{key}")
async def update_property(key: str, body: dict, user: User = Depends(require_permission(PROPERTIES_EDIT))):
    value = body.get("value", "")
    valid, msg = validate_property(key, value)
    if not valid:
        raise HTTPException(status_code=400, detail=msg)
    entry = _service.update(key, value)
    if entry is None:
        raise HTTPException(status_code=404, detail=f"Property '{key}' not found")
    _service.save()
    log_action(user.username, f"properties.update:{key}", value, category="properties")
    return {"success": True, "message": f"'{key}' updated to '{value}'"}
