from __future__ import annotations

from fastapi import APIRouter, Depends, WebSocket, WebSocketDisconnect, status

from backend.app.core.auth import get_user_from_token
from backend.app.core.dependencies import server_manager
from backend.app.core.security import verify_token
from backend.app.managers.performance_collector import PerformanceCollector
from backend.app.models.user import User
from backend.app.websocket.metrics import MetricsWebSocket

router = APIRouter(prefix="/performance", tags=["performance"])

_collector = PerformanceCollector()
_ws_handler: MetricsWebSocket | None = None


def get_collector() -> PerformanceCollector:
    global _collector
    _collector.set_server_manager(server_manager)
    return _collector


def get_ws_handler() -> MetricsWebSocket:
    global _ws_handler
    if _ws_handler is None:
        _ws_handler = MetricsWebSocket(get_collector())
    return _ws_handler


@router.get("/metrics")
async def get_metrics(_user: User = Depends(verify_token)) -> dict:
    collector = get_collector()
    # force a single collection
    return collector._collect()


@router.websocket("/ws")
async def metrics_websocket(ws: WebSocket):
    token = ws.query_params.get("token")
    if not token or not get_user_from_token(token):
        await ws.close(code=status.WS_1008_POLICY_VIOLATION)
        return
    handler = get_ws_handler()
    try:
        await handler.handle(ws)
    except WebSocketDisconnect:
        pass
