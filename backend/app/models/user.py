from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from enum import Enum


class UserRole(str, Enum):
    owner = "owner"
    admin = "admin"
    moderator = "moderator"
    viewer = "viewer"


@dataclass
class User:
    username: str
    password_hash: str
    role: UserRole = UserRole.viewer
    display_name: str = ""
    created_at: str = ""
    last_login: str = ""

    def __post_init__(self):
        if not self.created_at:
            self.created_at = datetime.now(timezone.utc).isoformat()
        if not self.display_name:
            self.display_name = self.username

    def to_dict(self) -> dict:
        return {
            "username": self.username,
            "role": self.role.value,
            "display_name": self.display_name,
            "created_at": self.created_at,
            "last_login": self.last_login,
        }

    def to_safe_dict(self) -> dict:
        return self.to_dict()

    @classmethod
    def from_dict(cls, data: dict) -> User:
        return cls(
            username=data["username"],
            password_hash=data["password_hash"],
            role=UserRole(data.get("role", "viewer")),
            display_name=data.get("display_name", data["username"]),
            created_at=data.get("created_at", ""),
            last_login=data.get("last_login", ""),
        )
