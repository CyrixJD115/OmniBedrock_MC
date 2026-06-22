from __future__ import annotations

from fastapi import APIRouter, Depends

from backend.app.core.dependencies import server_manager
from backend.app.core.permissions import SERVER_MANAGE
from backend.app.core.security import require_permission, verify_token
from backend.app.models.user import User
from backend.app.schemas.server import ServerActionRequest, ServerActionResponse, ServerStatusResponse
from backend.app.services.audit_service import log_action

router = APIRouter(prefix="/server", tags=["server"])


@router.get("/status")
async def get_status(_user: User = Depends(verify_token)) -> ServerStatusResponse:
    return ServerStatusResponse(**server_manager.get_status_dict())


@router.post("/action")
async def server_action(
    req: ServerActionRequest, _user: User = Depends(require_permission(SERVER_MANAGE))
) -> ServerActionResponse:
    actions = {
        "start": server_manager.start,
        "stop": server_manager.stop,
        "restart": server_manager.restart,
        "kill": server_manager.kill,
    }
    action_fn = actions.get(req.action)
    if not action_fn:
        return ServerActionResponse(success=False, message=f"Unknown action: {req.action}")
    msg = await action_fn()
    log_action(_user.username, f"server:{req.action}", detail=msg, category="server")
    return ServerActionResponse(success=True, message=msg)
