from __future__ import annotations

import json
import logging
from datetime import datetime, timezone
from pathlib import Path

from backend.app.core.config import settings

logger = logging.getLogger("audit")

_audit_file: Path = Path(settings.logs_dir) / "audit.jsonl"
MAX_ENTRIES = 10000


def log_action(
    username: str,
    action: str,
    detail: str = "",
    category: str = "general",
    ip: str = "",
) -> dict:
    entry = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "username": username,
        "action": action,
        "detail": detail,
        "category": category,
        "ip": ip,
    }
    _audit_file.parent.mkdir(parents=True, exist_ok=True)
    try:
        with _audit_file.open("a", encoding="utf-8") as f:
            f.write(json.dumps(entry) + "\n")
    except OSError as e:
        logger.error("Failed to write audit entry: %s", e)
        return entry

    try:
        lines = _audit_file.read_text(encoding="utf-8").strip().splitlines()
        if len(lines) > MAX_ENTRIES:
            _audit_file.write_text("\n".join(lines[-MAX_ENTRIES:]) + "\n", encoding="utf-8")
    except OSError:
        pass
    return entry


def query_audit(
    limit: int = 100,
    offset: int = 0,
    username: str | None = None,
    action: str | None = None,
    category: str | None = None,
) -> list[dict]:
    if not _audit_file.exists():
        return []

    try:
        lines = _audit_file.read_text(encoding="utf-8").strip().splitlines()
    except OSError:
        return []

    entries: list[dict] = []
    for line in lines:
        if not line.strip():
            continue
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue

        if username and entry.get("username") != username:
            continue
        if action and action not in entry.get("action", ""):
            continue
        if category and entry.get("category") != category:
            continue
        entries.append(entry)

    entries.reverse()
    return entries[offset:offset + limit]
