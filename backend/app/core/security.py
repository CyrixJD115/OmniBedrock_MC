from __future__ import annotations

import hashlib
import hmac
import secrets
from datetime import datetime, timedelta, timezone

from fastapi import Depends, HTTPException, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

from backend.app.core.config import settings

_bearer = HTTPBearer(auto_error=False)
_tokens: set[str] = set()


def generate_token() -> str:
    token = secrets.token_hex(32)
    _tokens.add(token)
    return token


def validate_token(token: str) -> bool:
    return token in _tokens


def verify_token(credentials: HTTPAuthorizationCredentials | None = Depends(_bearer)) -> str | None:
    if settings.debug and credentials is None:
        return "debug-token"
    if credentials is None:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Missing authorization header")
    token = credentials.credentials
    if not validate_token(token):
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Invalid or expired token")
    return token
