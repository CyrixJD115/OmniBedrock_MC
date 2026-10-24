from __future__ import annotations

import logging
from pathlib import Path

import yaml

from backend.app.core.config import settings
from backend.app.core.permissions import ALL_PERMISSIONS
from backend.app.models.role import Role

logger = logging.getLogger("roles")

_role_file: Path = Path(settings.data_dir) / "roles.yaml"
_roles: dict[str, Role] = {}

_DEFAULT_ROLES = {
    "owner": Role(
        name="owner",
        display_name="Owner",
        permissions=list(ALL_PERMISSIONS),
        is_default=False,
    ),
    "admin": Role(
        name="admin",
        display_name="Admin",
        permissions=list(ALL_PERMISSIONS),
        is_default=False,
    ),
    "moderator": Role(
        name="moderator",
        display_name="Moderator",
        permissions=[
            "CONSOLE_VIEW", "CONSOLE_SEND",
            "SERVER_VIEW",
            "PLAYERS_VIEW", "PLAYERS_KICK",
            "PROPERTIES_VIEW",
            "ADDONS_VIEW",
            "BACKUPS_VIEW", "BACKUPS_CREATE", "BACKUPS_RESTORE", "BACKUPS_DELETE",
            "FILES_VIEW",
            "AUDIT_VIEW",
            "SETTINGS_VIEW",
        ],
        is_default=False,
    ),
    "viewer": Role(
        name="viewer",
        display_name="Viewer",
        permissions=[
            "CONSOLE_VIEW",
            "SERVER_VIEW",
            "PLAYERS_VIEW",
            "AUDIT_VIEW",
        ],
        is_default=True,
    ),
}


def _load_roles() -> dict[str, Role]:
    if not _role_file.exists():
        return {}
    try:
        data = yaml.safe_load(_role_file.read_text(encoding="utf-8"))
        if not data:
            return {}
        return {r: Role.from_dict(d) for r, d in data.items()}
    except (yaml.YAMLError, OSError, KeyError) as e:
        logger.error("Failed to load roles: %s", e)
        return {}


def _save_roles() -> None:
    _role_file.parent.mkdir(parents=True, exist_ok=True)
    data = {r: role.to_dict() for r, role in _roles.items()}
    _role_file.write_text(yaml.safe_dump(data, default_flow_style=False, sort_keys=False), encoding="utf-8")


def init_roles() -> None:
    global _roles
    _roles = _load_roles()
    if not _roles:
        logger.info("No roles found. Creating default roles.")
        _roles = {r: Role(**role.to_dict()) for r, role in _DEFAULT_ROLES.items()}
        _save_roles()


def get_role(name: str) -> Role | None:
    return _roles.get(name)


def list_roles() -> list[Role]:
    return list(_roles.values())


def create_role(role: Role) -> bool:
    if role.name in _roles:
        return False
    _roles[role.name] = role
    _save_roles()
    return True


def update_role(name: str, **kwargs) -> Role | None:
    role = _roles.get(name)
    if not role:
        return None
    for k, v in kwargs.items():
        if hasattr(role, k) and v is not None:
            setattr(role, k, v)
    _save_roles()
    return role


def delete_role(name: str) -> bool:
    if name not in _roles:
        return False
    del _roles[name]
    _save_roles()
    return True


def resolve_permissions(role_name: str) -> list[str]:
    if role_name == "owner":
        return list(ALL_PERMISSIONS)
    role = get_role(role_name)
    if not role:
        return []
    return list(role.permissions or [])


def get_default_role_name() -> str:
    for role in _roles.values():
        if role.is_default:
            return role.name
    return "viewer"
