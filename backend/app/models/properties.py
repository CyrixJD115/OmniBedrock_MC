from __future__ import annotations

from pydantic import BaseModel


class PropertiesEntry(BaseModel):
    key: str
    value: str
    comment: str = ""
    inline_comment: str = ""
