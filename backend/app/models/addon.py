from __future__ import annotations

from pydantic import BaseModel


class Addon(BaseModel):
    name: str
    path: str
    pack_type: str  # behavior_pack or resource_pack
    uuid: str
    version: list[int]
    valid: bool
    manifest: dict | None = None
