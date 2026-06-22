from __future__ import annotations

import asyncio
import json
import logging
import time

from fastapi import WebSocket

from backend.app.services.server_manager import ServerManager, detect_level

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
        error_q = self._manager.subscribe_error_stats()
        stop_event = asyncio.Event()

        for line in self._manager.get_history():
            await ws.send_text(json.dumps({"type": "console", "line": line, "level": detect_level(line), "timestamp": time.time()}))

        async def reader():
            try:
                while not stop_event.is_set():
                    data = await ws.receive_text()
                    msg = json.loads(data)
                    cmd = msg.get("command", "")
                    if cmd:
                        await self._manager.send_command(cmd)
            except Exception:
                pass
            finally:
                stop_event.set()

        async def writer():
            try:
                while not stop_event.is_set():
                    try:
                        line = await asyncio.wait_for(stdout_q.get(), timeout=1)
                        await ws.send_text(json.dumps({"type": "console", "line": line, "level": detect_level(line), "timestamp": time.time()}))
                    except asyncio.TimeoutError:
                        try:
                            await ws.send_text(json.dumps({"type": "ping"}))
                        except Exception:
                            stop_event.set()
            except Exception:
                pass

        async def status_writer():
            try:
                while not stop_event.is_set():
                    status = await status_q.get()
                    await ws.send_text(json.dumps({"type": "status", "status": status.value}))
            except Exception:
                pass
            finally:
                stop_event.set()

        async def error_stats_writer():
            try:
                while not stop_event.is_set():
                    stats = await error_q.get()
                    await ws.send_text(json.dumps({"type": "error_stats", "errors": stats}))
            except Exception:
                pass

        tasks = [
            asyncio.create_task(reader()),
            asyncio.create_task(writer()),
            asyncio.create_task(status_writer()),
            asyncio.create_task(error_stats_writer()),
        ]

        try:
            done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            stop_event.set()
            for task in pending:
                task.cancel()
            if pending:
                await asyncio.gather(*pending, return_exceptions=True)
        finally:
            self._manager.unsubscribe_stdout(stdout_q)
            self._manager.unsubscribe_status(status_q)
            self._manager.unsubscribe_error_stats(error_q)
            self._connections.discard(ws)
            logger.info("Console WS disconnected (%d remaining)", len(self._connections))
