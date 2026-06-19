---
feature: ini-to-yaml-config
status: delivered
specs:
  - docs/compose/specs/2026-06-19-ini-to-yaml-config-design.md
plans:
  - docs/compose/plans/2026-06-19-ini-to-yaml-config.md
branch: refactor/ini-to-yaml
commits: 59d04ee..1d0bfd1
---

# Panel Config INI → YAML — Final Report

## What Was Built

The panel's own mutable state files were migrated from JSON/INI to YAML, and the storage directory was renamed from `backend/ini/` to `backend/data/`. Two files changed format: `users.json` → `users.yaml` (auth accounts) and `console_lock_state.ini` → `console_lock_state.yaml` (console lock state). All reads/writes now go through `yaml.safe_load` / `yaml.safe_dump`.

A latent lockout bug was also fixed: the default admin password is a one-time random print on first startup, stored only as a pbkdf2 hash. Once `users.yaml` exists the banner never reprints, so an admin who loses the password was permanently locked out. A `python start.py --reset-admin` CLI flag now clears the user store so the next startup regenerates a fresh admin and prints a new password.

The dead generic file-editor router (`inieditor.py`, mounted at `/files` with zero frontend callers) and a duplicate `Settings` class in `backend/app/core/__init__.py` were removed. The game-owned `bedrock_server/server.properties` was intentionally left in `.properties` format — the Minecraft Bedrock binary reads/writes it directly and cannot consume YAML.

## Architecture

State lives in `backend/data/` (gitignored), located via `settings.data_dir` in `backend/app/core/config.py` (renamed from `settings.ini_dir`).

- **Auth storage** — `backend/app/core/auth.py`: module-level `_user_file = data_dir / "users.yaml"`. `_load_users()` / `_save_users()` use `yaml.safe_dump(default_flow_style=False, sort_keys=False)`; each user serializes as `to_dict() | {"password_hash": ...}`. `init_users()` creates the admin only when the store is empty and prints the plaintext password once. `reset_admin_store()` clears `_users` and deletes the file (used by the CLI).
- **Console lock state** — `backend/app/services/server_manager.py`: `_write_lock_state()` is write-only fire-and-forget; it emits `{"console": {"state", "timestamp"}}` as YAML. Never read back.
- **Settings API** — `backend/app/routers/settings.py`: `AppSettingsResponse.data_dir` exposes the path (frontend does not consume path fields, so the rename was safe).
- **CLI recovery** — `start.py --reset-admin` calls `reset_admin_store()` before launching, so the server regenerates the admin on that startup.

### Design Decisions

- **Direct conversion, no abstraction layer.** `yaml.safe_load`/`safe_dump` inline where files were already read/written. A shared `YamlStore` helper would add structure for a single pair of callers — not worth it.
- **No data migration.** The old `users.json` admin password was unrecoverable, so the store is reset rather than converted. Lock state is ephemeral and write-only, so it is simply regenerated.
- **`server.properties` stays `.properties`.** It is owned by the game binary; converting it would break the server. Only panel-owned files moved.
- **pytest added as a dev dependency.** The project previously had no tests; focused tests were added for the auth persistence layer (the riskiest changed logic).

## Usage

- **Normal start:** `python start.py` — on a fresh `backend/data/` (no `users.yaml`), the admin banner prints with a 16-hex-char password. Subsequent starts load the existing admin silently.
- **Recover from lockout:** `python start.py --reset-admin` — clears the user store, then starts; a new admin + password prints on that startup.
- **Dependencies:** `pyyaml>=6.0` (runtime), `pytest>=8.0` (dev). `uv sync` installs both.

## Verification

- `uv run pytest backend/tests` → 4/4 pass (YAML round-trip, admin creation prints, no-reprint when users exist, reset deletes file).
- `uvx ruff check` → clean on all changed files (one pre-existing `E501` in an unrelated HTML literal in `main.py` predates this branch).
- Runtime: backend started on a test port → admin banner printed a fresh password; `backend/data/users.yaml` contained a pbkdf2 hash, not plaintext; toggling lock state wrote `console_lock_state.yaml` in YAML; `start.py --reset-admin` cleared the store and regenerated a new password with a new `created_at`.

## Journey Log

- [lesson] "Nothing happened on restart" was not a bug — `init_users()` only prints the password when the user store is empty; the existing `users.json` meant the banner was correctly suppressed. Symptom vs. root cause.
- [lesson] `backend/` is a namespace package (no `__init__.py`), so pytest needed `pythonpath = ["."]` in `[tool.pytest.ini_options]` to import `backend.app...`. Discovered when the first test collection failed.
- [lesson] Behavior-preserving refactor tests (round-trip) pass under both JSON and YAML; an explicit "file is block-YAML, not JSON" assertion was needed to make the format change a real red→green test.
- [note] A feature-parity audit vs. the old PySide6 app ran in parallel; its findings (addon enable/disable UI, command macros, server-settings form, properties auto-backup) are recorded in session notes as separate future work.

## Source Materials

| File | Role | Notes |
|------|------|-------|
| `docs/compose/specs/2026-06-19-ini-to-yaml-config-design.md` | Design | Approved as-written |
| `docs/compose/plans/2026-06-19-ini-to-yaml-config.md` | Implementation plan | All 7 tasks complete |
