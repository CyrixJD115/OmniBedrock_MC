from __future__ import annotations

import asyncio
import logging
import time

from backend.app.models.server import ServerStatus

logger = logging.getLogger("performance_collector")


class PerformanceCollector:
    def __init__(self) -> None:
        self._listeners: list[asyncio.Queue[dict]] = []
        self._task: asyncio.Task | None = None
        self._server_manager = None

    def set_server_manager(self, manager) -> None:
        self._server_manager = manager

    def subscribe(self) -> asyncio.Queue[dict]:
        q: asyncio.Queue[dict] = asyncio.Queue()
        self._listeners.append(q)
        return q

    def unsubscribe(self, q: asyncio.Queue) -> None:
        if q in self._listeners:
            self._listeners.remove(q)

    async def start(self) -> None:
        if self._task and not self._task.done():
            return
        self._task = asyncio.create_task(self._collect_loop())
        logger.info("Performance collector started")

    async def stop(self) -> None:
        if self._task and not self._task.done():
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass

    async def _collect_loop(self) -> None:
        while True:
            try:
                await asyncio.sleep(2)
                if not self._listeners:
                    continue

                metrics = self._collect()
                for q in self._listeners:
                    try:
                        q.put_nowait(metrics)
                    except asyncio.QueueFull:
                        # drain oldest
                        try:
                            q.get_nowait()
                            q.put_nowait(metrics)
                        except asyncio.QueueEmpty:
                            pass
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.debug("Metrics collection error: %s", e)

    def _collect(self) -> dict:
        import psutil

        status = self._server_manager.status if self._server_manager else ServerStatus.stopped
        pid = self._server_manager.pid if self._server_manager else None

        cpu = 0.0
        mem = 0.0
        mem_percent = 0.0

        if pid:
            try:
                proc = psutil.Process(pid)
                cpu = proc.cpu_percent(interval=0)
                mem_info = proc.memory_info()
                mem = mem_info.rss / (1024 * 1024)
                mem_percent = proc.memory_percent()
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                pass

        return {
            "timestamp": time.time(),
            "status": status.value,
            "cpu_percent": round(cpu, 1),
            "memory_mb": round(mem, 1),
            "memory_percent": round(mem_percent, 1),
            "tps": self._estimate_tps(),
        }

    def _estimate_tps(self) -> float:
        return 20.0 if self._server_manager and self._server_manager.status == ServerStatus.running else 0.0
