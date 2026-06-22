from __future__ import annotations

from fastapi import APIRouter, Depends, Query

from backend.app.core.permissions import AUDIT_VIEW
from backend.app.core.security import require_permission
from backend.app.models.user import User
from backend.app.services.audit_service import query_audit

router = APIRouter(prefix="/audit", tags=["audit"])


@router.get("/")
async def get_audit_log(
    limit: int = Query(100, ge=1, le=500),
    offset: int = Query(0, ge=0),
    username: str | None = Query(None),
    action: str | None = Query(None),
    category: str | None = Query(None),
    _user: User = Depends(require_permission(AUDIT_VIEW)),
) -> list[dict]:
    return query_audit(limit=limit, offset=offset, username=username, action=action, category=category)
