from __future__ import annotations

from fastapi import APIRouter, Depends

from backend.app.core.dependencies import server_manager
from backend.app.core.security import verify_token
from backend.app.schemas.server import ServerActionRequest, ServerActionResponse, ServerStatusResponse

router = APIRouter(prefix="/server", tags=["server"])


@router.get("/status")
async def get_status(auth: str = Depends(verify_token)) -> ServerStatusResponse:
    return ServerStatusResponse(**server_manager.get_status_dict())


@router.post("/action")
async def server_action(req: ServerActionRequest, auth: str = Depends(verify_token)) -> ServerActionResponse:
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
    return ServerActionResponse(success=True, message=msg)
