from __future__ import annotations

from pydantic import BaseModel


class ConsoleCommandRequest(BaseModel):
    command: str


class ConsoleCommandResponse(BaseModel):
    success: bool
    message: str = ""


class ConsoleLine(BaseModel):
    text: str
    level: str = "info"
    timestamp: float | None = None


class CommandHistoryItem(BaseModel):
    command: str
    timestamp: float
