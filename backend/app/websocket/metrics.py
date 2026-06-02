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

        async def ping():
            try:
                while True:
                    await asyncio.sleep(30)
                    try:
                        await ws.send_text(json.dumps({"type": "ping"}))
                    except Exception:
                        break
            except Exception:
                pass

        async def writer():
            try:
                while True:
                    data = await metrics_q.get()
                    await ws.send_text(json.dumps({"type": "metrics", "data": data}))
            except Exception:
                pass
            finally:
                self._collector.unsubscribe(metrics_q)
                self._connections.discard(ws)

        try:
            await asyncio.gather(writer(), ping())
        except Exception:
            pass
        finally:
            self._connections.discard(ws)
