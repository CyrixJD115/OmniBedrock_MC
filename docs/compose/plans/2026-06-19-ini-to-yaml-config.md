# Panel Config INI → YAML Implementation Plan

> [!NOTE]
> This document may not reflect the current implementation.
> See the final report for up-to-date state:
> [Final Report](../reports/ini-to-yaml-config.md)

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the panel's own state files (users, console lock state) from JSON/INI to YAML, rename `backend/ini/` → `backend/data/`, remove dead code, and add admin-lockout recovery.

**Architecture:** Direct in-place conversion using `yaml.safe_load`/`yaml.safe_dump` — no new abstraction layers. Rename the `settings.ini_dir` config to `settings.data_dir`. The game-owned `server.properties` is untouched.

**Tech Stack:** Python 3.11+, FastAPI, PyYAML, pytest (new dev dep), uv, ruff.

**Spec:** `docs/compose/specs/2026-06-19-ini-to-yaml-config-design.md`

**Working directory:** `/mnt/dev/Dev/Coding_Projects/Minecraft Server Panel/OmniBedrock_MC`

---

### Task 1: Add dependencies and scaffold tests

**Covers:** [S4] (pyproject row)

**Files:**
- Modify: `pyproject.toml`
- Create: `backend/tests/__init__.py`, `backend/tests/test_auth_yaml.py`

- [ ] **Step 1: Add deps to pyproject.toml**

In `pyproject.toml`, add `pyyaml` to the `dependencies` list (after `psutil>=6.0.0`) and add a dev dependency group. The final `[project]` dependencies block and new group:

```toml
dependencies = [
    "endstone>=0.11.0",
    "pyjwt>=2.13.0",
    "fastapi>=0.115.0",
    "uvicorn[standard]>=0.32.0",
    "websockets>=14.0",
    "pydantic>=2.0",
    "pydantic-settings>=2.0",
    "python-multipart>=0.0.12",
    "python-dateutil>=2.9",
    "watchdog>=4.0.0",
    "aiofiles>=24.0",
    "psutil>=6.0.0",
    "pyyaml>=6.0",
]

[dependency-groups]
dev = ["pytest>=8.0"]
```

- [ ] **Step 2: Sync deps**

Run: `uv sync`
Expected: resolves and installs `pyyaml` + `pytest`.

- [ ] **Step 3: Scaffold tests package**

Create empty `backend/tests/__init__.py`.

Create `backend/tests/test_auth_yaml.py` with a single baseline test to confirm pytest runs:

```python
import yaml


def test_pyyaml_importable():
    assert yaml.__version__
```

- [ ] **Step 4: Run baseline test**

Run: `uv run pytest backend/tests/test_auth_yaml.py -v`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add pyproject.toml uv.lock backend/tests/__init__.py backend/tests/test_auth_yaml.py
git commit -m "build: add pyyaml and pytest dev deps with tests scaffold"
```

---

### Task 2: Rename `ini_dir` → `data_dir` (atomic)

**Covers:** [S2], [S4] (config.py, __init__.py, settings.py rows)

This renames the config attribute and directory across all consumers in one atomic change so the app stays runnable. File formats stay JSON/INI in this task; conversion happens in Tasks 3–4.

**Files:**
- Modify: `backend/app/core/config.py:17`
- Modify: `backend/app/core/__init__.py` (delete duplicate `Settings`)
- Modify: `backend/app/core/auth.py:22`
- Modify: `backend/app/services/server_manager.py:48`
- Modify: `backend/app/routers/settings.py:19,47`

- [ ] **Step 1: config.py — rename attribute + path**

In `backend/app/core/config.py`, change line 17:

```python
    data_dir: str = str(Path(__file__).resolve().parent.parent.parent / "data")
```

- [ ] **Step 2: __init__.py — delete duplicate dead Settings**

Replace the entire contents of `backend/app/core/__init__.py` with:

```python
from __future__ import annotations
```

(Nothing imports `settings` from the `backend.app.core` package — verified. The canonical `Settings` lives in `config.py`.)

- [ ] **Step 3: auth.py — point at data_dir**

In `backend/app/core/auth.py`, change line 22:

```python
_user_file: Path = Path(settings.data_dir) / "users.json"
```

- [ ] **Step 4: server_manager.py — point at data_dir**

In `backend/app/services/server_manager.py`, change lines 48–50:

```python
        self._data_dir = Path(settings.data_dir)
        self._data_dir.mkdir(parents=True, exist_ok=True)
        self._lock_file = self._data_dir / "console_lock_state.ini"
```

- [ ] **Step 5: settings.py — rename response field**

In `backend/app/routers/settings.py`, change the `AppSettingsResponse` field (line 19) and the constructor arg (line 47):

```python
class AppSettingsResponse(BaseModel):
    app_name: str
    debug: bool
    bedrock_server_dir: str
    backups_dir: str
    data_dir: str
    logs_dir: str
```

and inside `get_settings`:

```python
        data_dir=app_settings.data_dir,
```

- [ ] **Step 6: Verify the app imports cleanly**

Run: `uv run python -c "import backend.app.main; print('ok')"`
Expected: prints `ok` (no `AttributeError: ini_dir`).

- [ ] **Step 7: Lint**

Run: `uv run ruff check backend/app/core/config.py backend/app/core/__init__.py backend/app/core/auth.py backend/app/services/server_manager.py backend/app/routers/settings.py`
Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add backend/app/core/config.py backend/app/core/__init__.py backend/app/core/auth.py backend/app/services/server_manager.py backend/app/routers/settings.py
git commit -m "refactor: rename settings.ini_dir to data_dir and drop duplicate Settings"
```

---

### Task 3: Convert auth storage to YAML (TDD)

**Covers:** [S4] (auth.py row), [S5], [S6]

**Files:**
- Modify: `backend/app/core/auth.py:1-40` (imports, `_load_users`, `_save_users`, `reset_admin_store`)
- Test: `backend/tests/test_auth_yaml.py`

- [ ] **Step 1: Write the failing tests**

Replace `backend/tests/test_auth_yaml.py` with:

```python
import backend.app.core.auth as auth
from backend.app.models.user import UserRole


def test_users_yaml_roundtrip(tmp_path, monkeypatch):
    monkeypatch.setattr(auth, "_user_file", tmp_path / "users.yaml")
    auth._users = {}
    created = auth.create_user("alice", "pw123", UserRole.admin, "Alice")
    assert created is not None

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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest backend/tests/test_auth_yaml.py -v`
Expected: 4 FAIL (`AttributeError: ... reset_admin_store` and YAML round-trip failures, since storage is still JSON).

- [ ] **Step 3: Implement YAML storage + reset_admin_store**

In `backend/app/core/auth.py`:

Add the import at the top (after `import time` on line 9, alongside other stdlib imports):

```python
import yaml
```

Replace `_load_users` (lines 26–34):

```python
def _load_users() -> dict[str, User]:
    if not _user_file.exists():
        return {}
    try:
        data = yaml.safe_load(_user_file.read_text(encoding="utf-8"))
        if not data:
            return {}
        return {u: User.from_dict(d) for u, d in data.items()}
    except (yaml.YAMLError, OSError, KeyError) as e:
        logger.error("Failed to load users: %s", e)
        return {}
```

Replace `_save_users` (lines 37–40):

```python
def _save_users() -> None:
    _user_file.parent.mkdir(parents=True, exist_ok=True)
    data = {u: user.to_dict() | {"password_hash": user.password_hash} for u, user in _users.items()}
    _user_file.write_text(yaml.safe_dump(data, default_flow_style=False, sort_keys=False), encoding="utf-8")
```

Change the `_user_file` filename (line 22) to YAML:

```python
_user_file: Path = Path(settings.data_dir) / "users.yaml"
```

Add `reset_admin_store` after `_save_users`:

```python
def reset_admin_store() -> None:
    global _users
    _users = {}
    try:
        if _user_file.exists():
            _user_file.unlink()
    except OSError as e:
        logger.error("Failed to remove user store: %s", e)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest backend/tests/test_auth_yaml.py -v`
Expected: 4 PASS.

- [ ] **Step 5: Lint**

Run: `uv run ruff check backend/app/core/auth.py`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add backend/app/core/auth.py backend/tests/test_auth_yaml.py
git commit -m "refactor: store users as YAML with admin reset helper"
```

---

### Task 4: Convert console lock state to YAML

**Covers:** [S4] (server_manager.py row), [S6]

The lock state is write-only fire-and-forget (no read-back exists), so this is a format change to the writer only.

**Files:**
- Modify: `backend/app/services/server_manager.py:50,329-335`

- [ ] **Step 1: Change the lock filename**

In `backend/app/services/server_manager.py`, line 50 (set in Task 2):

```python
        self._lock_file = self._data_dir / "console_lock_state.yaml"
```

- [ ] **Step 2: Rewrite _write_lock_state to emit YAML**

Add the import near the top (after `import time`, line 9):

```python
import yaml
```

Replace `_write_lock_state` (lines 329–335):

```python
    def _write_lock_state(self, state: str) -> None:
        try:
            payload = yaml.safe_dump(
                {"console": {"state": state, "timestamp": int(time.time())}},
                default_flow_style=False,
                sort_keys=False,
            )
            self._lock_file.write_text(payload)
        except OSError as e:
            logger.error("Failed to write lock state: %s", e)
```

- [ ] **Step 3: Verify import + lint**

Run: `uv run python -c "import backend.app.services.server_manager; print('ok')"`
Expected: prints `ok`.

Run: `uv run ruff check backend/app/services/server_manager.py`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add backend/app/services/server_manager.py
git commit -m "refactor: write console lock state as YAML"
```

---

### Task 5: Remove the dead inieditor router

**Covers:** [S4] (inieditor.py row)

The router is mounted at `/files` but has zero frontend callers (verified — no `inieditor`/`/files` usage in `frontend/src`).

**Files:**
- Delete: `backend/app/routers/inieditor.py`
- Modify: `backend/app/main.py:11-26`

- [ ] **Step 1: Delete the router file**

```bash
git rm backend/app/routers/inieditor.py
```

- [ ] **Step 2: Remove its import + registration in main.py**

In `backend/app/main.py`, remove `inieditor,` from the routers import block (lines 14–26). The block becomes:

```python
from backend.app.routers import (
    addons,
    audit,
    auth,
    backups,
    console,
    performance,
    players,
    properties,
    server,
    worlds,
)
```

Remove the registration line `app.include_router(inieditor.router, prefix=prefix)` (line 78).

- [ ] **Step 3: Verify app imports**

Run: `uv run python -c "import backend.app.main; print('ok')"`
Expected: prints `ok`.

Run: `uv run ruff check backend/app/main.py`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add backend/app/main.py
git commit -m "refactor: remove unused generic file-editor router"
```

---

### Task 6: Add `--reset-admin` CLI recovery

**Covers:** [S5]

**Files:**
- Modify: `start.py:76-90`

- [ ] **Step 1: Add the flag + reset hook**

In `start.py`, add the argument inside `main()`'s parser (after `--no-reload`, line 80):

```python
    parser.add_argument("--reset-admin", action="store_true", help="Clear the admin user store so a fresh admin is generated on next startup")
```

Then add the reset action immediately after `args = parser.parse_args()` (line 81), before the `signal.signal` lines:

```python
    if args.reset_admin:
        from backend.app.core.auth import reset_admin_store

        reset_admin_store()
        print("[reset] Admin user store cleared. A new admin + password will print on startup.")
```

- [ ] **Step 2: Verify the flag parses**

Run: `uv run python start.py --help`
Expected: help text includes `--reset-admin`.

- [ ] **Step 3: Lint**

Run: `uv run ruff check start.py`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add start.py
git commit -m "feat: add start.py --reset-admin for lockout recovery"
```

---

### Task 7: Legacy cleanup, gitignore, and full verification

**Covers:** [S6], [S7]

**Files:**
- Modify: `.gitignore`
- Delete: `backend/ini/` (runtime state only)

- [ ] **Step 1: Update .gitignore**

In `.gitignore`, change the line `backend/ini/` to:

```
backend/data/
```

- [ ] **Step 2: Remove the legacy ini dir**

The old `backend/ini/` holds only runtime state (`users.json`, `console_lock_state.ini`) that is regenerated. Delete it:

```bash
rm -rf backend/ini
```

- [ ] **Step 3: Run the full test suite + lint**

Run: `uv run pytest backend/tests -v`
Expected: all PASS.

Run: `uv run ruff check backend start.py`
Expected: no errors.

- [ ] **Step 4: Start backend, confirm admin banner + files**

Run (in one terminal): `uv run python start.py --backend --no-reload`

Expected in output:
- `Default admin account created` banner with a `Username: admin` and a 16-hex-char `Password:`.
- `backend/data/` directory created with `users.yaml`.

Stop the server (Ctrl-C).

- [ ] **Step 5: Verify users.yaml content**

Run: `cat backend/data/users.yaml` (or Read tool)
Expected: YAML with an `admin:` key containing `password_hash`, `role: owner`, etc. (NOT plaintext password.)

- [ ] **Step 6: Verify lock-state writes as YAML**

Start the backend, log in via the frontend, toggle the console lock on then off, then:

Run: `cat backend/data/console_lock_state.yaml`
Expected: YAML with `console:` → `state:` and `timestamp:`.

Stop the server.

- [ ] **Step 7: Verify --reset-admin works**

Run: `uv run python start.py --reset-admin --backend --no-reload`
Expected: prints `[reset] Admin user store cleared.` then on startup reprints a fresh `Default admin account created` banner with a NEW password. Confirm `backend/data/users.yaml` was recreated.

Stop the server.

- [ ] **Step 8: Commit**

```bash
git add .gitignore
git commit -m "chore: gitignore backend/data and remove legacy ini dir"
```
