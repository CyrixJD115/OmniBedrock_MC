from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from backend.app.core import auth
from backend.app.core import roles as roles_module
from backend.app.core.dependencies import backup_service
from backend.app.main import app
from backend.app.models.role import Role
from backend.app.models.user import User, UserRole


@pytest.fixture
def client():
    return TestClient(app)


@pytest.fixture
def auth_header():
    if not roles_module._roles:
        roles_module._roles = {n: Role(**r.to_dict()) for n, r in roles_module._DEFAULT_ROLES.items()}
    user = User(username="admin", password_hash="x", role=UserRole.admin)
    auth._users["admin"] = user
    token = auth.create_access_token(user)
    return {"Authorization": f"Bearer {token}"}


def test_get_settings_returns_defaults(client, auth_header, tmp_path, monkeypatch):
    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: tmp_path / "backup_settings.yaml",
    )
    r = client.get("/api/v1/backups/settings", headers=auth_header)
    assert r.status_code == 200
    data = r.json()
    assert "manual" in data and "auto" in data and "pre_post" in data


def test_put_settings_persists(client, auth_header, tmp_path, monkeypatch):
    path = tmp_path / "backup_settings.yaml"
    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path", lambda: path
    )
    r = client.put(
        "/api/v1/backups/settings",
        headers=auth_header,
        json={"pre_post": {"before": [{"type": "send", "value": "say hi"}], "after": []}},
    )
    assert r.status_code == 200
    assert "say hi" in path.read_text()


def test_include_items_lists_world_entries(client, auth_header, tmp_path, monkeypatch):
    monkeypatch.setattr(
        "backend.app.services.backup_service.settings.bedrock_server_dir", str(tmp_path)
    )
    world = tmp_path / "worlds" / "W"
    world.mkdir(parents=True)
    (world / "db").mkdir()
    (world / "level.dat").write_bytes(b"x")
    r = client.get("/api/v1/backups/include-items", headers=auth_header, params={"world": "W"})
    assert r.status_code == 200
    names = {i["name"] for i in r.json()["items"]}
    assert "db" in names and "level.dat" in names


def test_create_returns_409_when_busy(client, auth_header, monkeypatch):
    monkeypatch.setattr(backup_service, "_active", True)
    r = client.post(
        "/api/v1/backups/create", headers=auth_header, json={"world": "W", "tag": "manual", "run_hooks": False}
    )
    assert r.status_code == 409


def test_test_command_runs_send(client, auth_header, monkeypatch):
    sent: list[str] = []

    async def mock_send(cmd: str, notify=None) -> None:
        sent.append(cmd)

    monkeypatch.setattr(
        "backend.app.routers.backups._send_to_server", mock_send
    )
    r = client.post(
        "/api/v1/backups/test-command",
        headers=auth_header,
        json={"entry": {"type": "send", "value": "say test"}},
    )
    assert r.status_code == 200
    assert sent == ["say test"]
    assert r.json()["kind"] == "send"
