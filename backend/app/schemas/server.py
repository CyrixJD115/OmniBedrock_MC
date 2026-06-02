from __future__ import annotations

from pydantic import BaseModel


class ServerStatusResponse(BaseModel):
    status: str
    pid: int | None = None
    uptime: float | None = None
    version: str | None = None


class ServerActionRequest(BaseModel):
    action: str  # start, stop, restart, kill


class ServerActionResponse(BaseModel):
    success: bool
    message: str
