from __future__ import annotations

from pydantic import BaseModel


class Player(BaseModel):
    name: str
    uuid: str | None = None
    xuid: str | None = None
    ip: str | None = None
