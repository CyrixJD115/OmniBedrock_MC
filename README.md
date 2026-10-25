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

The panel never depends on its source tree or working directory. All runtime data lives under one **data root**, resolved in this order (highest wins):

1. `--portable` (or `portable = true` in the config file) → `<executable dir>/data/`
2. `--global-data-dir <path>` (or `global_data_dir` in the config file, or `OMNI_GLOBAL_DATA_DIR`) → that directory
3. default → the platform app-data directory: `~/.local/share/omnibedrock/` on Linux (`XDG_DATA_HOME` honored), `%LOCALAPPDATA%\OmniBedrock\` on Windows, `~/Library/Application Support/omnibedrock/` on macOS

Supplying `--portable` and `--global-data-dir` together (or setting both file keys in one config file) is rejected with an explicit error; a CLI flag always beats the same-named config-file setting. Required directories are created automatically at startup, and nothing is ever written relative to wherever the binary was launched from.

Every remaining setting follows: **defaults → config file → environment / `.env` → CLI flags**. Relative paths in config-file settings resolve from the working directory (`global_data_dir` is the exception: it resolves against the config file's own directory). See [`deploy/config.example.toml`](deploy/config.example.toml) for a fully commented sample.

### Config files

- `--config <file>` — explicit path; `.toml` (preferred) or legacy `.yaml`/`.yml`. The file must exist.
- Otherwise auto-discovered: `<executable dir>/config.toml` first, then `~/.config/omnibedrock/config.toml` on Linux (XDG_CONFIG_HOME honored). Windows/macOS use the executable-side file only.
- Config discovery never consults the data directory, so data-root settings cannot create circular resolution.
- Only one file is ever read — discovered files are never merged.
- Unknown keys are rejected (typo protection). Secrets (`SECRET_KEY`, `CURSEFORGE_API_KEY`, `ADMIN_PASSWORD`) are never read from config files — keep them in the environment or `.env`.

### Data root layout

Whatever resolves the root, the layout under it is the same:

```
<root>/
├── config.toml    # only in portable mode; the ONLY config file read there
├── server/        # default instance: game server + worlds
├── instances/     # additional instances (<id>/server, settings, stdin.fifo)
├── logs/          # panel + audit logs, per-instance console journals
├── backups/       # world backups (per-instance subdirectories)
├── cache/         # download caches, generated docker build contexts
├── broadcaster/   # MCXboxBroadcast home
└── tools/         # uv/Python/Endstone toolchains
```

Explicit `--data-dir`/`--bedrock-server-dir`/`--logs-dir`/`--backups-dir` (or their env vars) still override individual locations, and external drives/paths can be mixed in deliberately. Limitations: the directory must be writable, and network mounts need to allow the executable's file locking. System-wide and portable state are never mixed — portable mode ignores the other config locations.

### Paths

| Path | Default | Controls |
| --- | --- | --- |
| `BEDROCK_SERVER_DIR` / `--bedrock-server-dir` | `<root>/server` | Game server root (binary, `worlds/`, properties) |
| `DATA_DIR` / `--data-dir` | `<root>` | Accounts, roles, panel settings, signing key, uv/Python/Endstone toolchains |
| `LOGS_DIR` / `--logs-dir` | `<root>/logs` | Audit log, tracing, console replay spool |
| `BACKUPS_DIR` / `--backups-dir` | `<root>/backups` | World backups |
| `CACHE_DIR` / `--cache-dir` | `<root>/cache` | Download caches (CurseForge images); follows DATA_DIR unless set |

Paths containing spaces are fully supported; the Endstone toolchain relocates to `~/.local/state/omnibedrock/` automatically because the Endstone launcher's `LD_PRELOAD`/`LD_LIBRARY_PATH` cannot contain spaces (see startup log for the resolved location). Temporary files use the OS temp dir (`TMPDIR`).

### Other settings

| Variable | Default | Purpose |
| --- | --- | --- |
| `OMNI_HOST`, `OMNI_PORT` | `0.0.0.0`, `17754` | API binding |
| `ENDSTONE_BIN` | managed toolchain → PATH | Explicit endstone executable |
| `ENDSTONE_PYTHON` | unset | Legacy `-m endstone` fallback; probed with `import endstone` before use |
| `BROADCASTER_DIR` | `<root>/broadcaster` | Panel-level broadcaster home (jar location + the default instance's broadcaster): config.yml, screenshot, cache |
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

The startup log reports the resolved configuration: data mode (default / global-data-dir / portable), config source, every effective path, and the detected Endstone/BDS toolchain versions — facts read from the on-disk markers, not assumptions. Example:

```bash
# Platform default: ~/.local/share/omnibedrock (created on first start)
omnibedrock

# Custom data root (CLI, env, or config file)
omnibedrock --global-data-dir /srv/omnibedrock
OMNI_GLOBAL_DATA_DIR=/srv/omnibedrock omnibedrock

# System install with a config file next to the binary
omnibedrock --config /etc/omnibedrock/config.toml

# Fully self-contained tree next to the executable
omnibedrock --portable
```

Upgrading is safe: the panel reuses valid installations (re-running "Install Endstone toolchain" without a version pin is a no-op when the toolchain is present), and BDS/toolchain upgrades never touch worlds, `server.properties`, or the allowlist.

Auto-restart and stop/kill timeouts are persisted in `server_settings.yaml` through `/api/v1/settings/server`. Backup configuration lives in `backup_settings.yaml`. The scheduler applies configuration changes immediately and waits the configured interval before running. Server shutdown cancels scheduling, lets active jobs clean up save state, stops the game, and drains output. Ensure service/container stop timeouts cover your configured hooks and grace periods.

## Dual backend: Endstone or vanilla BDS

The panel supervises either an Endstone launcher or the official `bedrock_server` binary in the same server directory. The active backend, and the vanilla version pin, live in `server_settings.yaml` and are editable from Settings → Server backend. Switching stops the game if it is running, applies the selection, and restarts on the new backend; `worlds/`, `server.properties`, `allowlist.json`, and `permissions.json` are shared between both backends and never touched by a switch.

Vanilla installs are managed in the panel: the version registry and SHA256 checksums come from [EndstoneMC/bedrock-server-data](https://github.com/EndstoneMC/bedrock-server-data) (MIT), and the archives themselves download from the official `minecraft.net` CDN URLs that registry provides — the panel never executes an archive whose hash does not match. Pick `latest` to track the newest release or pin any registry version (release or preview). Installing extracts to a staging copy first, refuses symlink entries, replaces upstream files, preserves your user data, marks the binary executable, and records the applied version in `data/bds_version.json`. Install progress streams both into the game console and the backend status endpoint. Vanilla BDS accepts the same console commands, backups, and properties tooling as Endstone; TPS is only reported when the backend prints it.

New instances get a minimal `server.properties` seeded with `transport=raknet`, `allow-list=false`, and the instance's `server-port`/`server-portv6` pair; every start re-pins the ports and adds any of those keys that are missing. The explicit values matter because modern BDS fills unset keys with defaults that break panel-managed servers: `transport=nethernet` ignores `server-port` entirely (clients can never reach the advertised port), and `allow-list=true` with an empty list spams a warning every boot and rejects joins. Deliberately chosen values always win, and a non-raknet `transport` logs a loud console warning at start so a silent port mismatch can't hide.

## MCXboxBroadcast integration

The optional [MCXboxBroadcast](https://github.com/MCXboxBroadcast/Broadcaster) standalone is **per server instance**: every instance runs its own supervised Java process (own start/stop/restart, own log ring, own `config.yml`, own Microsoft account token cache, killed cleanly when the instance stops or is deleted), so two servers can broadcast different worlds with different accounts without interfering. The Broadcaster page follows the active instance; all `/broadcaster/*` endpoints are scoped with `X-Instance`.

Home directories: the migrated `default` instance keeps the panel-level `BROADCASTER_DIR` (default `<data root>/broadcaster`) for backward compatibility; every other instance gets `<data root>/instances/<id>/broadcaster/`. The jar itself is shared — drop `MCXboxBroadcastStandalone.jar` into `BROADCASTER_DIR` once (or point `BROADCASTER_JAR` at it) and every instance resolves it; a jar inside an instance's own dir wins over the shared one. Ensure `java` is on `PATH` (or set `JAVA_BIN`). The panel seeds the documented default `config.yml` on first read and validates YAML before saving.

**Join address gotcha:** `session.session-info.ip` must be an address **joining clients** can reach. The broadcaster's Xbox LIVE session hands friends to the broadcaster over NetherNet, and after that fake handshake the client is told (Bedrock `TransferPacket`) to connect to exactly `session-info.ip:port` over plain RakNet — the broadcaster never proxies gameplay, it only logs "Transferred bedrock client …". So: friends joining over the internet need your **public/WAN IP** here (with the gameplay port forwarded as UDP on the router to the server machine); a LAN IP only works for players on the same network; a VPN IP (Tailscale, ZeroTier, …) works for VPN members. If the address is unreachable there is exactly one failure shape: the broadcaster logs "Transferred bedrock client … to target server", the client errors out (e.g. InitialConnection-13), and the game server never logs the player. `session.ice-port-range` pins the WebRTC ICE ports (one per in-progress join) so only that small UDP range needs opening/forwarding alongside the gameplay port.

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

All protected REST routes use `Authorization: Bearer <JWT>`. WebSockets and direct ZIP downloads also accept `?token=`. `/health` and `GET /api/v1/openapi.json` are public — the latter is a machine-readable inventory of every endpoint family with its permission gate and request-body keys. Endpoints for a non-default instance are scoped with the `X-Instance: <id>` header (or `?instance=<id>` for browser downloads). Main endpoint families under `/api/v1`:

| Family | Endpoints |
| --- | --- |
| auth | `POST /auth/login`, `GET /auth/me`, `GET/POST /auth/users`, `PUT/DELETE /auth/users/{username}` |
| roles | `GET/POST /roles/`, `GET/PUT/DELETE /roles/{name}` |
| server/console | `GET /server/status`, `POST /server/action`, `POST /console/command`, `GET /console/tail`, `WS /console/ws` |
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
| instances | `GET/POST /instances`, `GET/PUT/DELETE /instances/{id}`, `POST /instances/{id}/action`, `GET /host` |

`GET /console/tail?lines=200` returns the last N console lines as JSON (`{"instance","latest","events":[…]}`, same event shape as the socket) without holding a WebSocket open; add `&format=text` for one line per row, handy for `curl … | tail` health checks. The live feed stays on `WS /console/ws`.

### Instances and access control

```bash
TOKEN=$(curl -s -X POST http://localhost:17754/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"Admin","password":"…"}' | jq -r .token)

# Create (id derives from the name; port 0 auto-picks a free pair)
curl -X POST http://localhost:17754/api/v1/instances \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"friend-server","backend":"vanilla","deployment":"native","port":0}'

# Share it: PUT /instances/{id} replaces the whole access list.
# roles are names from the shared catalog (GET /roles) — the built-in
# instance roles are "viewer" (read-only) and "instance-manager".
curl -X PUT http://localhost:17754/api/v1/instances/friend-server \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"access":[{"user":"friend","roles":["viewer"]}]}'

curl -X POST http://localhost:17754/api/v1/instances/friend-server/action \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"action":"start"}'

# Delete is destructive and two-step: ?confirm=yes (add &keep_data=1 to keep files)
```

Edits are full-document PUT (no PATCH): a `PUT /instances/{id}` with only `access` leaves everything else untouched, and an `access` array always replaces the previous list. Only the instance owner, a panel owner, or holders of `INSTANCES_SHARE` may change access; name/port/deployment/resource edits additionally need the owner path or `INSTANCES_LIMITS`. You cannot grant an instance capability you do not hold on that instance yourself. Quotas (`GET /auth/me`) apply to the creating user; the panel owner is exempt.

### Upgrading the panel binary

After `kill`/SIGTERM the panel drains: it waits for running instances to stop gracefully before exiting, which can take tens of seconds. Wait for the process to actually exit (`systemctl --user stop omnibedrock`, or poll the PID) before replacing the binary — an immediate swap fails with `Text file busy`.

## Development and validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd frontend && npm run check && npm run build
```

On a fresh clone, `cargo build` runs `npm ci && npm run build` in `frontend/` itself (and rebuilds the UI whenever `frontend/src` changes); if that build fails, the cargo build fails too rather than silently bundling a placeholder UI. Set `OMNI_SKIP_FRONTEND_BUILD=1` to skip npm and accept the placeholder explicitly. CI and the Dockerfile build the UI in their own steps, so they are unaffected.

Integration tests use temporary stores, a fake Endstone executable, and loopback sockets. They do not touch your live server. A real Endstone/Bedrock installation and container startup should also be smoke-tested before switching production. The API's Windows build path uses Tokio child termination, but Unix process-group descendant cleanup is the tested behavior; Windows descendant cleanup is not covered. OpenAPI/Swagger generation from FastAPI is not retained.

```
backend/
  omnibedrock-core/        shared types: configuration, errors, filesystem helpers
  omnibedrock-server/      child supervision, console journal, metrics, backups, BDS versions, broadcaster
  omnibedrock-api/         axum routes, WebSocket handlers, auth; integration tests
frontend/                  existing SvelteKit UI
```

## License

MIT; see [LICENSE](LICENSE).
