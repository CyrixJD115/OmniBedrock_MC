from __future__ import annotations

import asyncio
import json
import logging
from pathlib import Path

import aiofiles
from fastapi import WebSocket

from backend.app.core.config import settings

logger = logging.getLogger("ws_logs")


class LogStreamWebSocket:
    def __init__(self) -> None:
        self._connections: set[WebSocket] = set()

    async def handle(self, ws: WebSocket) -> None:
        await ws.accept()
        self._connections.add(ws)
        logger.info("Log stream WS connected (%d total)", len(self._connections))

        log_file = Path(settings.logs_dir) / "omnibedrock.log"

        async def follow():
            try:
                if not log_file.exists():
                    log_file.parent.mkdir(parents=True, exist_ok=True)
                    log_file.write_text("")
                async with aiofiles.open(str(log_file)) as f:
                    await f.seek(0, 2)
                    while True:
                        line = await f.readline()
                        if line:
                            await ws.send_text(json.dumps({"type": "log", "line": line.rstrip()}))
                        else:
                            await asyncio.sleep(0.1)
            except Exception:
                pass
            finally:
                self._connections.discard(ws)

        try:
            await asyncio.gather(follow(), self._ping_loop(ws))
        except Exception:
            pass
        finally:
            self._connections.discard(ws)

    async def _ping_loop(self, ws: WebSocket) -> None:
        try:
            while True:
                await asyncio.sleep(30)
                try:
                    await ws.send_text(json.dumps({"type": "ping"}))
                except Exception:
                    break
        except Exception:
            pass
