from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from backend.app.main import app
from backend.app.models.user import User, UserRole


def test_ws_rejects_missing_token():
    client = TestClient(app)
    with pytest.raises(Exception):
        with client.websocket_connect("/api/v1/backups/ws") as ws:
            ws.receive_json()


def test_ws_rejects_invalid_token():
    client = TestClient(app)
    with pytest.raises(Exception):
        with client.websocket_connect("/api/v1/backups/ws?token=garbage") as ws:
            ws.receive_json()


def test_ws_accepts_when_user_resolves(monkeypatch):
    monkeypatch.setattr(
        "backend.app.websocket.backup.get_user_from_token",
        lambda token: User(username="admin", password_hash="x", role=UserRole.admin),
    )
    client = TestClient(app)
    with client.websocket_connect("/api/v1/backups/ws?token=anything") as ws:
        data = ws.receive_json()
        assert data["type"] == "hello"
        assert "active" in data
