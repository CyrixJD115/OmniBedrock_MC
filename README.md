<h1 align="center">OmniBedrock</h1>
<p align="center"><img src="assets/OmniBedrock_Icon.png" alt="OmniBedrock" width="250"></p>

Minecraft Bedrock / Endstone control panel with a native Rust backend and a SvelteKit, TypeScript, and Tailwind frontend. The panel runs as one binary; the game server remains a separate Endstone process.

## Features

- Dual game backends: Endstone (Python plugin runtime) or vanilla Bedrock Dedicated Server, with in-panel version installs.
- Start, stop, restart, and kill; crash budgets, automatic restart, persisted grace periods.
- Interactive console with structured log levels, error aggregation, bounded history, disk replay, and reconnect cursors.
- Players: list, kick, ban, pardon, op, and deop.
- Visual and raw `server.properties` editing with validation and comment preservation.
- Manual and scheduled ZIP backups, selection of included files, pre/post hooks, progress WebSocket, downloads, trash, and world restore.
- World browser, behavior/resource pack manifests, rename, UUID editing, and pack ordering.
- Config/INI file editing, JWT accounts, custom roles/permissions, and audit logs.
- CPU/RAM/TPS metrics over WebSocket, including Endstone's child processes.
- MCXboxBroadcast integration: standalone broadcaster as its own Java process with live logs, Microsoft sign-in code capture, multi-account management, config.yml editing, and screenshot upload.

## Native quick start

The tested deployment target is Linux x86-64. Install Rust 1.90+ and Node.js 22+, and provide an existing Endstone executable. Endstone's [installation instructions](https://endstone.dev/v0.11/getting-started/installation/) explain its game runtime requirements. The panel itself does not require Python. A pip-installed Endstone launcher and Python plugins still use their own Python environment.

```bash
cargo build --release --locked
cp .env.example .env
# Edit .env: BEDROCK_SERVER_DIR and ENDSTONE_BIN should point to your server/runtime.
./target/release/omnibedrock
```

The release build embeds the web UI (the build script runs `npm run build` in `frontend/` automatically when needed), so **one binary serves the whole panel**: open `http://localhost:17754` for the UI, with the API under `/api/v1` on the same port. No separate frontend process is required. Set `OMNI_SKIP_FRONTEND_BUILD=1` while building to never invoke npm.

A desktop window mode is available too: rebuild with `cargo build --release --locked --features desktop` (Linux needs the `webkit2gtk-4.1` library installed) and run `./target/release/omnibedrock --desktop`. The same single-process server starts and the panel opens in a native WebKit window; closing the window shuts everything down cleanly. Without the feature the flag prints a clear rebuild hint.

When there are no users, startup creates an `Admin` account with owner role using the default password `123456`, prints it once to stderr, and warns you to change it (the login page also shows a hint and prefills while the default is active). Set `ADMIN_PASSWORD` to use a supplied initial password (6+ characters) instead; changing the password anywhere flips the default-credentials flag off. Leave `SECRET_KEY` blank to generate a private persisted key, or supply a random secret of at least 32 characters. Bootstrap credentials are not written to the backend log.

Login can be turned off entirely from Settings → Access (or `PUT /api/v1/settings/panel`): with login disabled the panel serves every request — REST and WebSockets — as the owner account, and the login page redirects straight in. The choice persists in `data/panel_settings.yaml` and survives restarts. Startup logs each phase (directories, auth store, services) with elapsed milliseconds so slow boots are visible.

```bash
cd frontend
npm ci
cp vite.config.example.ts vite.config.ts
npm run dev
```

Open **http://localhost:17755**. The API listens on **17754**, and Vite forwards `/api`, including WebSockets, to it. `bash scripts/dev.sh` launches both processes after dependencies are installed. Old `start.py` / `dev.py` remain convenience wrappers and now launch the Rust backend. (The Vite dev server is optional: the release binary serves the UI itself on 17754, and `npm run dev` is only needed for frontend hot reload.)

For production, the single binary needs no reverse proxy for the UI. If you prefer splitting them, run `ADAPTER=node npm run build` and `HOST=127.0.0.1 PORT=17755 node build` in `frontend`, then put the frontend and API behind one reverse proxy. `deploy/nginx.conf` includes the required WebSocket headers; change the Docker service names to your native upstream addresses. Move the frontend to an internal port if your proxy owns 17755. Use HTTPS at the proxy when accessing the panel remotely. `deploy/omnibedrock.service` is a systemd example; adjust its user, paths, and stop timeout for your hooks.

`bash scripts/package.sh` creates `dist/omnibedrock-linux.tar.gz` with the release binary (UI embedded) and deployment files; `FEATURES=desktop bash scripts/package.sh` builds in the desktop window mode. The default binary dynamically links the platform C runtime; use a matching Linux distribution. Static builds can be made with an installed musl Rust target/toolchain and `cargo build --release --locked --target x86_64-unknown-linux-musl`.

## Docker

```bash
cp .env.example .env
# Set ADMIN_PASSWORD if desired. Blank values generate credentials/key on first boot.
docker compose up --build -d
docker compose logs backend
```

Open **http://localhost:17755**. Compose builds the Rust API and Node frontend, and uses Nginx for a same-origin HTTP/WebSocket gateway. Bedrock UDP ports 19132 and 19133 are published. Persistent named volumes hold the server, panel accounts, backups, logs, and the broadcaster home. The API port is bound to loopback. Set `FRONTEND_ORIGIN` and `CORS_ORIGINS` to your public URL when deploying behind TLS.

The multi-stage root Dockerfile has two runtime targets:

- `runtime`: binary and C runtime, for an externally provided standalone Endstone executable.
- `endstone` (default): includes Endstone 0.11.0 in its own Python environment plus a headless JRE for the optional MCXboxBroadcast integration — drop `MCXboxBroadcastStandalone.jar` into the `broadcaster` volume. Compose pins this version; set `ENDSTONE_VERSION` deliberately to upgrade. Endstone may download Bedrock on first server start; alternatively install vanilla BDS from Settings → Server backend.

```bash
docker build --target runtime -t omnibedrock-api .
docker build --target endstone --build-arg ENDSTONE_VERSION=0.11.0 -t omnibedrock .
```

The Endstone image targets `linux/amd64`. Named volumes are initialized for UID 1000; if replacing them with bind mounts, give that UID write access. The health check tests the panel API, so the panel remains healthy while the game server is stopped.

## Configuration

Precedence (lowest to highest): **defaults → portable layout → config file → environment / `.env` → CLI flags**. Relative paths resolve from the working directory. See [`deploy/config.example.toml`](deploy/config.example.toml) for a fully commented sample.

### Config files

- `--config <file>` — explicit path; `.toml` (preferred) or legacy `.yaml`/`.yml`. The file must exist.
- Otherwise auto-discovered: `<executable dir>/config.toml` first, then `~/.config/omnibedrock/config.toml` on Linux (XDG_CONFIG_HOME honored). Windows/macOS use the executable-side file only.
- Only one file is ever read — discovered files are never merged.
- Unknown keys are rejected (typo protection). Secrets (`SECRET_KEY`, `CURSEFORGE_API_KEY`, `ADMIN_PASSWORD`) are never read from config files — keep them in the environment or `.env`.

### Portable mode

`omnibedrock --portable` keeps everything the panel manages under `<executable dir>/portable/`:

```
portable/
├── config.toml   # optional; the ONLY config file read in portable mode
├── data/         # accounts, settings, toolchains, managed cache
├── server/       # game server + worlds
├── logs/         # panel + audit logs
└── backups/
```

Explicit `--data-dir`/`--bedrock-server-dir`/`--logs-dir`/`--backups-dir` (or their env vars) still override individual locations, and external drives/paths can be mixed in deliberately. Limitations: the directory must be writable, and network mounts need to allow the executable's file locking. System-wide and portable state are never mixed — portable mode ignores the other config locations.

### Paths

| Path | Default | Controls |
| --- | --- | --- |
| `BEDROCK_SERVER_DIR` / `--bedrock-server-dir` | `bedrock_server` | Game server root (binary, `worlds/`, properties) |
| `DATA_DIR` / `--data-dir` | `backend/data` | Accounts, roles, panel settings, signing key, uv/Python/Endstone toolchains |
| `LOGS_DIR` / `--logs-dir` | `backend/logs` | Audit log, tracing, console replay spool |
| `BACKUPS_DIR` / `--backups-dir` | `backend/backups` | World backups |
| `CACHE_DIR` / `--cache-dir` | `<DATA_DIR>/cache` | Download caches (CurseForge images); follows DATA_DIR unless set |

Paths containing spaces are fully supported; the Endstone toolchain relocates to `~/.local/state/omnibedrock/` automatically because the Endstone launcher's `LD_PRELOAD`/`LD_LIBRARY_PATH` cannot contain spaces (see startup log for the resolved location). Temporary files use the OS temp dir (`TMPDIR`).

### Other settings

| Variable | Default | Purpose |
| --- | --- | --- |
| `OMNI_HOST`, `OMNI_PORT` | `0.0.0.0`, `17754` | API binding |
| `ENDSTONE_BIN` | managed toolchain → PATH | Explicit endstone executable |
| `ENDSTONE_PYTHON` | unset | Legacy `-m endstone` fallback; probed with `import endstone` before use |
| `BROADCASTER_DIR` | `backend/broadcaster` | MCXboxBroadcast home: config.yml, screenshot, cache, jar |
| `BROADCASTER_JAR` | `$BROADCASTER_DIR/MCXboxBroadcastStandalone.jar` | Explicit jar path |
| `JAVA_BIN` | `java` on PATH | Java runtime for the broadcaster |
| `SECRET_KEY` | generated/persisted | SHA256-derived HS256 JWT signing key |
| `ADMIN_PASSWORD` | `123456` | Initial account password only |
| `CORS_ORIGINS` | localhost ports 17755/5173/3000 | Comma-separated list or JSON array |
| `CONSOLE_MAX_LINES` | `2000` | RAM history count |
| `CONSOLE_MAX_BYTES` | `16777216` | RAM history text budget |
| `CONSOLE_MAX_LINE_BYTES` | `65536` | Oversized line chunk size |
| `CONSOLE_JOURNAL_BYTES` | `268435456` | Disk replay budget; minimum 4 MiB |
| `COMMAND_QUEUE_CAPACITY` | `256` | Bounded stdin queue |
| `WS_HEARTBEAT_INTERVAL` | `30` | Heartbeat and permission revalidation interval |
| `DEBUG`, `RUST_LOG` | `false`, info | Tracing filter; HTTP query tokens are excluded |

The startup log reports the resolved configuration: portable flag, config source, every effective path, and the detected Endstone/BDS toolchain versions — facts read from the on-disk markers, not assumptions. Example:

```bash
# System install with a config file next to the binary
omnibedrock --config /etc/omnibedrock/config.toml

# Fully self-contained tree next to the executable
omnibedrock --portable
```

Upgrading is safe: the panel reuses valid installations (re-running "Install Endstone toolchain" without a version pin is a no-op when the toolchain is present), and BDS/toolchain upgrades never touch worlds, `server.properties`, or the allowlist.

Auto-restart and stop/kill timeouts are persisted in `server_settings.yaml` through `/api/v1/settings/server`. Backup configuration lives in `backup_settings.yaml`. The scheduler applies configuration changes immediately and waits the configured interval before running. Server shutdown cancels scheduling, lets active jobs clean up save state, stops the game, and drains output. Ensure service/container stop timeouts cover your configured hooks and grace periods.

## Migrating the Python installation

The Python source is retained in `backend-py/` as a reference; the Rust backend does not import it or run FastAPI. Stop the old backend before starting Rust. Back up its data first.

1. Point `BEDROCK_SERVER_DIR`, `DATA_DIR`, `BACKUPS_DIR`, and `LOGS_DIR` to your existing installation, or copy its stores into the corresponding Rust directories. The directories normally came from `backend/data`, `backend/backups`, and `backend/logs`; this checkout moved them to `backend-py/`.
2. Preserve `users.yaml`, `roles.yaml`, `backup_settings.yaml`, and `audit.jsonl`. Existing Python PBKDF2 hashes are verified and upgraded to Argon2id after a successful login. Malformed stores fail startup rather than silently replacing accounts.
3. Preserve the existing `SECRET_KEY` (at least 32 characters) to retain issued JWTs; otherwise users log in again. Tokens retain the Python SHA256-derived HS256 format and seven-day lifetime.
4. Configure the Endstone executable from the existing environment, such as `ENDSTONE_BIN=/opt/omnibedrock/.venv/bin/endstone`.

The REST paths and principal payload shapes are preserved. Python settings that were only held in memory begin with their previous defaults. Old exported archives remain in place; the backup catalog covers `BACKUPS_DIR/worlds/<world>/*.zip`. Custom export directories are supported within `BACKUPS_DIR`; archives there can be retrieved directly from that directory.

## Dual backend: Endstone or vanilla BDS

The panel supervises either an Endstone launcher or the official `bedrock_server` binary in the same server directory. The active backend, and the vanilla version pin, live in `server_settings.yaml` and are editable from Settings → Server backend. Switching stops the game if it is running, applies the selection, and restarts on the new backend; `worlds/`, `server.properties`, `allowlist.json`, and `permissions.json` are shared between both backends and never touched by a switch.

Vanilla installs are managed in the panel: the version registry and SHA256 checksums come from [EndstoneMC/bedrock-server-data](https://github.com/EndstoneMC/bedrock-server-data) (MIT), and the archives themselves download from the official `minecraft.net` CDN URLs that registry provides — the panel never executes an archive whose hash does not match. Pick `latest` to track the newest release or pin any registry version (release or preview). Installing extracts to a staging copy first, refuses symlink entries, replaces upstream files, preserves your user data, marks the binary executable, and records the applied version in `data/bds_version.json`. Install progress streams both into the game console and the backend status endpoint. Vanilla BDS accepts the same console commands, backups, and properties tooling as Endstone; TPS is only reported when the backend prints it.

## MCXboxBroadcast integration

The optional [MCXboxBroadcast](https://github.com/MCXboxBroadcast/Broadcaster) standalone runs as its own supervised Java process, independent of the game server (own start/stop/restart, own log ring, killed cleanly on panel shutdown). Its home is `BROADCASTER_DIR` (default `backend/broadcaster`), which holds `config.yml`, `screenshot.jpg`, the auth `cache/`, and by default `MCXboxBroadcastStandalone.jar` itself — drop the jar there or point `BROADCASTER_JAR` at it, ensure `java` is on `PATH` (or set `JAVA_BIN`), then use the Broadcaster page. The panel seeds the documented default `config.yml` on first read and validates YAML before saving.

When a Microsoft account needs authentication, the panel parses the device-code prompt from the log stream (`open the page <url> and enter the code <code>`) and surfaces each account's code and verification URL in the UI — including sub-session accounts (`accounts add <id>`), which the panel proxies to the broadcaster console. Extra accounts persist in the broadcaster's own `cache/sub_sessions.json`; per-account screenshots are written next to each token cache. Commands (`accounts list`, `restart`, `dumpsession`, …) can be sent from the page or `POST /api/v1/broadcaster/command`; everything is also visible on `WS /api/v1/broadcaster/ws`.



## Console protocol and reliability

`/api/v1/console/ws?token=<JWT>` sends `hello`, current `status`, current `error_stats`, retained console lines, then live events. Existing console fields (`type`, `line`, `level`, Unix-seconds `timestamp`) remain. Added fields:

```json
{"type":"console","line":"[INFO] Server ready","level":"info","timestamp":1700000000.0,"sequence":42,"stream_id":"f0a1c2d3-4567-4890-abcd-0123456789ab","source":"stdout","continued":false}
```

Reconnect using `after=<last-sequence>&stream_id=<id>` on the same endpoint. The built-in frontend does this automatically and discards duplicates. A new panel process has a new stream ID and sends `hello` with `resumed:false`; the frontend resets its history. The RAM ring gives fast normal delivery. A buffered segmented disk spool recovers older events and applies back-pressure to pipe readers on slow storage. It flushes every 250 ms and before disk replay. A crash/power failure can lose buffered writes; this is not an fsync-per-line durability guarantee.

Disk retention includes prior spool files, deleting the oldest first as new events arrive. If a cursor expires from both RAM and disk, the server emits `{"type":"gap","from":...,"to":...}` before the remaining events. Disk failure is logged and falls back to bounded RAM replay. No finite retention window can guarantee delivery to indefinitely disconnected clients. Connected readers with sufficient throughput and reconnects within retention recover every sequenced event; the integration suite verifies a 10,000-line burst against a 128-line RAM window.

Commands use `{"command":"list"}` on the console socket or `POST /api/v1/console/command`. Stopped servers reject commands; stale commands are never carried into the next process. Stdin has a five-second enqueue/write deadline. Each socket has one writer and a ten-second send deadline. Oversized lines are chunked with `continued:true` to preserve text without unbounded allocations. Invalid UTF-8 is replaced. Stdout/stderr have independent readers, source tags, and one global sequence; relative ordering across the two OS pipes is arrival order.

## Backup safety and intentional improvements

- Live backups require before/after `send` hooks for `save hold` and `save resume`. The backend polls `save query` until Bedrock confirms the snapshot can be copied. Stopped-server backups can use `run_hooks:false`.
- Post-hooks run after pre-hook/compression failure. Dry runs execute no shell, console, or wait side effects.
- Shell hooks fail the job on nonzero exit or timeout. They run as the panel OS user; editing hooks and shell test commands requires `SETTINGS_EDIT`.
- Compression/extraction runs off the async workers. ZIP writes publish atomically; a disconnected HTTP client does not cancel an active backup.
- Restores stop the game, reject traversal, other-world files, symlinks, oversized archives, and incomplete world archives. Extraction is staged before replacing the world; the previous world remains as `<world>.bak.<timestamp>.<id>`.
- Scheduled retention moves only older `auto_backup_` archives to trash and preserves manual archives.
- File paths are confined to configured roots. Sensitive stores are edited through their dedicated APIs, preventing role/JWT changes through the generic file editor. Export folders must stay inside `BACKUPS_DIR`.
- HTTP and WebSocket permissions are enforced per operation, including player ban/op and console sends. Deleted users are rejected immediately on REST calls; active sockets revalidate on heartbeats. Owners cannot all be removed.
- History survives game restarts; statistics reset for a new game process. Manual start resets the crash budget. MSPT converts to TPS instead of being interpreted as TPS. Metrics include launcher descendants.

## API

All protected REST routes use `Authorization: Bearer <JWT>`. WebSockets and direct ZIP downloads also accept `?token=`. `/health` is public. Main endpoint families under `/api/v1`:

| Family | Endpoints |
| --- | --- |
| auth | `POST /auth/login`, `GET /auth/me`, `GET/POST /auth/users`, `PUT/DELETE /auth/users/{username}` |
| roles | `GET/POST /roles/`, `GET/PUT/DELETE /roles/{name}` |
| server/console | `GET /server/status`, `POST /server/action`, `POST /console/command`, `WS /console/ws` |
| server backend | `GET /server/backend`, `GET /server/backend/versions`, `POST /server/backend/install`, `POST /server/backend/switch` |
| properties | `GET /properties/`, `GET/PUT /properties/raw`, `PUT /properties/{key}` |
| players/worlds | `GET /players/`, `POST /players/action`, `GET /worlds/`, `GET /worlds/{world}`, `GET /worlds/{world}/contents` |
| addons | `GET /addons/`, `GET/PUT /addons/manifest`, `GET/PUT /addons/order/{world}/{pack_type}`, `PUT /addons/rename`, `POST /addons/randomize-uuid`, `PUT /addons/change-uuid` |
| backups | `GET /backups/`, `POST /backups/create`, `GET /backups/worlds`, `GET /backups/trash`, `DELETE /backups/{world}/{filename}` |
| restore/download | `POST /backups/restore/{world}/{filename}`, `POST /backups/{world}/{filename}/restore-to-world`, `GET /backups/{world}/{filename}/download` |
| backup configuration | `GET/PUT /backups/settings`, `GET/PUT /backups/scheduler`, `POST /backups/test-command`, `GET /backups/include-items`, `GET /backups/folders`, `WS /backups/ws` |
| files/settings/audit | `GET /files/`, `GET/PUT/DELETE /files/{filename}`, `GET /settings/`, `GET/PUT /settings/server`, `GET /audit/` |
| performance/logs | `GET /performance/metrics`, `WS /performance/ws`, `WS /logs/ws` |
| broadcaster | `GET /broadcaster/status`, `POST /broadcaster/action`, `POST /broadcaster/command`, `GET/PUT /broadcaster/config`, `GET/POST /broadcaster/accounts`, `GET /broadcaster/device-code`, `GET/PUT /broadcaster/screenshot`, `WS /broadcaster/ws` |

## Development and validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd frontend && npm run check && npm run build
```

Integration tests use temporary stores, a fake Endstone executable, and loopback sockets. They do not touch your live server. A real Endstone/Bedrock installation and container startup should also be smoke-tested before switching production. The API's Windows build path uses Tokio child termination, but Unix process-group descendant cleanup is the tested behavior; Windows descendant cleanup is not covered. OpenAPI/Swagger generation from FastAPI is not retained.

```
backend/
  omnibedrock-core/        shared types: configuration, errors, filesystem helpers
  omnibedrock-server/      child supervision, console journal, metrics, backups, BDS versions, broadcaster
  omnibedrock-api/         axum routes, WebSocket handlers, auth; integration tests
frontend/                  existing SvelteKit UI
backend-py/                Python migration reference
```

## License

MIT; see [LICENSE](LICENSE).
