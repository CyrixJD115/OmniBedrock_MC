from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException, status
from pydantic import BaseModel

from backend.app.core.auth import (
    authenticate_user,
    create_access_token,
    create_user,
    delete_user,
    list_users,
    update_user,
)
from backend.app.core.security import require_role, verify_token
from backend.app.models.user import User, UserRole

router = APIRouter(prefix="/auth", tags=["auth"])


class LoginRequest(BaseModel):
    username: str
    password: str


class LoginResponse(BaseModel):
    token: str
    user: dict


class CreateUserRequest(BaseModel):
    username: str
    password: str
    role: UserRole = UserRole.viewer
    display_name: str = ""


class UpdateUserRequest(BaseModel):
    role: UserRole | None = None
    display_name: str | None = None
    password: str | None = None


@router.post("/login")
async def login(req: LoginRequest) -> LoginResponse:
    user = authenticate_user(req.username, req.password)
    if not user:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Invalid username or password")
    token = create_access_token(user)
    return LoginResponse(token=token, user=user.to_safe_dict())


@router.get("/me")
async def get_me(user: User = Depends(verify_token)) -> dict:
    return user.to_safe_dict()


@router.get("/users")
async def get_users(current_user: User = Depends(require_role(UserRole.owner, UserRole.admin))) -> list[dict]:
    return [u.to_safe_dict() for u in list_users()]


@router.post("/users")
async def create_user_endpoint(
    req: CreateUserRequest,
    current_user: User = Depends(require_role(UserRole.owner)),
) -> dict:
    user = create_user(req.username, req.password, req.role, req.display_name)
    if not user:
        raise HTTPException(status_code=status.HTTP_409_CONFLICT, detail="Username already exists")
    return user.to_safe_dict()


@router.put("/users/{username}")
async def update_user_endpoint(
    username: str,
    req: UpdateUserRequest,
    current_user: User = Depends(require_role(UserRole.owner)),
) -> dict:
    user = update_user(username, role=req.role, display_name=req.display_name, password=req.password)
    if not user:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="User not found")
    return user.to_safe_dict()


@router.delete("/users/{username}")
async def delete_user_endpoint(
    username: str,
    current_user: User = Depends(require_role(UserRole.owner)),
) -> dict:
    if username == current_user.username:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Cannot delete yourself")
    if not delete_user(username):
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="User not found")
    return {"success": True, "message": f"Deleted user {username}"}
