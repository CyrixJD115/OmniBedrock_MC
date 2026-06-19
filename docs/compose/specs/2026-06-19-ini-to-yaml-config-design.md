# Panel Config: INI → YAML Restructure

**Status:** Approved
**Date:** 2026-06-19

## [S1] Problem

The panel stores its own mutable state in `backend/ini/` as a mix of JSON (`users.json`) and hand-rolled INI (`console_lock_state.ini`). The `ini` naming and mixed formats are inconsistent. Additionally, the default admin password is a one-time random print on first startup (`backend/app/core/auth.py:43-64`); once `users.json` exists the banner never reprints, and the password is stored only as a pbkdf2 hash — so an admin who loses it is permanently locked out (current situation).

A hard constraint: `bedrock_server/server.properties` is read/written by the Minecraft Bedrock server binary in `.properties` format and **cannot** become YAML. It is out of scope.

## [S2] Solution overview

Migrate panel-owned state files to YAML. Rename `backend/ini/` → `backend/data/` and the `settings.ini_dir` config → `settings.data_dir`. Remove dead code (an unused generic file-editor router and a duplicate `Settings` class). Resolve the lockout by resetting the admin store on next startup (existing one-time behavior) and adding a `--reset-admin` CLI flag for permanent recovery.

Approach chosen: **direct conversion, no new abstraction layers** — `yaml.safe_load`/`safe_dump` inline where the files are already read/written.

## [S3] Scope decisions

- In scope: `users.json` → `users.yaml`; `console_lock_state.ini` → `console_lock_state.yaml`; rename dir + setting; remove dead `inieditor` router; remove duplicate `Settings`; add `--reset-admin`; declare `pyyaml`.
- Out of scope: `server.properties` (game-owned), frontend feature additions, any new abstraction layer.

## [S4] File changes

| File | Change |
|------|--------|
| `backend/app/core/config.py` | Rename `ini_dir` → `data_dir`; path → `backend/data/` |
| `backend/app/core/__init__.py` | **Delete** the duplicate dead `Settings` class entirely (canonical one is `config.py`) |
| `backend/app/core/auth.py` | `_user_file` → `data/users.yaml`; `_load_users`/`_save_users` use `yaml.safe_load`/`yaml.safe_dump` |
| `backend/app/services/server_manager.py` | `_lock_file` → `data/console_lock_state.yaml`; `_write_lock_state` emits `yaml.safe_dump` |
| `backend/app/routers/settings.py` | response field `ini_dir` → `data_dir` |
| `backend/app/routers/inieditor.py` | **Delete** (no frontend usage) + remove its registration in `main.py` |
| `.gitignore` | `backend/ini/` → `backend/data/` |
| `pyproject.toml` | Add `pyyaml` to dependencies |

## [S5] Admin reset & CLI

- **Next-startup reset:** with no `users.yaml` present, `init_users()` regenerates the admin and prints the password (existing logic, unchanged).
- **`start.py --reset-admin`:** deletes `data/users.yaml` (and the legacy `ini/users.json` if present), prints a confirmation, then proceeds to start servers normally so the password prints on that startup. Permanent lockout recovery.

## [S6] Data migration

- `users.json` → **not migrated** (password unrecoverable; user chose reset). Deleted by `--reset-admin`.
- `console_lock_state.ini` → **not migrated** (write-only ephemeral state; server is stopped). Stale file may remain harmlessly.

## [S7] Verification

1. Add `pyyaml` dep; `uv sync`.
2. Start backend → confirm admin password banner prints.
3. Log in → confirm `data/users.yaml` written with hashed password.
4. Toggle console lock → confirm `data/console_lock_state.yaml` updates.
5. Run `start.py --reset-admin` → confirm wipe + fresh password on startup.
