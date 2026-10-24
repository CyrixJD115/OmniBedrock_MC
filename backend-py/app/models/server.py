from __future__ import annotations

from enum import Enum


class ServerStatus(str, Enum):
    stopped = "stopped"
    starting = "starting"
    running = "running"
    stopping = "stopping"
    crashed = "crashed"
