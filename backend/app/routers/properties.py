from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import PlainTextResponse

from backend.app.core.security import verify_token
from backend.app.models.user import User
from backend.app.services.properties_service import PropertiesService

router = APIRouter(prefix="/properties", tags=["properties"])

_service = PropertiesService()


@router.get("/")
async def get_properties(_user: User = Depends(verify_token)) -> list[dict]:
    return [e.model_dump() for e in _service.load()]


@router.get("/raw")
async def get_properties_raw(_user: User = Depends(verify_token)) -> PlainTextResponse:
    return PlainTextResponse(_service.get_raw())


@router.put("/raw")
async def save_properties_raw(body: dict, _user: User = Depends(verify_token)):
    text = body.get("text", "")
    _service.save_raw(text)
    return {"success": True, "message": "Properties saved"}


@router.put("/{key}")
async def update_property(key: str, body: dict, _user: User = Depends(verify_token)):
    value = body.get("value", "")
    entry = _service.update(key, value)
    if entry is None:
        raise HTTPException(status_code=404, detail=f"Property '{key}' not found")
    _service.save()
    return {"success": True, "message": f"'{key}' updated to '{value}'"}
