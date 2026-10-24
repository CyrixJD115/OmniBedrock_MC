import backend.app.core.auth as auth
from backend.app.models.user import UserRole


def test_users_yaml_roundtrip(tmp_path, monkeypatch):
    monkeypatch.setattr(auth, "_user_file", tmp_path / "users.yaml")
    auth._users = {}
    created = auth.create_user("alice", "pw123", UserRole.admin, "Alice")
    assert created is not None

    raw = (tmp_path / "users.yaml").read_text(encoding="utf-8")
    assert not raw.lstrip().startswith("{"), "user store must be YAML, not JSON"

    loaded = auth._load_users()
    assert "alice" in loaded
    assert loaded["alice"].role == UserRole.admin
    assert loaded["alice"].display_name == "Alice"
    assert auth._verify_password("pw123", loaded["alice"].password_hash)
    assert not auth._verify_password("wrong", loaded["alice"].password_hash)


def test_init_users_creates_admin_when_empty(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(auth, "_user_file", tmp_path / "users.yaml")
    auth._users = {}
    auth.init_users()
    out = capsys.readouterr().out
    assert "Default admin account created" in out
    assert "Password:" in out
    assert (tmp_path / "users.yaml").exists()


def test_init_users_skips_when_users_exist(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(auth, "_user_file", tmp_path / "users.yaml")
    auth._users = {}
    auth.init_users()
    capsys.readouterr()  # drain first creation
    auth.init_users()  # second call should NOT reprint
    out = capsys.readouterr().out
    assert "Default admin account created" not in out


def test_reset_admin_store_deletes_file(tmp_path, monkeypatch):
    monkeypatch.setattr(auth, "_user_file", tmp_path / "users.yaml")
    auth._users = {}
    auth.create_user("bob", "pw", UserRole.admin)
    assert (tmp_path / "users.yaml").exists()
    auth.reset_admin_store()
    assert not (tmp_path / "users.yaml").exists()
    assert auth._users == {}
