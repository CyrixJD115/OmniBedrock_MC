from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException, status
from pydantic import BaseModel

from backend.app.core.permissions import USERS_CREATE, USERS_DELETE, USERS_EDIT, USERS_VIEW
from backend.app.core.roles import create_role, delete_role, get_role, list_roles, update_role
from backend.app.core.security import require_permission
from backend.app.models.role import Role
from backend.app.models.user import User

router = APIRouter(prefix="/roles", tags=["roles"])


class CreateRoleRequest(BaseModel):
    name: str
    display_name: str = ""
    permissions: list[str] = []
    is_default: bool = False


class UpdateRoleRequest(BaseModel):
    permissions: list[str] | None = None
    display_name: str | None = None
    is_default: bool | None = None


@router.get("/")
async def get_roles(_user: User = Depends(require_permission(USERS_VIEW))) -> list[dict]:
    return [r.to_dict() for r in list_roles()]


@router.get("/{name}")
async def get_role_by_name(name: str, _user: User = Depends(require_permission(USERS_VIEW))) -> dict:
    role = get_role(name)
    if not role:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Role not found")
    return role.to_dict()


@router.post("/")
async def create_role_endpoint(req: CreateRoleRequest, user: User = Depends(require_permission(USERS_CREATE))) -> dict:
    existing = get_role(req.name)
    if existing:
        raise HTTPException(status_code=status.HTTP_409_CONFLICT, detail="Role already exists")
    role = Role(name=req.name, display_name=req.display_name or req.name, permissions=req.permissions, is_default=req.is_default)
    if not create_role(role):
        raise HTTPException(status_code=status.HTTP_500_INTERNAL_SERVER_ERROR, detail="Failed to create role")
    return role.to_dict()


@router.put("/{name}")
async def update_role_endpoint(
    name: str, req: UpdateRoleRequest,
    user: User = Depends(require_permission(USERS_EDIT)),
) -> dict:
    role = update_role(name, permissions=req.permissions, display_name=req.display_name, is_default=req.is_default)
    if not role:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Role not found")
    return role.to_dict()


@router.delete("/{name}")
async def delete_role_endpoint(name: str, user: User = Depends(require_permission(USERS_DELETE))) -> dict:
    if name == "owner":
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Cannot delete owner role")
    if not delete_role(name):
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Role not found")
    return {"success": True, "message": f"Role {name} deleted"}
