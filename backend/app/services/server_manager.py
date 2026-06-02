from __future__ import annotations

import asyncio
import logging
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

from backend.app.core.config import settings
from backend.app.models.server import ServerStatus

logger = logging.getLogger("server_manager")


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

        self._reader_task: asyncio.Task | None = None
        self._writer_task: asyncio.Task | None = None

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

    def _resolve_endstone(self) -> str:
        endstone = shutil.which("endstone")
        if endstone:
            return endstone
        result = subprocess.run(
            [sys.executable, "-m", "endstone", "--version"],
            capture_output=True, text=True, timeout=10
        )
        if result.returncode == 0:
            self._server_version = result.stdout.strip()
            return f"{sys.executable} -m endstone"
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

            cmd = shlex.split(endstone_cmd)
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

            self._status = ServerStatus.running
            self._notify_status()
            return f"Server started (PID: {self._process.pid})"

    async def stop(self) -> str:
        async with self._lock:
            if self._status != ServerStatus.running:
                return "Server is not running"

            self._status = ServerStatus.stopping
            self._notify_status()

            logger.info("Stopping server gracefully...")
            await self._send_command("stop")

            wait_start = time.time()
            while time.time() - wait_start < 8:
                if self._process and self._process.returncode is not None:
                    break
                await asyncio.sleep(0.5)

            if self._process and self._process.returncode is None:
                logger.warning("Server did not stop gracefully, terminating...")
                self._process.terminate()
                try:
                    self._process.wait(timeout=4)
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

    async def _reader_loop(self) -> None:
        assert self._process and self._process.stdout
        try:
            while self._process and self._process.returncode is None:
                line = await asyncio.get_event_loop().run_in_executor(None, self._process.stdout.readline)
                if not line:
                    break
                line = line.rstrip("\n\r")
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

    def _cleanup(self) -> None:
        if self._reader_task and not self._reader_task.done():
            self._reader_task.cancel()
        if self._writer_task and not self._writer_task.done():
            self._writer_task.cancel()
        self._process = None

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
        }

