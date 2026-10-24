from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone


@dataclass
class Role:
    name: str
    display_name: str = ""
    permissions: list[str] | None = None
    is_default: bool = False
    created_at: str = ""

    def __post_init__(self):
        if not self.display_name:
            self.display_name = self.name
        if not self.created_at:
            self.created_at = datetime.now(timezone.utc).isoformat()
        if self.permissions is None:
            self.permissions = []

    def to_dict(self) -> dict:
        return {
            "name": self.name,
            "display_name": self.display_name,
            "permissions": sorted(self.permissions or []),
            "is_default": self.is_default,
            "created_at": self.created_at,
        }

    @classmethod
    def from_dict(cls, data: dict) -> Role:
        return cls(
            name=data["name"],
            display_name=data.get("display_name", data["name"]),
            permissions=data.get("permissions", []),
            is_default=data.get("is_default", False),
            created_at=data.get("created_at", ""),
        )
