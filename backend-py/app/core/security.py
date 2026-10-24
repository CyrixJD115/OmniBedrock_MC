from __future__ import annotations

from fastapi import Depends, Header, HTTPException, Query, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

from backend.app.core.auth import decode_access_token, get_user, get_user_from_token
from backend.app.core.roles import resolve_permissions
from backend.app.models.user import User

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


async def verify_token_query(
    authorization: str | None = Header(None),
    token: str | None = Query(None),
) -> User:
    """Verify JWT from either the Authorization header or a query parameter.

    This is used for endpoints that need to support direct browser access
    (e.g. file downloads via <a> tags that cannot set custom headers).
    """
    token_str = token
    if not token_str and authorization:
        parts = authorization.split()
        if len(parts) == 2 and parts[0].lower() == "bearer":
            token_str = parts[1]

    if not token_str:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Not authenticated")

    user = get_user_from_token(token_str)
    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid or expired token",
        )
    return user


def require_permission(*permissions: str):
    async def _check(user: User = Depends(verify_token)) -> User:
        user_perms = resolve_permissions(user.role)
        for perm in permissions:
            if perm not in user_perms:
                raise HTTPException(status_code=status.HTTP_403_FORBIDDEN, detail=f"Missing permission: {perm}")
        return user
    return _check
