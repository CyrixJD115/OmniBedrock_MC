from __future__ import annotations

import asyncio
import json
import logging
import time
from typing import Any

from fastapi import WebSocket

from backend.app.core.config import settings
from backend.app.services.server_manager import ServerManager

logger = logging.getLogger("ws_console")


class ConsoleWebSocket:
    def __init__(self, server_manager: ServerManager) -> None:
        self._manager = server_manager
        self._connections: set[WebSocket] = set()

    async def handle(self, ws: WebSocket) -> None:
        await ws.accept()
        self._connections.add(ws)
        logger.info("Console WS connected (%d total)", len(self._connections))

        stdout_q = self._manager.subscribe_stdout()
        status_q = self._manager.subscribe_status()

        async def reader():
            try:
                while True:
                    data = await ws.receive_text()
                    msg = json.loads(data)
                    cmd = msg.get("command", "")
                    if cmd:
                        await self._manager.send_command(cmd)
            except Exception:
                pass
            finally:
                self._manager.unsubscribe_stdout(stdout_q)
                self._manager.unsubscribe_status(status_q)
                self._connections.discard(ws)
                logger.info("Console WS disconnected (%d remaining)", len(self._connections))

        async def writer():
            try:
                while True:
                    line = await asyncio.wait_for(stdout_q.get(), timeout=30)
                    await ws.send_text(json.dumps({"type": "console", "line": line, "timestamp": time.time()}))
            except asyncio.TimeoutError:
                try:
                    await ws.send_text(json.dumps({"type": "ping"}))
                except Exception:
                    pass
            except Exception:
                pass

        async def status_writer():
            try:
                while True:
                    status = await status_q.get()
                    await ws.send_text(json.dumps({"type": "status", "status": status.value}))
            except Exception:
                pass

        try:
            await asyncio.gather(reader(), writer(), status_writer())
        except Exception:
            pass
        finally:
            self._connections.discard(ws)
