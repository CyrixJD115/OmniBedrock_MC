from __future__ import annotations

import json
import logging

from fastapi import WebSocket, WebSocketDisconnect

from backend.app.core.auth import get_user_from_token
from backend.app.services.backup_service import BackupService

logger = logging.getLogger("ws_backup")


class BackupWebSocket:
    def __init__(self, backup_service: BackupService) -> None:
        self._service = backup_service
        self._connections: set[WebSocket] = set()

    async def handle(self, ws: WebSocket) -> None:
        token = ws.query_params.get("token", "")
        user = get_user_from_token(token) if token else None
        if user is None:
            await ws.close(code=1008)
            return
        await ws.accept()
        self._connections.add(ws)
        self._service._subscribers.add(self._broadcast)
        logger.info("Backup WS connected (%d total)", len(self._connections))
        try:
            await ws.send_text(json.dumps({"type": "hello", "active": self._service._active}))
            while True:
                await ws.receive_text()
        except WebSocketDisconnect:
            pass
        finally:
            self._connections.discard(ws)
            self._service._subscribers.discard(self._broadcast)
            logger.info("Backup WS disconnected (%d remaining)", len(self._connections))

    async def _broadcast(self, event: dict) -> None:
        dead: list[WebSocket] = []
        payload = json.dumps(event)
        for ws in list(self._connections):
            try:
                await ws.send_text(payload)
            except Exception:
                dead.append(ws)
        for ws in dead:
            self._connections.discard(ws)
