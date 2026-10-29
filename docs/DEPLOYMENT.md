# Deployment & Resource Enforcement

OmniBedrock can manage several Minecraft Bedrock instances side by side, each
with its own runtime (vanilla BDS or Endstone), world data, console, backups,
users, and resource limits. **A configured limit is only as strong as the
deployment backend that runs the instance** — this document states exactly
what each platform enforces.

## Enforcement matrix

| Deployment | Where it works | Memory limit | CPU limit | Enforcement |
|---|---|---|---|---|
| `native` | everywhere | — (monitored) | — (monitored) | **Advisory.** The panel reports usage but the OS enforces nothing. |
| `systemd` | Linux with a systemd **user** session | `MemoryMax` + `MemorySwapMax=0` (hard; OOM kills the service) | `CPUQuota` (scheduler share, not dedicated cores) | cgroup v2 via a persistent user service unit |
| `docker` | any host running Docker | `--memory` + `--memory-swap` (hard; OOM kills the container) | `--cpus` (scheduler quota, not dedicated cores) | container runtime |

Notes:

- CPU limits are **time quotas** (a 1.0 limit ≈ one core's worth of scheduled
  time), never pinned or isolated cores.
- `docker` on macOS/Windows runs inside the Docker Desktop VM. Container
  limits are enforced by the VM's runtime, but the VM itself caps total host
  resources — configure the VM's allocation in Docker Desktop.
- The panel owner can always see, on every instance card, whether its limits
  are `enforced` (cgroup/container) or `advisory` (native). The Performance
  page repeats this per instance.
- Availability is probed at panel start (`docker version`,
  `systemctl --user show-environment`). Deployment choices that the host
  cannot honor are rejected at instance creation, not at start time.

## Recommended setups

- **Linux server / desktop (best):** run the panel natively and create
  instances with `deployment = systemd`. Requires the panel to run inside a
  systemd user session (`$DBUS_SESSION_BUS_ADDRESS` / `loginctl` user
  slice). No root needed; limits are real cgroup controls, and servers keep
  running through panel restarts and (with autostart + linger) reboots.
- **Windows / macOS:** install Docker Desktop, build the two runtime images
  once (below), and create instances with `deployment = docker`. Native
  instances still work for simple use, but their limits are advisory only.
- **Any host with Docker:** `deployment = docker` gives identical, enforced
  behavior across platforms.

## Docker images

The panel shells out to the `docker` CLI and runs each instance as:

```
docker run -i --rm --name omnibedrock-<id> \
  [--memory <mb>m --memory-swap <mb>m] [--cpus <n>] \
  -p <port>:19132/udp \
  -v <instance server dir>:/data -w /data \
  <image>
```

Worlds and configuration live on the host and are mounted into the container,
so containers stay disposable — recreating or updating a container never
touches them. Containers run as the **panel user's uid/gid** (`--user`), so
files written into `/data` stay owned by the panel user and deletable by it;
no container runs as root (Bedrock's UDP ports are unprivileged). Build the
images once from the repo root, or let the panel do
it: when a configured `omnibedrock-*` image is missing, the first instance
start generates the same Dockerfile under `<cache>/docker/` and builds it,
streaming progress into the instance console (this needs network access to
pull the base image):

```sh
docker build -f deploy/docker/Dockerfile.vanilla   -t omnibedrock-vanilla:latest .
docker build -f deploy/docker/Dockerfile.endstone  -t omnibedrock-endstone:latest .
# optional: pin the Endstone version
docker build -f deploy/docker/Dockerfile.endstone --build-arg ENDSTONE_VERSION=0.11.14 -t omnibedrock-endstone:latest .
```

Custom (e.g. registry) images can be configured (`config.toml`, env, or
CLI); custom images are never auto-built — pull or build them yourself:

```toml
docker_image_vanilla = "registry.example.com/omnibedrock-vanilla:latest"
docker_image_endstone = "registry.example.com/omnibedrock-endstone:latest"
```

Console input/output is piped through the container, so the panel Console,
player commands, and the backup `save`/`resume` hooks work unchanged. Graceful
stop sends `stop` over stdin; force-stop uses `docker kill`. If the panel is
killed hard, a stale container is removed automatically before the next start.

## Ports: every instance owns a pair

Bedrock binds two UDP ports: `server-port` (IPv4) and `server-portv6`
(IPv6, dual-stack). The two numbers must differ — identical values make BDS
collide with itself — so an instance registered at port **P** occupies the
pair **P (v4) and P+1 (v6)**, exactly like a stock install's 19132/19133.
Players connect on P. The panel validates the whole pair against both IP
stacks before assigning or changing a port, so neighbors can never overlap
(not even via the v6 slot), and docker deployments publish both
`P→19132/udp` and `P+1→19133/udp` while pinning the stock pair inside the
container.

## Disk usage vs. disk quotas

The panel **reports** disk usage per instance (refreshed about once a minute,
labeled *monitoring only*) and host filesystem usage, but it does **not**
enforce disk quotas. Honest per-platform reality:

| Platform | Enforceable per-instance disk quota? |
|---|---|
| Docker (`--storage-opt size=…`) | Only when the daemon uses overlay2 on an XFS backing filesystem mounted with `pquota`. Without that (ext4, Docker Desktop), the flag makes container creation **fail** — so OmniBedrock does not set it by default. |
| systemd/cgroups | cgroup v2 has no filesystem-size controller; enforcement needs filesystem-level project quotas (`xfs_quota`/`f2fs` project quotas) applied to the instance directories by the admin. |
| Native (any OS) | None from the panel. |

What you get everywhere: per-instance disk usage on the instance cards,
host disk capacity and usage on the Instances page, and world sizes on the
Worlds page. Watch those, and use the backup retention settings to keep
storage bounded. If your filesystem supports project quotas, you can apply
them to `<data_dir>/instances/<id>` manually; the panel will not fight you.

## Container health checks and restarts

OmniBedrock containers are started **without** a Docker `HEALTHCHECK` and
**without** `--restart`: both jobs are done by the panel's supervisor, which
owns the lifecycle.

- **Why no `--health-cmd`:** Bedrock Dedicated Server and Endstone expose no
  in-container probe command (no HTTP status endpoint, no CLI health binary).
  A `pgrep`-style check would only detect a dead PID 1 — which container exit
  already reports — and a UDP query ping would need a query client inside
  every image. Adding one would be theater, not health checking.
- **The substitute:** the supervisor watches the `docker run` client's exit
  and the container's stdout stream. An unexpected exit marks the instance
  `crashed`, is visible on the dashboard and console, and auto-restart (with
  a crash budget) brings it back exactly as for native instances. Graceful
  stop sends `stop` over stdin; force-stop uses `docker kill`; a stale
  container from a hard panel kill is removed before the next start.
- **If you want Docker-level restarts anyway:** create the container policy
  yourself is not supported — the panel recreates containers and must stay
  the single manager. Running the *panel itself* as a systemd service (below)
  gives you panel-level restart.

## Least-privilege operation (Linux)

The panel runs its instances as the panel's own user. For hardened operation,
run the whole panel under a dedicated account with a hardened systemd unit —
instance services (`omnibedrock-*.service`) then live under that user's
slice and inherit its restrictions:

```ini
# ~/.config/systemd/user/omnibedrock.service  (enable with systemctl --user enable --now omnibedrock)
[Unit]
Description=OmniBedrock panel
After=network-online.target

[Service]
ExecStart=%h/omnibedrock/omnibedrock --portable
# Hardening: the OS tree is read-only; the panel's own home tree and the
# docker socket stay writable.
ProtectSystem=full
PrivateTmp=true
NoNewPrivileges=true
[Install]
WantedBy=default.target
```

Notes:

- The `docker` deployment needs the user to be in the `docker` group (or use
  rootless Docker). Group membership is effectively root on the host — if
  that matters, prefer `systemd` instances and keep the user out of `docker`.
- Keep `data_dir` on a filesystem the panel user owns; backups can be
  pointed at a separate volume via `BACKUPS_DIR`.
- The panel never runs user-supplied shell commands with elevated
  privileges; backup hooks are the only arbitrary-command feature and run as
  the panel user, configured by admins only.

## systemd specifics: real user service units

Each `systemd` instance runs inside a persistent **user** unit,
`omnibedrock-<id>.service`, written to `~/.config/systemd/user/`. The panel
never needs root, and it never touches system-level units. The unit's
`ExecStart` points back at the panel binary running a hidden
`__unit-helper` subcommand that:

- spawns the actual game server with stdin wired to a FIFO
  (`<instance dir>/stdin.fifo`) that the panel writes console commands into,
- inherits stdout/stderr so the journal captures them (`journalctl --user -u
  omnibedrock-<id>`, mirrored live into the panel Console),
- shuts down gracefully when asked: `stop` command first, then SIGTERM, then
  SIGKILL — the same ladder the panel uses for owned processes.

Because systemd parents the process:

- **The server survives panel restarts.** On boot (or panel restart) the
  panel re-attaches: console follows the journal, status comes from
  `systemctl --user show`, and metrics sample the unit's `MainPID` tree.
- **Restart policy stays with the panel.** The unit itself is `Restart=no`;
  crash detection, the crash budget, and auto-restart behave identically to
  native/docker instances.
- **Autostart at boot** is per instance ("Start at boot" in the UI,
  `autostart: true` via the API): it runs `systemctl --user enable` on the
  unit. User units only start at boot when the account has linger enabled —
  the panel reports linger in its host capabilities and shows a hint when it
  is off:

  ```sh
  loginctl enable-linger $USER
  ```

Unit files carry a `# Managed by OmniBedrock` header. A pre-existing file
without it is never overwritten — remove or rename it and the panel will
manage the unit again. The unit is regenerated on every start (so a moved
panel binary heals the `ExecStart` path), removed when the instance is
deleted, and disabled when the deployment switches away from systemd.

Inspect a running instance:

```sh
systemctl --user status omnibedrock-<id>.service
journalctl --user -u omnibedrock-<id>.service -f
```

When the panel runs as a system service without a user manager (e.g. under a
plain `sudo`), the user-manager probe fails and the backend is reported
unavailable — use `native` or `docker` instances then. Availability and
linger state are probed at panel start and reported through `/api/v1/host`.

## Roles and instance access (unified)

One role catalog serves both panel permissions and per-instance shares. A role is a named
permission set with a color, description and hierarchy position; users hold any number of roles
and get the **union** of their permissions.

- **Built-in roles** (all editable; keys are stable): `owner` (sentinel holder, full control),
  `admin`, `moderator`, `instance-manager` (the classic "manage" share), `user` (own instances),
  `viewer` (default for new users). Built-ins cannot be deleted; custom roles can.
- **Combination rules are purely additive.** No role can revoke another's permissions, and no
  combination can bypass the rules that live *outside* the permission list: owner-sentinel
  checks, per-user quotas, and the "you cannot grant what you do not hold" delegation guards.
- **Owner protection.** Exactly one role carries the `grants_owner` sentinel (renames of the
  display name are fine; the sentinel is never API-settable). The platform refuses changes that
  would leave zero owner roles or zero users holding one.
- **Delegation guards.** Editors without the owner sentinel cannot assign or author roles
  granting permissions they do not hold themselves (globally, or on the instance for shares).
- **Resource limits are granular.** `INSTANCES_LIMITS` (grantable globally or through an
  instance share) lets a non-owner tune an instance's RAM/CPU/deployment/port. Per-user quotas
  still apply to every non-owner edit, and acting panel owners remain deliberately exempt.
- **Instance shares** (Instances → Edit) assign catalog roles to a user *on one instance*. On a
  shared instance, capability comes from the entry's roles — global role permissions do **not**
  leak onto other people's instances. Holders of the global `INSTANCES_VIEW_ALL` capability
  (stock admin/owner) also project their global permissions onto every instance. The migrated
  `default` instance keeps the legacy open-to-authenticated model gated by global permissions.
- **Legacy migration.** Old single-role users gain `roles: [<their role>]`; instance access
  entries with `level: view|manage` map to the `viewer` / `instance-manager` roles; still-stock
  built-in role permission lists are refreshed with new capabilities (customized roles are never
  touched). All of this is persisted back on first boot.

Roles, memberships, and shares are audited (`roles.*`, user role changes, access updates).

## Users, quotas, and instances

- The first account (role `owner`) controls everything: users, roles, quotas,
  instances, and host settings.
- The built-in `user` role can create and fully manage **their own** instances
  (runtime choice included: one user can run Endstone while another runs
  vanilla), but cannot raise their own quotas or touch instances they were not
  granted access to.
- Per-user quotas (`max instances`, total RAM, total CPUs) are enforced by the
  backend inside a single lock, so concurrent creates cannot race past them.
  Quotas apply to everyone except the panel owner.
- Instance sharing: the owner (or instance owner) grants other users `view`
  or `manage` access per instance. Unauthorized instance ids return 404 so
  their existence is not revealed.
- The migrated `default` instance keeps the legacy single-server model: every
  authenticated user can reach it according to their role permissions. Create
  per-user instances when you need isolation.

## Data layout

```
<data_dir>/instances.yaml             # instance registry
<data_dir>/instances/<id>/server/     # that instance's BDS/Endstone files
<data_dir>/instances/<id>/server_settings.yaml
<logs_dir>/instances/<id>/console/    # console journal per instance
<backups_dir>/<id>/                   # per-instance backups
```

The migrated `default` instance keeps its pre-multi-instance paths
(`<bedrock_server_dir>`, `<data_dir>/server_settings.yaml`, …) exactly as
before; upgrading an existing installation moves nothing.
