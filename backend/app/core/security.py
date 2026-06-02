from __future__ import annotations

from fastapi import Depends, HTTPException, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

from backend.app.core.auth import decode_access_token, get_user
from backend.app.models.user import User, UserRole

_bearer = HTTPBearer(auto_error=True)


def verify_token(credentials: HTTPAuthorizationCredentials = Depends(_bearer)) -> User:
    payload = decode_access_token(credentials.credentials)
    if not payload:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Invalid or expired token")
    username = payload.get("sub")
    if not username:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Invalid token payload")
    user = get_user(username)
    if not user:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="User not found")
    return user


def require_role(*roles: UserRole):
    async def _check(user: User = Depends(verify_token)) -> User:
        role_hierarchy = {
            UserRole.owner: 4,
            UserRole.admin: 3,
            UserRole.moderator: 2,
            UserRole.viewer: 1,
        }
        user_level = role_hierarchy.get(user.role, 0)
        for allowed in roles:
            if user_level >= role_hierarchy.get(allowed, 0):
                return user
        raise HTTPException(status_code=status.HTTP_403_FORBIDDEN, detail="Insufficient permissions")
    return _check
