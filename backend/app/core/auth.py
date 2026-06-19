from __future__ import annotations

import hashlib
import json
import logging
import os
import secrets
import time
from datetime import datetime, timezone
from pathlib import Path

import jwt

from backend.app.core.config import settings
from backend.app.models.user import User, UserRole

logger = logging.getLogger("auth")

ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_SECONDS = 86400 * 7  # 7 days

_user_file: Path = Path(settings.data_dir) / "users.json"
_users: dict[str, User] = {}


def _load_users() -> dict[str, User]:
    if not _user_file.exists():
        return {}
    try:
        data = json.loads(_user_file.read_text(encoding="utf-8"))
        return {u: User.from_dict(d) for u, d in data.items()}
    except (json.JSONDecodeError, OSError, KeyError) as e:
        logger.error("Failed to load users: %s", e)
        return {}


def _save_users() -> None:
    _user_file.parent.mkdir(parents=True, exist_ok=True)
    data = {u: user.to_dict() | {"password_hash": user.password_hash} for u, user in _users.items()}
    _user_file.write_text(json.dumps(data, indent=2), encoding="utf-8")


def init_users() -> None:
    global _users
    _users = _load_users()
    if not _users:
        logger.info("No users found. Creating default admin user.")
        username = "admin"
        password = secrets.token_hex(8)
        hashed = _hash_password(password)
        user = User(
            username=username,
            password_hash=hashed,
            role=UserRole.owner,
            display_name="Administrator",
        )
        _users[username] = user
        _save_users()
        logger.info("Default admin account created — Username: %s, Password: %s", username, password)
        print(f"\n{'='*50}", flush=True)
        print("  Default admin account created", flush=True)
        print(f"  Username: {username}", flush=True)
        print(f"  Password: {password}", flush=True)
        print(f"{'='*50}\n", flush=True)


def _hash_password(password: str) -> str:
    salt = os.urandom(32)
    key = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, 100000)
    return salt.hex() + ":" + key.hex()


def _verify_password(password: str, stored: str) -> bool:
    try:
        salt_hex, key_hex = stored.split(":", 1)
        salt = bytes.fromhex(salt_hex)
        expected = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, 100000)
        return key_hex == expected.hex()
    except (ValueError, AttributeError):
        return False


def authenticate_user(username: str, password: str) -> User | None:
    user = _users.get(username)
    if not user:
        return None
    if not _verify_password(password, user.password_hash):
        return None
    user.last_login = datetime.now(timezone.utc).isoformat()
    _users[username] = user
    _save_users()
    return user


def create_user(username: str, password: str, role: UserRole, display_name: str = "") -> User | None:
    if username in _users:
        return None
    hashed = _hash_password(password)
    user = User(
        username=username,
        password_hash=hashed,
        role=role,
        display_name=display_name or username,
    )
    _users[username] = user
    _save_users()
    return user


def update_user(
    username: str,
    role: UserRole | None = None,
    display_name: str | None = None,
    password: str | None = None,
) -> User | None:
    user = _users.get(username)
    if not user:
        return None
    if role is not None:
        user.role = role
    if display_name is not None:
        user.display_name = display_name
    if password is not None:
        user.password_hash = _hash_password(password)
    _users[username] = user
    _save_users()
    return user


def delete_user(username: str) -> bool:
    if username not in _users:
        return False
    del _users[username]
    _save_users()
    return True


def get_user(username: str) -> User | None:
    return _users.get(username)


def list_users() -> list[User]:
    return list(_users.values())


def create_access_token(user: User) -> str:
    payload = {
        "sub": user.username,
        "role": user.role.value,
        "iat": int(time.time()),
        "exp": int(time.time()) + ACCESS_TOKEN_EXPIRE_SECONDS,
    }
    return jwt.encode(payload, settings.secret_key, algorithm=ALGORITHM)


def decode_access_token(token: str) -> dict | None:
    try:
        payload = jwt.decode(token, settings.secret_key, algorithms=[ALGORITHM])
        return payload
    except jwt.ExpiredSignatureError:
        return None
    except jwt.InvalidTokenError:
        return None


def get_user_from_token(token: str) -> User | None:
    payload = decode_access_token(token)
    if not payload:
        return None
    username = payload.get("sub")
    if not username:
        return None
    return get_user(username)
