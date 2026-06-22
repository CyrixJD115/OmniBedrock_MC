from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException

from backend.app.core.security import verify_token
from backend.app.models.user import User
from backend.app.schemas.addon import AddonReorderRequest, ManifestUpdateRequest
from backend.app.services.addon_service import AddonService
from backend.app.services.audit_service import log_action

router = APIRouter(prefix="/addons", tags=["addons"])

_service = AddonService()


@router.get("/")
async def list_addons(_user: User = Depends(verify_token)) -> dict:
    return _service.list_addons()


@router.get("/manifest")
async def get_manifest(path: str, _user: User = Depends(verify_token)) -> dict:
    manifest = _service.get_manifest(path)
    if manifest is None:
        raise HTTPException(status_code=404, detail="Manifest not found")
    return manifest


@router.put("/manifest")
async def update_manifest(req: ManifestUpdateRequest, user: User = Depends(verify_token)) -> dict:
    ok = _service.update_manifest(req.path, req.manifest)
    if not ok:
        raise HTTPException(status_code=500, detail="Failed to update manifest")
    log_action(user.username, "addons.manifest_update", req.path, category="addons")
    return {"success": True, "message": "Manifest updated"}


@router.get("/order/{world}/{pack_type}")
async def get_pack_order(world: str, pack_type: str, _user: User = Depends(verify_token)) -> list[dict]:
    return _service.get_pack_order(world, pack_type)


@router.put("/order/{world}/{pack_type}")
async def set_pack_order(
    world: str, pack_type: str, req: AddonReorderRequest, user: User = Depends(verify_token)
) -> dict:
    ok = _service.set_pack_order(world, pack_type, req.uuids)
    if not ok:
        raise HTTPException(status_code=500, detail="Failed to update pack order")
    log_action(user.username, "addons.reorder", f"{world}/{pack_type}", category="addons")
    return {"success": True, "message": "Pack order updated"}
