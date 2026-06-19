from __future__ import annotations

import asyncio

import pytest

from backend.app.services.backup_service import BackupAlreadyRunning, BackupService


@pytest.fixture
def svc(tmp_path, monkeypatch):
    monkeypatch.setattr("backend.app.services.backup_service.settings.backups_dir", str(tmp_path))
    monkeypatch.setattr(
        "backend.app.services.backup_service.settings.bedrock_server_dir", str(tmp_path / "server")
    )
    (tmp_path / "server" / "worlds" / "W").mkdir(parents=True)
    (tmp_path / "server" / "worlds" / "W" / "level.dat").write_bytes(b"x")
    return BackupService()


def _collect():
    events: list[dict] = []

    async def notify(e: dict) -> None:
        events.append(e)

    return events, notify


def test_pre_zip_post_order(svc, monkeypatch):
    called: list[str] = []

    async def fake_zip(world, backup_path, *a, **kw):
        called.append("zip")
        backup_path.write_bytes(b"zip")

    monkeypatch.setattr(svc, "_create_zip", fake_zip)
    monkeypatch.setattr(
        "backend.app.services.backup_service._send_to_server",
        lambda cmd: called.append(f"send:{cmd}") or None,
    )
    events, notify = _collect()
    pre_post = {
        "before": [{"type": "send", "value": "say pre"}],
        "after": [{"type": "send", "value": "say post"}],
    }
    asyncio.run(
        svc.run_backup(
            "W", tag="manual", full_backup=True, zip_prefix="manual_backup",
            export_folder="", compression="store", include_items=None,
            pre_post=pre_post, dry_run=False, notify=notify,
        )
    )
    assert called == ["send:say pre", "zip", "send:say post"]


def test_wait_sleeps_and_comment_skipped(svc, monkeypatch):
    slept: list[int] = []

    async def fake_sleep(n):
        slept.append(n)

    async def fake_zip(*a, **kw):
        pass

    monkeypatch.setattr("backend.app.services.backup_service.asyncio.sleep", fake_sleep)
    monkeypatch.setattr(svc, "_create_zip", fake_zip)
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: None)
    events, notify = _collect()
    pre_post = {
        "before": [{"type": "wait", "value": 5}, {"type": "comment", "value": "hi"}],
        "after": [],
    }
    asyncio.run(
        svc.run_backup(
            "W", tag="manual", full_backup=True, zip_prefix="p",
            export_folder="", compression="store", include_items=None,
            pre_post=pre_post, dry_run=False, notify=notify,
        )
    )
    assert slept == [5]
    assert any(e["type"] == "output" for e in events)


def test_dry_run_skips_zip_but_send_fires(svc, monkeypatch):
    zip_called: list[int] = []

    async def fake_zip(*a, **kw):
        zip_called.append(1)

    monkeypatch.setattr(svc, "_create_zip", fake_zip)
    sends: list[str] = []
    monkeypatch.setattr(
        "backend.app.services.backup_service._send_to_server", lambda cmd: sends.append(cmd)
    )
    events, notify = _collect()
    pre_post = {"before": [{"type": "send", "value": "say hi"}], "after": []}
    asyncio.run(
        svc.run_backup(
            "W", tag="manual", full_backup=True, zip_prefix="p",
            export_folder="", compression="store", include_items=None,
            pre_post=pre_post, dry_run=True, notify=notify,
        )
    )
    assert zip_called == []
    assert sends == ["say hi"]


def test_one_job_at_a_time(svc, monkeypatch):
    async def fake_zip(*a, **kw):
        pass

    monkeypatch.setattr(svc, "_create_zip", fake_zip)
    monkeypatch.setattr("backend.app.services.backup_service._send_to_server", lambda cmd: None)
    svc._active = True
    events, notify = _collect()
    with pytest.raises(BackupAlreadyRunning):
        asyncio.run(
            svc.run_backup(
                "W", tag="manual", full_backup=True, zip_prefix="p",
                export_folder="", compression="store", include_items=None,
                pre_post={"before": [], "after": []}, dry_run=False, notify=notify,
            )
        )
