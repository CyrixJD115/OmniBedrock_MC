from __future__ import annotations

import re
from typing import Any

KNOWN_PROPERTIES: dict[str, dict[str, Any]] = {
    "server-name": {"type": "string", "default": "Dedicated Server"},
    "gamemode": {"type": "string", "default": "survival", "options": ["survival", "creative", "adventure"]},
    "difficulty": {"type": "string", "default": "easy", "options": ["peaceful", "easy", "normal", "hard"]},
    "allow-cheats": {"type": "bool", "default": False},
    "max-players": {"type": "int", "default": 10, "min": 1, "max": 1000},
    "online-mode": {"type": "bool", "default": True},
    "white-list": {"type": "bool", "default": False},
    "server-port": {"type": "int", "default": 19132, "min": 1, "max": 65535},
    "server-portv6": {"type": "int", "default": 19133, "min": 1, "max": 65535},
    "view-distance": {"type": "int", "default": 32, "min": 2, "max": 96},
    "tick-distance": {"type": "int", "default": 4, "min": 4, "max": 12},
    "player-idle-timeout": {"type": "int", "default": 30, "min": 0},
    "level-name": {"type": "string", "default": "Bedrock level"},
    "level-seed": {"type": "string", "default": ""},
    "default-player-permission-level": {
        "type": "string", "default": "member",
        "options": ["visitor", "member", "operator"],
    },
    "texturepack-required": {"type": "bool", "default": False},
    "content-log-file-enabled": {"type": "bool", "default": False},
    "compression-threshold": {"type": "int", "default": 1, "min": 0, "max": 65535},
    "server-authoritative-movement": {"type": "bool", "default": True},
    "enable-lan-visibility": {"type": "bool", "default": True},
    "chat-restriction": {"type": "string", "default": "None", "options": ["None", "Dropped", "Disabled"]},
    "disable-player-interaction": {"type": "bool", "default": False},
}


def validate_property(key: str, value: str) -> tuple[bool, str]:
    if key not in KNOWN_PROPERTIES:
        return True, ""

    prop = KNOWN_PROPERTIES[key]
    ptype = prop["type"]

    if ptype == "int":
        try:
            val = int(value)
        except ValueError:
            return False, f"'{key}' must be an integer"
        vmin = prop.get("min")
        vmax = prop.get("max")
        if vmin is not None and val < vmin:
            return False, f"'{key}' minimum is {vmin}"
        if vmax is not None and val > vmax:
            return False, f"'{key}' maximum is {vmax}"

    elif ptype == "bool":
        if value.lower() not in ("true", "false", "1", "0"):
            return False, f"'{key}' must be true/false"

    elif ptype == "string":
        options = prop.get("options")
        if options and value not in options:
            return False, f"'{key}' must be one of: {', '.join(options)}"

    return True, ""


def parse_version_string(s: str) -> tuple[int, ...]:
    parts = re.findall(r"\d+", s)
    return tuple(int(p) for p in parts[:4]) or (0,)
