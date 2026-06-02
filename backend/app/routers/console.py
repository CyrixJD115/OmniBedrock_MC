from __future__ import annotations

from fastapi import APIRouter, Depends, WebSocket, WebSocketDisconnect

from backend.app.core.dependencies import server_manager
from backend.app.core.security import verify_token
from backend.app.models.user import User
from backend.app.schemas.console import ConsoleCommandRequest, ConsoleCommandResponse
from backend.app.websocket.console import ConsoleWebSocket

router = APIRouter(prefix="/console", tags=["console"])

_ws_handler: ConsoleWebSocket | None = None


def get_ws_handler() -> ConsoleWebSocket:
    global _ws_handler
    if _ws_handler is None:
        _ws_handler = ConsoleWebSocket(server_manager)
    return _ws_handler


@router.post("/command")
async def send_command(req: ConsoleCommandRequest, _user: User = Depends(verify_token)) -> ConsoleCommandResponse:
    await server_manager.send_command(req.command)
    return ConsoleCommandResponse(success=True, message="")


@router.websocket("/ws")
async def console_websocket(ws: WebSocket):
    handler = get_ws_handler()
    try:
        await handler.handle(ws)
    except WebSocketDisconnect:
        pass
