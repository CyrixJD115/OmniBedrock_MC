from __future__ import annotations

import pytest

from backend.app.services.backup_settings_service import (
    DEFAULT_AUTO,
    DEFAULT_MANUAL,
    BackupSettingsService,
)


@pytest.fixture
def service(tmp_path, monkeypatch):
    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: tmp_path / "backup_settings.yaml",
    )
    return BackupSettingsService()


def test_defaults_when_absent(service):
    data = service.load()
    assert data["manual"] == DEFAULT_MANUAL
    assert data["auto"] == DEFAULT_AUTO
    assert data["pre_post"] == {"before": [], "after": []}


def test_roundtrip_persists_yaml(service):
    pre_post = {"before": [{"type": "send", "value": "say Backup starting"}], "after": []}
    service.save(pre_post=pre_post)
    raw = service._path().read_text()
    assert not raw.lstrip().startswith("{"), "must be YAML, not JSON"
    assert "say Backup starting" in raw
    reloaded = service.load()
    assert reloaded["pre_post"]["before"][0]["value"] == "say Backup starting"


def test_partial_save_preserves_other_sections(service):
    service.save(manual={"world": "MyWorld", "full_backup": False})
    service.save(auto={"enabled": True, "interval_minutes": 5})
    data = service.load()
    assert data["manual"]["world"] == "MyWorld"
    assert data["auto"]["enabled"] is True
    assert data["pre_post"] == {"before": [], "after": []}


def test_invalid_command_type_rejected(service):
    with pytest.raises(ValueError):
        service.save(pre_post={"before": [{"type": "bomb", "value": "x"}], "after": []})
