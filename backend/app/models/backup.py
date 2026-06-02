from __future__ import annotations

from datetime import datetime
from pydantic import BaseModel


class BackupEntry(BaseModel):
    filename: str
    world: str
    size_bytes: int
    modified: datetime
    comment: str = ""
