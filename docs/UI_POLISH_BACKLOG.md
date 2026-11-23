# UI Polish Backlog — Next Session

Handoff from the 2026-11-23 overhaul session (deployed through `9a64d72`).
The overhaul itself is live and verified; what remains is consistency polish
on the pages that were not reworked, plus one standing watch item.

## Context

The reworked pages (Dashboard, Instances, Mods, Backups, Files, Properties,
Settings, Players, Performance) share a newer layout language:

- Sectioned layouts with tab navigation (see `frontend/src/routes/backups/+page.svelte`
  for the reference pattern: tab bar with icons, per-tab panels).
- Typed API client calls from `frontend/src/lib/api/client.ts` — no raw fetches.
- `$state`/`$derived` runes, stores from `frontend/src/stores/`.
- A11y discipline: `aria-label` on icon-only controls, focusable cards,
  keyboard-navigable tables (see commit `e9ab331` for the standard).
- Status/empty states rendered inline, not as toasts.

Before every commit: `cargo fmt --all`, `cargo clippy --workspace
--all-targets --locked -- -D warnings`, `cargo test --workspace --locked`,
`npm --prefix frontend run check` (0 errors **and** 0 warnings),
`npm --prefix frontend test`. All source files stay under 1000 lines.

## Status (2026-11-23, later session)

Done: Console (`b85d1a7`), Broadcaster (`687327f`), Store (`a4ecf44`,
also readable API errors panel-wide), Audit (this commit). Users/Roles
was reviewed and left as is: already two focused panels (267/352 lines)
that render cleanly; extracting a shared permission matrix is optional.

## Pages to rework (in priority order)

1. **Console** — `frontend/src/routes/console/+page.svelte` +
   `frontend/src/lib/components/terminal/Terminal.svelte` (386 lines).
   Align with the instance-scoped patterns used by Players/Backups
   (instance switcher placement, empty states, the shared console store in
   `frontend/src/lib/broadcaster/console.ts` has the per-instance
   bookkeeping model to follow).
2. **Broadcaster** — `frontend/src/routes/broadcaster/+page.svelte` (521 lines).
   Split into components (status card, accounts, config editor backed by
   `frontend/src/lib/broadcaster/configModel.ts`). Note the NetherNet
   health-probe semantics (memory: `broadcaster-nethernet-probe`) — any
   reachability UI must not assume RakNet.
3. **Store** — `frontend/src/routes/store/+page.svelte` (617 lines).
   Split listing/detail; reuse the CurseForge image cache endpoints.
4. **Users/Roles** — `frontend/src/lib/components/users/UsersPanel.svelte` (267)
   and `RolesPanel.svelte` (352). Modal flows and permission matrix could
   become shared components.
5. **Audit** — oldest layout; table + filters, low risk.

## Watch item: Mods page size

`frontend/src/routes/addons/+page.svelte` is **805 lines** — fine today, but
**split it the moment it grows past ~850** (extract the compatibility tab
and the upload queue into components; `frontend/src/lib/addons/sortable.svelte.ts`
is already separate). Hard ceiling: 1000 lines.

## Live-environment facts learned this session

- Production: `ssh novacorelinux`, panel at `~/Server/OmniBedrock` (see
  `memory/live-deployment.md`; deploy flow unchanged). Current live commit:
  `9a64d72`.
- Panel API auth: `POST /api/v1/auth/login` → bearer; websockets need a
  short-lived `dt` token from `POST /auth/download-token` passed as `?dt=`.
  Instance scoping is `?instance=` or `x-instance` header on every route.
- After a NetherNet server stops, the game port stays unbindable ~60s
  (TIME_WAIT from loopback keep-alives). `wait_ports_free` handles it —
  budget is 75s — so expect a stop→start to take up to ~1 min and say so
  in the console. Do not "fix" this back to a short timeout.
- **furralis-craft has 2 root-owned files** in its server folder (flagged by
  the migration preflight, e.g. `worlds/RealismCraft/level.dat.bak-20261105`).
  Until `sudo chown -R` fixes them, docker/systemd migration for that
  instance is blocked. Fix on the host, then re-run the preflight.
- Docker migration (native↔docker↔systemd) was verified end-to-end on
  novacorelinux on a scratch instance; zero leftover containers/units.
  The shared `omnibedrock-vanilla` image (135 MB) is intentionally kept.
- Player join/leave tracking was verified by tests only — nobody joined the
  live server during the session. On the next real join, confirm
  `data/instances/furralis-craft/players.json` (xuid, sessions,
  playtime; `state_dir` is the instance root, alongside
  `console_lock_state.yaml`) and the Players tab session timers.

## Suggested session plan

1. One page per commit, Console first (highest traffic).
2. Run the full gate list above per commit; push only after all gates pass.
3. Deploy per `memory/live-deployment.md` with the user's explicit go-ahead;
  check `GET /api/v1/players` is empty first.
