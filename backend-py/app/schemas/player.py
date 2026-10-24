from __future__ import annotations

from pydantic import BaseModel


class PlayerListResponse(BaseModel):
    players: list[dict]
    count: int


class PlayerActionRequest(BaseModel):
    action: str
    target: str
    reason: str = ""
