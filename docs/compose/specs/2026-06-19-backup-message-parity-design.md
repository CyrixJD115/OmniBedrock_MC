# Backups Tab: Full PySide6 Parity (pre/post commands + player-message broadcast)

**Status:** Approved
**Date:** 2026-06-19
**Branch:** `feature/backup-message-parity`
**Stack:** FastAPI (Python) + SvelteKit (TypeScript + TailwindCSS) — extend in place
**Design reference:** the existing web panel's dark Minecraft theme (Tailwind + `app.css`)

## [S1] Problem

The web panel's `/backups` page is partial: it lists/creates/deletes/restores/downloads zips and toggles a fixed scheduler. It is missing the PySide6 `tabs/backups.py` centerpiece — the **pre/post command editor** (`ini/bakcmd.ini`) whose `send` directives broadcast `say <message>` to all in-game players before/after a backup, plus the manual/auto option UIs, include-picker, progress bar, and per-mode status logs that the desktop app has.

The existing `backup_service.create_backup` already accepts a plumbed-but-unused `progress: asyncio.Queue[str]` param (`backend/app/services/backup_service.py:53`) — a ready hook point. The existing `BackupScheduler` is double-instantiated (router singleton vs lifespan) and only reads `enabled`.

Goal: deliver full PySide6 Backups-tab feature parity in the web UI, with the pre/post command + player-message broadcast as the signature feature, on the current stack.

## [S2] Scope

**In scope** (this cycle):
- Pre/post command editor with 4 entry types (shell command / wait / comment / send-to-console).
- Player-message broadcast: `send` entries execute `server_manager.send_command(value)` against the live Endstone server (e.g. `say Backup starting…`).
- Manual backup options: world, full/partial, zip prefix, export folder, compression (deflate/store), dry-run, include-picker.
- Automatic backup options: enable, interval (minutes), keep count, export folder, compression, full/partial, include-picker; Start/Stop.
- Backups list with progress bar (0–100) during active jobs; download/delete/restore(trash)/undo (existing).
- Per-mode status logs (Manual / Automatic) + a full Logs view; auto-scroll; cap 5000 lines; Clear.
- Config persistence in one structured YAML file.
- Real-time job event streaming over a JWT-authed WebSocket.
- Backup audit logging (fixes the partial-audit gap for backups).

**Out of scope** (later cycles): addon JSON viewer, console command-history, initialization-files editor, settings/user-management UI, properties backup-catalog, performance graphs, the other PySide6 tabs. Cross-cutting fixes unrelated to backups (e.g. console-WS auth, orphaned `/logs/ws`) are deferred except for the new backup WS which will be JWT-authed by construction.

## [S3] PySide6 feature coverage (parity checklist)

Source: `tabs/backups.py` (1175 lines), `src/backup_worker.py`, `ini/bakcmd.ini`, `ini/baksettings.json`. Every feature below must be covered.

**Pre/post command editor (the signature feature)**
- Two ordered lists: `==before` (pre) and `==after` (post), drag-to-reorder.
- Four entry types via Add/Edit dialog: **Command** (shell), **Wait (seconds)** 1–600, **Comment** (ignored at exec), **Send to console** (emits the exact console string, e.g. `say …`).
- Per-list actions: Add, Edit, Delete, **Send Test** (run one entry immediately); same via right-click context menu.
- Send Test result display: exit code + truncated output for commands; immediate console dispatch for sends; info text for wait/comment.

**Execution semantics (`src/backup_worker.py`)**
- Order: pre-commands → ZIP creation → post-commands.
- `# comment` → ignored silently.
- `--<N>` wait → sleep N seconds with periodic "waiting" status.
- send directive → notify (forwarded to live server stdin → in-game broadcast).
- shell command → `subprocess` with **300s timeout**, streamed output lines.
- Dry-run: ZIP not written; post shell/comment/wait report "would run"; **send directives still fire**.

**Manual sub-tab**
- World dropdown; Create Backup Now; Select include items… (picker).
- Options: Full backup checkbox; Zip prefix (default `manual_backup`); Export folder base + Browse; Compression (deflate/store); Dry-run checkbox; Edit pre/post commands; Save Manual Settings.
- Compact status log + Clear.

**Automatic sub-tab**
- Enable checkbox; Interval (minutes) 1–2592000; Keep count 1–999; Export folder + Browse; Compression; Select include items; Start/Stop toggle; Save Auto Settings; Full backup checkbox.
- Compact status log + Clear.

**Backups sub-tab**
- Table: Filename, Size, Modified (most-recent first).
- Progress bar 0–100 during a job.
- Open Backups Folder (desktop-only — see [S10]).

**Logs sub-tab**
- Full read-only log; cap 5000 lines; auto-scroll.

**Include picker (`IncludePickerDialog`)**
- Candidate items: top-level world-dir entries + standard extras (`behavior_packs`, `resource_packs`, `db`, `level.dat`, `levelname.txt`, `world_behavior_packs.json`, `world_resource_packs.json`).
- Select All / Clear All / OK / Cancel. Persisted per mode.

**Backup file naming**: `{world}_{tag}_{YYYY-MM-DD-HHMMSS}.zip` (current web service already matches).

## [S4] Architecture (Approach A: backup WebSocket + async job runner)

A single backup job runs at a time (PySide6 also serialized). The pipeline executes as an asyncio task and emits structured events to a JWT-authed WebSocket:

```
client ──POST /backups/create──▶ router ──▶ BackupService.run_backup(job, notify)
                                              │
                       pre-cmds ─┐            │ events {progress,output,status,...}
                       zip       ├─ async ────┤
                       post-cmds ┘            │
                                              ▼
                          BackupWebSocket (/api/v1/backups/ws, JWT) ──▶ subscribed /backups clients
```

- `notify(event: dict)` fans events to all WS subscribers (pub/sub, mirroring `server_manager.subscribe_stdout`).
- Shell commands via `asyncio.create_subprocess_shell` (non-blocking; 300s timeout; stream stdout/stderr lines as `output` events).
- `send` entries call `server_manager.send_command(value)` — the existing stdin writer that the console uses. The Endstone `say` command broadcasts to all players.
- One job at a time: a module-level lock/ref; a second create request while running returns 409.

Why this approach: real-time UX matching PySide6's 200ms notify-queue drain; backup output stays separate from the console stream; the project already uses WS (console, metrics) so the pattern is established; and the new WS is JWT-authed by construction, fixing the WS-auth gap for this endpoint.

## [S5] Config model

New file `backend/data/backup_settings.yaml` (the panel's YAML data dir; gitignored — locate via `ls backend/data/`, glob misses it). Consolidates PySide6's `baksettings.json` + `bakcmd.ini`. Loaded/saved with `yaml.safe_load`/`safe_dump` (project convention — no new abstraction layer).

```yaml
manual:
  world: ""                 # last-selected world (UI convenience only)
  full_backup: true
  zip_prefix: "manual_backup"
  export_folder: ""         # "" = default backups/worlds/<world>/
  compression: "deflate"    # deflate | store
  dry_run: false
  include_items: []         # [] = worker defaults (see [S3] include picker)
auto:
  enabled: false
  interval_minutes: 30
  keep_count: 10
  export_folder: ""
  compression: "deflate"
  full_backup: true
  include_items: []
pre_post:
  before: []                # list of {type, value}
  after: []                 # list of {type, value}
```

**Command entry shape**: `{type: "command"|"wait"|"comment"|"send", value: <str|int>}`.
- `command` → `value` is the shell string.
- `wait` → `value` is an int 1–600.
- `comment` → `value` is documentation text (ignored at exec).
- `send` → `value` is the exact console command (e.g. `say Backup starting…`).

The PySide6 directive aliases (`>`, `>>`, `!`, `send:`, `send `) all meant "send this string to the console"; the web model standardizes on the `send` type with the literal command as `value`. A one-time `bakcmd.ini` importer is included as a convenience for carry-over desktop configs; the canonical store is this YAML.

Defaults seed an empty file on first read (sensible defaults above).

## [S6] Backend services

| File | Change |
|------|--------|
| `backend/app/services/backup_settings_service.py` | **New.** Load/save `backend/data/backup_settings.yaml`; typed accessors for manual/auto/pre_post. Seeds defaults if absent. |
| `backend/app/services/backup_service.py` | Add `run_backup(world, tag, options, pre_post, notify)`: orchestrates pre → zip → post, calling `notify(event)` per step. Reuses existing zip logic (`create_backup` body). New `_run_command_entry(entry, notify, phase, dry_run)` dispatcher. Keeps existing list/create/delete/restore/trash/download methods. |
| `backend/app/managers/backup_scheduler.py` | Fix double-instantiation: make it a single lifespan-started singleton (started in `main.py` lifespan alongside `PerformanceCollector`). Read `auto` + `pre_post` from `backup_settings.yaml` each tick; call `BackupService.run_backup(...)` with hooks; keep existing interval/keep-cleanup. |
| `backend/app/core/dependencies.py` | Provide the shared `backup_service` + `backup_settings_service` + `backup_scheduler` singletons (the scheduler currently re-instantiated per-router is removed). |

**`run_backup` event emission** (each calls `notify`):
- `status {phase: "pre"|"zip"|"post"|"done", message}`
- `output {stream: "pre"|"zip"|"post", line}` (shell stdout/stderr lines)
- `progress {percent: 0–100, message}` (coarse: e.g. 10 pre-start, ramp during zip, 90 post-start, 100 done)
- `done {success: bool, filename: str|None, message}`
- `error {message}` (on exceptions / shell non-zero not fatal)

**Dispatcher per entry type** (`_run_command_entry`):
- `comment` → emit one `output` line `# <value>` (documentation), no execution.
- `wait` → `await asyncio.sleep(int(value))`; emit `status {phase, message: "Waiting <N>s …"}` once, then proceed.
- `send` → `server_manager.send_command(str(value))`; emit `output {stream: phase, line: "[send] <value>"}`. Fires even in dry-run.
- `command` → `proc = asyncio.create_subprocess_shell(value, stdout=PIPE, stderr=STDOUT)`; stream lines as `output`; `await asyncio.wait_for(proc.wait(), timeout=300)`; on timeout → kill + `error` event; emit exit code. In dry-run → emit `output "Would run (dry-run): <value>"` instead of executing.

## [S7] Backend WebSocket + API endpoints

**New WebSocket** — `backend/app/websocket/backup.py`, `BackupWebSocket` at `/api/v1/backups/ws`, registered in `routers/backups.py`.
- **JWT-authed** (verify token from query param `?token=` or subprotocol; reject 401 if missing/invalid) — fixes the WS-auth gap for this endpoint by construction.
- On connect: subscribe to the backup event fan-out; if a job is active, replay its current phase/percent.
- Streams events as JSON (shapes in [S6]).

**Router `backend/app/routers/backups.py`** — all `verify_token`; settings/run/test gated `require_role(admin, owner)`:

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/backups/settings` | Full `backup_settings.yaml` (manual+auto+pre_post) |
| PUT | `/backups/settings` | Save manual and/or auto and/or pre_post (admin/owner); audit `backup.settings_update` |
| POST | `/backups/create` | **Extended**: accepts full options object `{world, tag, full_backup, zip_prefix, export_folder, compression, dry_run, include_items, run_hooks: bool}`. Runs the hooked pipeline via `run_backup`; events → WS. Returns `{job_id}`. 409 if a job is already running. Audit `backup.run`. |
| POST | `/backups/test-command` | Execute one entry immediately (the "Send Test"); returns `{kind, output, exit_code}`. Audit `backup.test_command`. |
| GET | `/backups/include-items?world=` | Candidate items for the picker (top-level world entries + standard extras) |
| GET | `/backups/scheduler` | **Extended**: full `auto` section (not just `enabled`) |
| PUT | `/backups/scheduler` | **Extended**: write `auto` section; start/stop the scheduler; audit `backup.scheduler_update` |
| *(kept)* | `/backups/worlds`, `/backups/`, `/backups/restore/{world}/{filename}`, `/backups/trash`, `DELETE /backups/{world}/{filename}`, `/backups/{world}/{filename}/download` | Unchanged |

Audit logging uses the existing `audit_service.log_action` (currently only called from `server.py`).

## [S8] Command execution security

The `command` type runs arbitrary shell on the server host — full PySide6 parity, but a real consideration in a web context. Handling:
- All pre/post + send-test endpoints gated behind `require_role(admin, owner)`.
- Frontend shows a confirmation dialog **listing every shell command** before starting a backup that includes any `command` entries.
- Every shell execution is audit-logged (`backup.run` / `backup.test_command`) with the command string + exit code.
- Net: safer than the desktop app (which ran untrusted as the local OS user) because the web panel adds auth + RBAC + audit, while keeping full feature parity.
- `send`/`wait`/`comment` need no special gating beyond the admin role on the settings endpoints.

## [S9] Scheduler integration

- One `BackupScheduler` singleton, started in `main.py` app lifespan (next to `PerformanceCollector`); removed from per-router instantiation.
- Each tick: read `auto` from `backup_settings.yaml`; if `enabled`, for each configured world (or all worlds if none configured) call `BackupService.run_backup(world, tag="auto", options=<from auto>, pre_post=<from yaml>, notify=<fan-out>)`; then existing `_cleanup_old` keeps `keep_count` newest per world.
- Interval is `auto.interval_minutes`; respects Start/Stop via `PUT /backups/scheduler`.
- Scheduler-emitted events flow to the same WS subscribers and appear in the Automatic status log + Logs view.

## [S10] Frontend — `/backups` restructure

`frontend/src/routes/backups/+page.svelte` restructured into 4 sub-tabs matching PySide6. Extends the existing dark theme (Tailwind + `app.css` component classes; neon accents). New components under `frontend/src/lib/components/backups/`.

- **Manual**: world `<select>`; Create Backup Now; Select include items… → `IncludePicker.svelte`; options card (full / zip prefix / export folder + folder-browser / compression / dry-run); Edit pre/post commands → `CommandEditor.svelte`; Save Manual Settings. Compact status log + Clear. Create Backup Now shows the **shell-command confirmation dialog** (see [S8]) listing every `command` entry before running if any are present.
- **Automatic**: enable; interval (min); keep count; export folder; compression; include picker; Start/Stop toggle; Save Auto Settings; full backup. Status log + Clear.
- **Backups**: existing table (Filename/Size/Modified) + Download/Delete/Undo; **progress bar 0–100** bound to WS `progress` events (shown during active job); refresh on `done`.
- **Logs**: full read-only auto-scroll log (cap 5000 lines) fed by WS `output`/`status`/`error` events; Clear.

**`CommandEditor.svelte` (centerpiece modal):**
- Two drag-reorder lists (Before/After). Drag via HTML5 DnD (no new dep) or `svelte-dnd-action` only if already available — prefer zero-dep HTML5 DnD.
- Per-list buttons: Add, Edit, Delete, **Send Test**; identical right-click context menu.
- `CommandEntryEditor.svelte` sub-modal: Type `<select>` (Command/Wait/Comment/Send) + typed value field (text for command/comment/send; number 1–600 for wait). Per-type validation.
- Send Test → `POST /backups/test-command`; render result inline (exit code + output / dispatched / info).
- Save → `PUT /backups/settings` (pre_post section).

**`IncludePicker.svelte` modal:** candidate items from `GET /backups/include-items`; checkboxes; Select All / Clear All; OK/Cancel; writes to the mode's `include_items`.

**Folder browser:** native OS dir pickers don't exist in browsers. "Browse" opens a small server-side directory browser modal (lists dirs under the backups root / a safe base) OR is a plain text input for an absolute/relative path. Decision: **text input + a lightweight directory-browser modal** reusing a new `GET /backups/folders?base=` endpoint scoped under the configured backups root (path traversal guarded).

**Backup WS client:** a small `frontend/src/lib/api/backupStream.ts` (mirrors the existing `websocket.ts` pattern) connecting to `/api/v1/backups/ws?token=<jwt>`; a `backupEvents` Svelte store consumed by the progress bar + status logs + Logs view. Subscribed when on `/backups`, disconnected on leave/logout (mirrors console/metrics WS lifecycle in `+layout.svelte`).

**API client additions** (`frontend/src/lib/api/client.ts`): `getBackupSettings`, `updateBackupSettings`, `runBackup` (replaces `createBackup`), `testBackupCommand`, `getIncludeItems`, `getSchedulerConfig`/`updateScheduler` (already present — extend for new fields), `listBackupFolders`.

## [S11] Desktop → web adaptations

| Desktop concept | Web adaptation | Feature dropped? |
|-----------------|----------------|-----------------|
| Native OS directory picker (Browse) | Server-side folder-browser modal + text input | No |
| "Open Backups Folder" (`xdg-open`/`open`/`startfile`) | Show + copy server path | Functionality (host FS open) N/A in browser; path is surfaced |
| `bakcmd.ini` directive aliases (`>`,`>>`,`!`,`send:`,`send `) | Standardized `send` type; importer handles old files | No (semantics preserved) |
| QSS slant aesthetic, ambient audio, tray | Existing web theme; N/A | Not backup features |

No backup feature is dropped.

## [S12] Testing & verification

**Backend pytest** (`backend/tests/`, run `uv run pytest`):
- `backup_settings_service` YAML round-trip (manual/auto/pre_post); defaults seeded on empty; assert YAML not JSON (`assert not raw.lstrip().startswith("{")`).
- `run_backup` ordering: pre → zip → post (mock the zip + each entry; assert call order).
- Dispatcher: `send` → `server_manager.send_command` called with value; `wait` → `asyncio.sleep` called; `comment` → no exec; `command` → subprocess invoked (mock) with 300s timeout; dry-run skips zip + shell but **send still fires**.
- Scheduler reads pre_post + auto; calls `run_backup` with hooks.
- `BackupWebSocket` rejects missing/invalid JWT.
- One-job-at-a-time: second `create` while running → 409.
- Folder-browser path-traversal guard (`..` rejected).

**Lint/typecheck:**
- `uvx ruff check` (ruff not in venv; ephemeral via uvx; do NOT add as dep).
- `npm run check` (svelte-check; frontend has no test script).

**Runtime verification** (start panel via `uv run start.py`, OMNI_PORT default):
1. Add a `send` pre-command `say Backup starting…` and post-command `say Backup done`.
2. With the Endstone server running, run a manual backup; confirm both `say` messages broadcast in-game + appear in console, progress bar advances, logs populate, zip created.
3. Test each entry type via Send Test (shell echo, wait, comment, send).
4. Enable auto backups with a short interval; confirm an auto run fires with hooks + cleanup keeps `keep_count`.
5. Dry-run: no zip written; sends still fire.

## [S13] Open questions / risks

- **One job at a time**: enforced server-side. The UI should disable "Create Backup Now" while a job is active (driven by WS status).
- **Progress percent granularity**: zip progress is coarse (single executor call). Percent will step (e.g. 10 → "zipping" → 90) rather than smooth. Acceptable; matches PySide6 which also had a coarse bar.
- **Shell timeout 300s**: long-running legitimate commands (large sync) could exceed it. Documented; surfaced as an `error` event on timeout. Configurable later if needed (not this cycle).
- **WS token transport**: query-param `?token=` is simplest for browser WS; subprotocol is the alternative. Query-param tokens can leak in server logs — mitigate by logging URL without query (FastAPI access log config). Acceptable for this panel's threat model.
