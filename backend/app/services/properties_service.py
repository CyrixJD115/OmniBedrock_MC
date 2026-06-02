from __future__ import annotations

import re
from pathlib import Path

from backend.app.core.config import settings
from backend.app.models.properties import PropertiesEntry


class PropertiesService:
    def __init__(self) -> None:
        self._file_path = Path(settings.bedrock_server_dir) / "server.properties"
        self._entries: list[PropertiesEntry] = []

    def load(self) -> list[PropertiesEntry]:
        if not self._file_path.exists():
            self._entries = []
            return []
        text = self._file_path.read_text(encoding="utf-8")
        self._entries = self._parse(text)
        return self._entries

    def _parse(self, text: str) -> list[PropertiesEntry]:
        entries: list[PropertiesEntry] = []
        for line in text.splitlines():
            stripped = line.strip()
            if not stripped:
                entries.append(PropertiesEntry(key="", value="", comment="", inline_comment=""))
                continue
            if stripped.startswith("#"):
                entries.append(PropertiesEntry(key="", value="", comment=stripped, inline_comment=""))
                continue
            inline_comment = ""
            if "#" in stripped:
                idx = stripped.index("#")
                inline_comment = stripped[idx:].strip()
                stripped = stripped[:idx].strip()
            if "=" in stripped:
                key, val = stripped.split("=", 1)
                entries.append(PropertiesEntry(key=key.strip(), value=val.strip(), inline_comment=inline_comment))
            else:
                entries.append(PropertiesEntry(key=stripped, value="", inline_comment=inline_comment))
        return entries

    def update(self, key: str, value: str) -> PropertiesEntry | None:
        for entry in self._entries:
            if entry.key == key:
                entry.value = value
                return entry
        return None

    def get(self, key: str) -> PropertiesEntry | None:
        for entry in self._entries:
            if entry.key == key:
                return entry
        return None

    def save(self) -> None:
        self._file_path.parent.mkdir(parents=True, exist_ok=True)
        lines: list[str] = []
        for entry in self._entries:
            if entry.comment:
                lines.append(entry.comment)
            elif not entry.key:
                lines.append("")
            else:
                val = f"{entry.key}={entry.value}"
                if entry.inline_comment:
                    val += f" {entry.inline_comment}"
                lines.append(val)
        self._file_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def get_raw(self) -> str:
        if self._file_path.exists():
            return self._file_path.read_text(encoding="utf-8")
        return ""

    def save_raw(self, text: str) -> None:
        self._file_path.parent.mkdir(parents=True, exist_ok=True)
        self._file_path.write_text(text, encoding="utf-8")
        self._entries = self._parse(text)
