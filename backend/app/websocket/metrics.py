from __future__ import annotations

import asyncio
import json
import logging

from fastapi import WebSocket

from backend.app.managers.performance_collector import PerformanceCollector

logger = logging.getLogger("ws_metrics")


class MetricsWebSocket:
    def __init__(self, collector: PerformanceCollector) -> None:
        self._collector = collector
        self._connections: set[WebSocket] = set()

    async def handle(self, ws: WebSocket) -> None:
        await ws.accept()
        self._connections.add(ws)
        logger.info("Metrics WS connected (%d total)", len(self._connections))

        metrics_q = self._collector.subscribe()
        stop_event = asyncio.Event()

        async def writer():
            try:
                while not stop_event.is_set():
                    data = await metrics_q.get()
                    await ws.send_text(json.dumps({"type": "metrics", "data": data}))
            except Exception:
                pass
            finally:
                stop_event.set()

        async def ping():
            try:
                while not stop_event.is_set():
                    await asyncio.sleep(30)
                    try:
                        await ws.send_text(json.dumps({"type": "ping"}))
                    except Exception:
                        stop_event.set()
            except Exception:
                pass

        tasks = [
            asyncio.create_task(writer()),
            asyncio.create_task(ping()),
        ]

        try:
            done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            stop_event.set()
            for task in pending:
                task.cancel()
            if pending:
                await asyncio.gather(*pending, return_exceptions=True)
        finally:
            self._collector.unsubscribe(metrics_q)
            self._connections.discard(ws)
            logger.info("Metrics WS disconnected (%d remaining)", len(self._connections))
