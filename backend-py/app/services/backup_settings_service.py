from __future__ import annotations

from pathlib import Path

import yaml

from backend.app.core.config import settings

_VALID_TYPES = {"command", "wait", "comment", "send"}

DEFAULT_MANUAL: dict = {
    "world": "",
    "full_backup": True,
    "zip_prefix": "manual_backup",
    "export_folder": "",
    "compression": "deflate",
    "dry_run": False,
    "include_items": [],
}

DEFAULT_AUTO: dict = {
    "enabled": False,
    "interval_minutes": 30,
    "keep_count": 10,
    "export_folder": "",
    "compression": "deflate",
    "full_backup": True,
    "include_items": [],
}

DEFAULT_PRE_POST: dict = {"before": [], "after": []}


def _settings_path() -> Path:
    return Path(settings.data_dir) / "backup_settings.yaml"


def _validate_entries(entries: list[dict]) -> None:
    for e in entries:
        if e.get("type") not in _VALID_TYPES:
            raise ValueError(f"Invalid command type: {e.get('type')!r}")
        if e.get("type") == "wait":
            n = int(e.get("value", 0))
            if not 1 <= n <= 600:
                raise ValueError("wait value must be 1..600 seconds")


class BackupSettingsService:
    def __init__(self) -> None:
        pass

    def _path(self) -> Path:
        return _settings_path()

    def load(self) -> dict:
        path = self._path()
        if not path.exists():
            data = {
                "manual": dict(DEFAULT_MANUAL),
                "auto": dict(DEFAULT_AUTO),
                "pre_post": {"before": [], "after": []},
            }
            self._write(data)
            return data
        with path.open("r", encoding="utf-8") as f:
            raw = yaml.safe_load(f) or {}
        return {
            "manual": {**DEFAULT_MANUAL, **(raw.get("manual") or {})},
            "auto": {**DEFAULT_AUTO, **(raw.get("auto") or {})},
            "pre_post": {
                "before": (raw.get("pre_post") or {}).get("before") or [],
                "after": (raw.get("pre_post") or {}).get("after") or [],
            },
        }

    def save(
        self,
        manual: dict | None = None,
        auto: dict | None = None,
        pre_post: dict | None = None,
    ) -> dict:
        data = self.load()
        if manual is not None:
            data["manual"] = {**DEFAULT_MANUAL, **manual}
        if auto is not None:
            data["auto"] = {**DEFAULT_AUTO, **auto}
        if pre_post is not None:
            before = pre_post.get("before", [])
            after = pre_post.get("after", [])
            _validate_entries(before)
            _validate_entries(after)
            data["pre_post"] = {"before": before, "after": after}
        self._write(data)
        return data

    def _write(self, data: dict) -> None:
        path = self._path()
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("w", encoding="utf-8") as f:
            yaml.safe_dump(data, f, default_flow_style=False, sort_keys=False)
