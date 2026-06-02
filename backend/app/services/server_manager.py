from __future__ import annotations

import asyncio
import logging
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

from backend.app.core.config import settings
from backend.app.models.server import ServerStatus

logger = logging.getLogger("server_manager")

_TPS_RE = re.compile(r"(?:tps|mspt)[:\s]*([0-9.]+)", re.IGNORECASE)


class ServerManager:
    def __init__(self) -> None:
        self._process: subprocess.Popen | None = None
        self._status: ServerStatus = ServerStatus.stopped
        self._start_time: float = 0.0
        self._server_version: str | None = None
        self._lock = asyncio.Lock()

        self._stdin_queue: asyncio.Queue[str] = asyncio.Queue()
        self._stdout_handler: list[asyncio.Queue[str]] = []
        self._status_listeners: list[asyncio.Queue[ServerStatus]] = []
        self._history: list[str] = []
        self._max_history: int = 2000

        self._reader_task: asyncio.Task | None = None
        self._writer_task: asyncio.Task | None = None
        self._watchdog_task: asyncio.Task | None = None

        self._auto_restart: bool = True
        self._auto_restart_delay: int = 5
        self._max_crashes: int = 3
        self._crash_count: int = 0
        self._stop_timeout: int = 8
        self._kill_timeout: int = 4
        self._was_intentional_stop: bool = False

        self._last_tps: float | None = None

        self._ini_dir = Path(settings.ini_dir)
        self._ini_dir.mkdir(parents=True, exist_ok=True)
        self._lock_file = self._ini_dir / "console_lock_state.ini"

    @property
    def status(self) -> ServerStatus:
        return self._status

    @property
    def pid(self) -> int | None:
        return self._process.pid if self._process and self._process.returncode is None else None

    @property
    def uptime(self) -> float:
        if self._status == ServerStatus.running and self._start_time > 0:
            return time.time() - self._start_time
        return 0.0

    @property
    def server_version(self) -> str | None:
        return self._server_version

    @property
    def last_tps(self) -> float | None:
        return self._last_tps

    @property
    def crash_count(self) -> int:
        return self._crash_count

    @property
    def auto_restart(self) -> bool:
        return self._auto_restart

    def set_auto_restart(self, enabled: bool, delay: int | None = None, max_crashes: int | None = None) -> None:
        self._auto_restart = enabled
        if delay is not None:
            self._auto_restart_delay = max(1, delay)
        if max_crashes is not None:
            self._max_crashes = max(0, max_crashes)
        if not enabled:
            self._crash_count = 0

    def set_grace_periods(self, stop_timeout: int, kill_timeout: int) -> None:
        self._stop_timeout = max(1, stop_timeout)
        self._kill_timeout = max(1, kill_timeout)

    def _resolve_endstone(self) -> list[str]:
        endstone = shutil.which("endstone")
        if endstone:
            return [endstone]
        result = subprocess.run(
            [sys.executable, "-m", "endstone", "--version"],
            capture_output=True, text=True, timeout=10
        )
        if result.returncode == 0:
            self._server_version = result.stdout.strip()
            return [sys.executable, "-m", "endstone"]
        msg = "endstone binary not found and `python -m endstone` failed"
        raise RuntimeError(msg)

    async def start(self) -> str:
        async with self._lock:
            if self._status in (ServerStatus.running, ServerStatus.starting):
                return "Server is already running or starting"

            self._status = ServerStatus.starting
            self._notify_status()

            server_dir = Path(settings.bedrock_server_dir)
            server_dir.mkdir(parents=True, exist_ok=True)

            try:
                endstone_cmd = self._resolve_endstone()
            except RuntimeError as e:
                self._status = ServerStatus.stopped
                self._notify_status()
                return str(e)

            cmd = list(endstone_cmd)
            if "--no-confirm" not in cmd:
                cmd.append("--no-confirm")
            cmd.extend(["-s", str(server_dir)])

            logger.info("Starting server: %s", " ".join(cmd))

            self._process = subprocess.Popen(
                cmd,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )

            self._write_lock_state("locked")
            self._start_time = time.time()

            loop = asyncio.get_event_loop()
            self._reader_task = loop.create_task(self._reader_loop())
            self._writer_task = loop.create_task(self._writer_loop())
            self._watchdog_task = loop.create_task(self._watchdog_loop())

            self._status = ServerStatus.running
            self._notify_status()
            return f"Server started (PID: {self._process.pid})"

    async def stop(self) -> str:
        async with self._lock:
            if self._status != ServerStatus.running:
                return "Server is not running"

            self._status = ServerStatus.stopping
            self._notify_status()
            self._was_intentional_stop = True

            logger.info("Stopping server gracefully...")
            await self._send_command("stop")

            wait_start = time.time()
            while time.time() - wait_start < self._stop_timeout:
                if self._process and self._process.returncode is not None:
                    break
                await asyncio.sleep(0.5)

            if self._process and self._process.returncode is None:
                logger.warning("Server did not stop gracefully, terminating...")
                self._process.terminate()
                try:
                    self._process.wait(timeout=self._kill_timeout)
                except subprocess.TimeoutExpired:
                    logger.error("Server still alive, killing...")
                    self._process.kill()
                    self._process.wait()

            self._cleanup()
            self._status = ServerStatus.stopped
            self._write_lock_state("unlocked")
            self._notify_status()
            return "Server stopped"

    async def kill(self) -> str:
        async with self._lock:
            if not self._process or self._process.returncode is not None:
                return "No running server to kill"

            self._was_intentional_stop = True
            logger.warning("Force killing server...")
            self._process.kill()
            self._process.wait()
            self._cleanup()
            self._status = ServerStatus.stopped
            self._write_lock_state("unlocked")
            self._notify_status()
            return "Server killed"

    async def restart(self) -> str:
        msg = await self.stop()
        if "error" in msg.lower():
            return msg
        await asyncio.sleep(1)
        return await self.start()

    async def send_command(self, command: str) -> None:
        await self._stdin_queue.put(command)

    async def _send_command(self, command: str) -> None:
        if self._process and self._process.stdin and self._process.returncode is None:
            self._process.stdin.write(command + "\n")
            self._process.stdin.flush()

    def subscribe_stdout(self) -> asyncio.Queue[str]:
        q: asyncio.Queue[str] = asyncio.Queue()
        self._stdout_handler.append(q)
        return q

    def unsubscribe_stdout(self, q: asyncio.Queue) -> None:
        if q in self._stdout_handler:
            self._stdout_handler.remove(q)

    def subscribe_status(self) -> asyncio.Queue[ServerStatus]:
        q: asyncio.Queue[ServerStatus] = asyncio.Queue()
        q.put_nowait(self._status)
        self._status_listeners.append(q)
        return q

    def unsubscribe_status(self, q: asyncio.Queue) -> None:
        if q in self._status_listeners:
            self._status_listeners.remove(q)

    def _notify_status(self) -> None:
        for q in self._status_listeners:
            q.put_nowait(self._status)

    def get_history(self) -> list[str]:
        return list(self._history)

    async def _reader_loop(self) -> None:
        assert self._process and self._process.stdout
        try:
            while self._process and self._process.returncode is None:
                line = await asyncio.get_event_loop().run_in_executor(None, self._process.stdout.readline)
                if not line:
                    break
                line = line.rstrip("\n\r")

                m = _TPS_RE.search(line)
                if m:
                    self._last_tps = float(m.group(1))

                self._history.append(line)
                if len(self._history) > self._max_history:
                    self._history.pop(0)
                for q in self._stdout_handler:
                    q.put_nowait(line)
        except (ValueError, OSError) as e:
            logger.debug("Reader loop ended: %s", e)
        finally:
            logger.info("Console reader loop finished")

    async def _writer_loop(self) -> None:
        try:
            while self._process and self._process.returncode is None:
                cmd = await self._stdin_queue.get()
                await self._send_command(cmd)
        except Exception as e:
            logger.debug("Writer loop ended: %s", e)

    async def _watchdog_loop(self) -> None:
        try:
            while self._process and self._process.returncode is None:
                await asyncio.sleep(1)

            returncode = self._process.poll() if self._process else None
            if returncode is not None and not self._was_intentional_stop:
                self._crash_count += 1
                logger.warning("Server exited unexpectedly (code %s, crash #%d)", returncode, self._crash_count)

                if self._auto_restart and self._crash_count <= self._max_crashes:
                    logger.info("Auto-restarting in %d seconds...", self._auto_restart_delay)
                    msg = (
                        f"[local] Server crashed (exit code {returncode})."
                        f" Restarting in {self._auto_restart_delay}s..."
                    )
                    self._history.append(msg)
                    for q in self._stdout_handler:
                        q.put_nowait(msg)

                    await asyncio.sleep(self._auto_restart_delay)
                    async with self._lock:
                        if self._process and self._process.returncode is not None:
                            self._cleanup()
                    await self.start()
                else:
                    if self._crash_count > self._max_crashes:
                        reason = f"crashed (max {self._max_crashes})"
                    else:
                        reason = "auto-restart disabled"
                    logger.warning("Server %s. Not restarting.", reason)
                    async with self._lock:
                        self._cleanup()
                        self._status = ServerStatus.crashed
                        self._write_lock_state("unlocked")
                        self._notify_status()
            elif returncode is not None:
                self._crash_count = 0
        except asyncio.CancelledError:
            pass
        except Exception as e:
            logger.error("Watchdog error: %s", e)

    def _cleanup(self) -> None:
        if self._reader_task and not self._reader_task.done():
            self._reader_task.cancel()
        if self._writer_task and not self._writer_task.done():
            self._writer_task.cancel()
        if self._watchdog_task and not self._watchdog_task.done():
            self._watchdog_task.cancel()
        self._process = None
        self._history.clear()

    def _write_lock_state(self, state: str) -> None:
        try:
            self._lock_file.write_text(
                f"[console]\nstate = {state}\ntimestamp = {time.time():.0f}\n"
            )
        except OSError as e:
            logger.error("Failed to write lock state: %s", e)

    def get_status_dict(self) -> dict:
        return {
            "status": self._status.value,
            "pid": self.pid,
            "uptime": self.uptime,
            "version": self._server_version,
            "tps": self._last_tps,
            "crash_count": self._crash_count,
            "auto_restart": self._auto_restart,
        }
