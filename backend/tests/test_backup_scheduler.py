from __future__ import annotations

import asyncio

from backend.app.managers.backup_scheduler import BackupScheduler
from backend.app.services.backup_service import BackupService
from backend.app.services.backup_settings_service import BackupSettingsService


def test_scheduler_runs_backup_with_hooks(tmp_path, monkeypatch):
    monkeypatch.setattr("backend.app.services.backup_service.settings.backups_dir", str(tmp_path))
    server_dir = tmp_path / "server"
    monkeypatch.setattr(
        "backend.app.services.backup_service.settings.bedrock_server_dir", str(server_dir)
    )
    (server_dir / "worlds" / "W").mkdir(parents=True)
    (server_dir / "worlds" / "W" / "level.dat").write_bytes(b"x")

    monkeypatch.setattr(
        "backend.app.services.backup_settings_service._settings_path",
        lambda: tmp_path / "backup_settings.yaml",
    )

    svc = BackupService()
    settings_svc = BackupSettingsService()
    settings_svc.save(
        auto={"enabled": True, "interval_minutes": 0, "keep_count": 1},
        pre_post={
            "before": [{"type": "send", "value": "save hold"}, {"type": "send", "value": "say auto"}],
            "after": [{"type": "send", "value": "save resume"}],
        },
    )

    sent: list[str] = []

    async def mock_send(cmd: str, notify=None) -> None:
        sent.append(cmd)

    monkeypatch.setattr(
        "backend.app.services.backup_service._send_to_server", mock_send
    )

    calls: list[str] = []
    orig = svc.run_backup

    async def spy(world, tag, **kw):
        calls.append(tag)
        kw.setdefault("pre_post", {"before": [], "after": []})
        return await orig(world, tag, **kw)

    monkeypatch.setattr(svc, "run_backup", spy)

    sched = BackupScheduler(svc, settings_svc)
    sched.configure(enabled=True, interval_minutes=0, keep_count=1)
    asyncio.run(sched.tick_once())
    assert "auto" in calls
    assert "save hold" in sent
    assert "save resume" in sent
    assert "say auto" in sent
