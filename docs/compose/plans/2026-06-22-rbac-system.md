# RBAC System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded 4-role hierarchy (owner/admin/moderator/viewer) with a fully customizable RBAC system where roles define granular permission flags per module.

**Architecture:** YAML-backed custom roles (data_dir/roles.yaml), string-array permissions, owner as super-admin bypass. Permission checks via `require_permission("PERM_NAME")` FastAPI dependency replacing `require_role()`. Frontend gets resolved permissions via `GET /auth/me` and uses a reactive `$permissions` store for visibility guards.

**Tech Stack:** FastAPI, YAML (ruamel.yaml / pyyaml), Svelte 5, TailwindCSS, Lucide icons

---

## File structure

**New files:**
- `backend/app/models/role.py` — Role dataclass + serialization
- `backend/app/core/permissions.py` — Permission string constants
- `backend/app/core/roles.py` — YAML-based role store (CRUD + resolve)
- `backend/app/routers/roles.py` — REST API for role management
- `frontend/src/lib/stores/permissions.ts` — Svelte writable store + hasPermission()
- `frontend/src/routes/roles/+page.svelte` — Role CRUD management page

**Modified files:**
- `backend/app/models/user.py` — Remove UserRole enum, change `role: str`
- `backend/app/core/auth.py` — Update create_user/update_user for str role; add get_user_permissions(); add resolve_permissions()
- `backend/app/core/security.py` — Add require_permission(); keep verify_token() unchanged
- `backend/app/main.py` — import + init_roles() + register roles router
- 10 router files — Replace require_role() with require_permission()
- `frontend/src/lib/api/client.ts` — Add getMe(), role CRUD methods
- `frontend/src/types/index.ts` — Add Role type
- `frontend/src/lib/components/layout/Sidebar.svelte` — Permission-based nav visibility
- `frontend/src/routes/+layout.svelte` — Fetch permissions on auth
- `frontend/src/routes/users/+page.svelte` — Dynamic role selector from API
- `frontend/src/routes/teams/+page.svelte` — Permission-gated team UI

---

### Task 1: Role model + permissions constants

**Covers:** [S2]

**Files:**
- Create: `backend/app/models/role.py`
- Create: `backend/app/core/permissions.py`

- [ ] **Step 1: Create `permissions.py` with all 24 permission constants**

```python
from __future__ import annotations

# Console
CONSOLE_VIEW = "CONSOLE_VIEW"
CONSOLE_SEND = "CONSOLE_SEND"

# Server
SERVER_VIEW = "SERVER_VIEW"
SERVER_MANAGE = "SERVER_MANAGE"

# Players
PLAYERS_VIEW = "PLAYERS_VIEW"
PLAYERS_KICK = "PLAYERS_KICK"
PLAYERS_BAN = "PLAYERS_BAN"
PLAYERS_OP = "PLAYERS_OP"

# Properties
PROPERTIES_VIEW = "PROPERTIES_VIEW"
PROPERTIES_EDIT = "PROPERTIES_EDIT"

# Addons
ADDONS_VIEW = "ADDONS_VIEW"
ADDONS_MANAGE = "ADDONS_MANAGE"

# Backups
BACKUPS_VIEW = "BACKUPS_VIEW"
BACKUPS_CREATE = "BACKUPS_CREATE"
BACKUPS_RESTORE = "BACKUPS_RESTORE"
BACKUPS_DELETE = "BACKUPS_DELETE"

# Users
USERS_VIEW = "USERS_VIEW"
USERS_CREATE = "USERS_CREATE"
USERS_EDIT = "USERS_EDIT"
USERS_DELETE = "USERS_DELETE"

# Teams
TEAMS_VIEW = "TEAMS_VIEW"
TEAMS_CREATE = "TEAMS_CREATE"
TEAMS_MANAGE = "TEAMS_MANAGE"

# Files
FILES_VIEW = "FILES_VIEW"
FILES_EDIT = "FILES_EDIT"

# Audit
AUDIT_VIEW = "AUDIT_VIEW"

# Settings
SETTINGS_VIEW = "SETTINGS_VIEW"
SETTINGS_EDIT = "SETTINGS_EDIT"

ALL_PERMISSIONS = [
    CONSOLE_VIEW, CONSOLE_SEND,
    SERVER_VIEW, SERVER_MANAGE,
    PLAYERS_VIEW, PLAYERS_KICK, PLAYERS_BAN, PLAYERS_OP,
    PROPERTIES_VIEW, PROPERTIES_EDIT,
    ADDONS_VIEW, ADDONS_MANAGE,
    BACKUPS_VIEW, BACKUPS_CREATE, BACKUPS_RESTORE, BACKUPS_DELETE,
    USERS_VIEW, USERS_CREATE, USERS_EDIT, USERS_DELETE,
    TEAMS_VIEW, TEAMS_CREATE, TEAMS_MANAGE,
    FILES_VIEW, FILES_EDIT,
    AUDIT_VIEW,
    SETTINGS_VIEW, SETTINGS_EDIT,
]
```

- [ ] **Step 2: Create `role.py` model**

```python
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone


@dataclass
class Role:
    name: str
    display_name: str = ""
    permissions: list[str] | None = None
    is_default: bool = False
    created_at: str = ""

    def __post_init__(self):
        if not self.display_name:
            self.display_name = self.name
        if not self.created_at:
            self.created_at = datetime.now(timezone.utc).isoformat()
        if self.permissions is None:
            self.permissions = []

    def to_dict(self) -> dict:
        return {
            "name": self.name,
            "display_name": self.display_name,
            "permissions": sorted(self.permissions or []),
            "is_default": self.is_default,
            "created_at": self.created_at,
        }

    @classmethod
    def from_dict(cls, data: dict) -> Role:
        return cls(
            name=data["name"],
            display_name=data.get("display_name", data["name"]),
            permissions=data.get("permissions", []),
            is_default=data.get("is_default", False),
            created_at=data.get("created_at", ""),
        )
```

---

### Task 2: YAML role store

**Covers:** [S2]

**Files:**
- Create: `backend/app/core/roles.py`

- [ ] **Step 1: Write the role store module**

Same pattern as `auth.py` and `core/teams.py`: module-level `_roles: dict[str, Role]`, YAML file at `data_dir/roles.yaml`.

```python
from __future__ import annotations

from pathlib import Path

import yaml

from backend.app.core.config import settings
from backend.app.core.permissions import ALL_PERMISSIONS
from backend.app.models.role import Role

_role_file: Path = Path(settings.data_dir) / "roles.yaml"
_roles: dict[str, Role] = {}

# Default role definitions used when roles.yaml doesn't exist
_DEFAULT_ROLES = {
    "owner": Role(
        name="owner",
        display_name="Owner",
        permissions=list(ALL_PERMISSIONS),
        is_default=False,
    ),
    "admin": Role(
        name="admin",
        display_name="Admin",
        permissions=list(ALL_PERMISSIONS),
        is_default=False,
    ),
    "moderator": Role(
        name="moderator",
        display_name="Moderator",
        permissions=[
            "CONSOLE_VIEW", "CONSOLE_SEND",
            "SERVER_VIEW",
            "PLAYERS_VIEW", "PLAYERS_KICK",
            "PROPERTIES_VIEW",
            "ADDONS_VIEW",
            "BACKUPS_VIEW", "BACKUPS_CREATE", "BACKUPS_RESTORE", "BACKUPS_DELETE",
            "TEAMS_VIEW", "TEAMS_CREATE", "TEAMS_MANAGE",
            "FILES_VIEW",
            "AUDIT_VIEW",
            "SETTINGS_VIEW",
        ],
        is_default=False,
    ),
    "viewer": Role(
        name="viewer",
        display_name="Viewer",
        permissions=[
            "CONSOLE_VIEW",
            "SERVER_VIEW",
            "PLAYERS_VIEW",
            "TEAMS_VIEW",
            "AUDIT_VIEW",
        ],
        is_default=True,
    ),
}


def _load_roles() -> dict[str, Role]:
    if not _role_file.exists():
        return {}
    try:
        data = yaml.safe_load(_role_file.read_text(encoding="utf-8"))
        if not data:
            return {}
        return {r: Role.from_dict(d) for r, d in data.items()}
    except (yaml.YAMLError, OSError, KeyError) as e:
        logger.error("Failed to load roles: %s", e)
        return {}


def _save_roles() -> None:
    _role_file.parent.mkdir(parents=True, exist_ok=True)
    data = {r: role.to_dict() for r, role in _roles.items()}
    _role_file.write_text(yaml.safe_dump(data, default_flow_style=False, sort_keys=False), encoding="utf-8")


def init_roles() -> None:
    global _roles
    _roles = _load_roles()
    if not _roles:
        logger.info("No roles found. Creating default roles.")
        _roles = {r: Role(**role.to_dict()) for r, role in _DEFAULT_ROLES.items()}
        _save_roles()


def get_role(name: str) -> Role | None:
    return _roles.get(name)


def list_roles() -> list[Role]:
    return list(_roles.values())


def create_role(role: Role) -> bool:
    if role.name in _roles:
        return False
    _roles[role.name] = role
    _save_roles()
    return True


def update_role(name: str, **kwargs) -> Role | None:
    role = _roles.get(name)
    if not role:
        return None
    for k, v in kwargs.items():
        if hasattr(role, k) and v is not None:
            setattr(role, k, v)
    _save_roles()
    return role


def delete_role(name: str) -> bool:
    if name not in _roles:
        return False
    del _roles[name]
    _save_roles()
    return True


def resolve_permissions(role_name: str) -> list[str]:
    if role_name == "owner":
        return list(ALL_PERMISSIONS)
    role = get_role(role_name)
    if not role:
        return []
    return list(role.permissions or [])
```

---

### Task 3: Update User model + auth system

**Covers:** [S2, S3]

**Files:**
- Modify: `backend/app/models/user.py`
- Modify: `backend/app/core/auth.py`

- [ ] **Step 1: Update User model — remove UserRole enum, change role to str**

In `backend/app/models/user.py`:
- Remove the `UserRole` enum entirely (or keep as deprecated alias if referenced elsewhere)
- Change `role: UserRole = UserRole.viewer` → `role: str = "viewer"`
- Remove `UserRole(data.get("role", "viewer"))` from `from_dict` → just use `data.get("role", "viewer")`

```python
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from enum import Enum


# Kept for backwards compatibility during migration — may be removed later
class UserRole(str, Enum):
    owner = "owner"
    admin = "admin"
    moderator = "moderator"
    viewer = "viewer"


@dataclass
class User:
    username: str
    password_hash: str
    role: str = "viewer"
    display_name: str = ""
    created_at: str = ""
    last_login: str = ""

    def __post_init__(self):
        if not self.created_at:
            self.created_at = datetime.now(timezone.utc).isoformat()
        if not self.display_name:
            self.display_name = self.username

    def to_dict(self) -> dict:
        return {
            "username": self.username,
            "role": self.role,
            "display_name": self.display_name,
            "created_at": self.created_at,
            "last_login": self.last_login,
        }

    def to_safe_dict(self) -> dict:
        return self.to_dict()

    @classmethod
    def from_dict(cls, data: dict) -> User:
        role_raw = data.get("role", "viewer")
        # Handle legacy enum values
        if isinstance(role_raw, UserRole):
            role_raw = role_raw.value
        return cls(
            username=data["username"],
            password_hash=data["password_hash"],
            role=role_raw,
            display_name=data.get("display_name", data["username"]),
            created_at=data.get("created_at", ""),
            last_login=data.get("last_login", ""),
        )
```

- [ ] **Step 2: Update auth.py — str-based role + get_user_permissions() + /auth/me**

Changes in `backend/app/core/auth.py`:
- `create_user()`: `role: UserRole` → `role: str = "viewer"`
- `update_user()`: `role: UserRole | None` → `role: str | None`
- `create_access_token()`: `user.role.value` → `user.role` (now it's already a str)
- Add `get_user_permissions(username: str) -> list[str]`: returns `resolve_permissions(user.role)` from roles store
- Import `resolve_permissions` from `backend.app.core.roles`

```python
# Add import at top
from backend.app.core.roles import resolve_permissions

# In create_user:
def create_user(username: str, password: str, role: str = "viewer", display_name: str = "") -> User | None:
    ...

# In update_user:
def update_user(
    username: str,
    role: str | None = None,
    display_name: str | None = None,
    password: str | None = None,
) -> User | None:
    ...

# In create_access_token — user.role is already a str:
    "role": user.role,   # was user.role.value

# New function:
def get_user_permissions(username: str) -> list[str]:
    user = _users.get(username)
    if not user:
        return []
    return resolve_permissions(user.role)
```

- [ ] **Step 3: Add GET /auth/me endpoint**

In `backend/app/routers/auth.py`, add an endpoint that returns the current user + resolved permissions:

```python
@router.get("/me")
async def get_me(current_user: User = Depends(verify_token)) -> dict:
    perms = get_user_permissions(current_user.username)
    return {
        "user": current_user.to_safe_dict(),
        "permissions": perms,
    }
```

Also update the auth router imports and the `create_user`/`update_user` endpoint signatures to use `str` instead of `UserRole`.

---

### Task 4: require_permission dependency

**Covers:** [S4]

**Files:**
- Modify: `backend/app/core/security.py`

- [ ] **Step 1: Add require_permission() to security.py**

```python
from backend.app.core.roles import resolve_permissions

def require_permission(*permissions: str):
    async def _check(user: User = Depends(verify_token)) -> User:
        if user.role == "owner":
            return user
        user_perms = resolve_permissions(user.role)
        for perm in permissions:
            if perm not in user_perms:
                raise HTTPException(
                    status_code=status.HTTP_403_FORBIDDEN,
                    detail="Insufficient permissions",
                )
        return user
    return _check
```

Also export `require_permission` from the module and update imports in `__init__.py` if applicable.

---

### Task 5: Roles REST API

**Covers:** [S4]

**Files:**
- Create: `backend/app/routers/roles.py`

- [ ] **Step 1: Create the roles router**

```python
from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException

from backend.app.core.roles import create_role, delete_role, get_role, list_roles, update_role
from backend.app.core.security import require_permission, verify_token
from backend.app.models.role import Role
from backend.app.models.user import User

router = APIRouter(prefix="/roles", tags=["roles"])


@router.get("/")
async def get_roles(_user: User = Depends(require_permission("USERS_VIEW"))) -> list[dict]:
    return [r.to_dict() for r in list_roles()]


@router.get("/{name}")
async def get_role_endpoint(name: str, _user: User = Depends(require_permission("USERS_VIEW"))) -> dict:
    role = get_role(name)
    if not role:
        raise HTTPException(status_code=404, detail="Role not found")
    return role.to_dict()


@router.post("/")
async def create_role_endpoint(data: dict, _user: User = Depends(require_permission("USERS_CREATE"))) -> dict:
    role = Role(
        name=data["name"],
        display_name=data.get("display_name", data["name"]),
        permissions=data.get("permissions", []),
        is_default=data.get("is_default", False),
    )
    if not create_role(role):
        raise HTTPException(status_code=409, detail="Role already exists")
    return role.to_dict()


@router.put("/{name}")
async def update_role_endpoint(name: str, data: dict, _user: User = Depends(require_permission("USERS_EDIT"))) -> dict:
    role = update_role(
        name,
        display_name=data.get("display_name"),
        permissions=data.get("permissions"),
        is_default=data.get("is_default"),
    )
    if not role:
        raise HTTPException(status_code=404, detail="Role not found")
    return role.to_dict()


@router.delete("/{name}")
async def delete_role_endpoint(name: str, _user: User = Depends(require_permission("USERS_DELETE"))) -> dict:
    if not delete_role(name):
        raise HTTPException(status_code=404, detail="Role not found")
    return {"ok": True}
```

Note: The role router uses USERS_* permissions because managing roles is part of user administration (role dictates what users can do).

---

### Task 6: Migrate all backend routers

**Covers:** [S4]

**Files:**
- Modify: All 10 router files

This is the bulk of the backend work. For each router:
1. Change `from backend.app.core.security import require_role` to `require_permission`
2. Remove `from backend.app.models.user import UserRole` (where no longer needed)
3. Replace each `Depends(require_role(...))` with `Depends(require_permission(...))`
4. Replace manual `user.role not in (UserRole.admin, UserRole.owner)` checks with permission checks

- [ ] **Step 1: server.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Endpoint change (line 21):
async def server_action(
    req: ServerActionRequest, _user: User = Depends(require_permission("SERVER_MANAGE"))
) -> ServerActionResponse:
```

- [ ] **Step 2: console.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Endpoint change (line 26):
async def send_command(
    req: ConsoleCommandRequest, _user: User = Depends(require_permission("CONSOLE_SEND"))
) -> ConsoleCommandResponse:
```

- [ ] **Step 3: players.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token

# Endpoint change (line 44):
async def player_action(
    req: PlayerActionRequest, user: User = Depends(require_permission("PLAYERS_KICK"))
) -> dict:
```

Note: `player_action` handles kick/ban/op — using `PLAYERS_KICK` as the gate since any mod+ can use all player commands.

- [ ] **Step 4: properties.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Endpoint changes:
async def save_properties_raw(body: dict, user: User = Depends(require_permission("PROPERTIES_EDIT"))):
...
async def update_property(key: str, body: dict, user: User = Depends(require_permission("PROPERTIES_EDIT"))):
...
```

Remove the `UserRole = UserRole.viewer` default that was at line 34.

- [ ] **Step 5: addons.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Endpoint changes:
async def update_manifest(
    req: ManifestUpdateRequest, user: User = Depends(require_permission("ADDONS_MANAGE"))
) -> dict:
...
async def set_pack_order(
    world: str, pack_type: str, req: AddonReorderRequest, user: User = Depends(require_permission("ADDONS_MANAGE"))
):
```

- [ ] **Step 6: files.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Endpoint changes:
async def write_file(filename: str, body: dict, _user: User = Depends(require_permission("FILES_EDIT"))):
...
async def delete_file(filename: str, _user: User = Depends(require_permission("FILES_EDIT"))):
```

- [ ] **Step 7: auth.py**

```python
# Import changes:
from backend.app.core.security import require_permission, verify_token
from backend.app.core.auth import get_user_permissions  # new import

# Endpoint changes:
async def get_users(current_user: User = Depends(require_permission("USERS_VIEW"))) -> list[dict]:
...
async def create_user(... current_user: User = Depends(require_permission("USERS_CREATE"))):
...
async def update_user(... current_user: User = Depends(require_permission("USERS_EDIT"))):
...
async def delete_user(... current_user: User = Depends(require_permission("USERS_DELETE"))):
...

# Add GET /auth/me endpoint (already defined in Task 3)
```

Also update `create_user`/`update_user` function signatures in auth.py to accept `role: str` instead of `UserRole`.

- [ ] **Step 8: settings.py**

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Endpoint changes:
async def update_settings(... user: User = Depends(require_permission("SETTINGS_EDIT"))):
```

- [ ] **Step 9: backups.py**

Backups router has a mix of verify_token-only endpoints (destructive ops that need proper gates) and require_role endpoints (admin/owner for settings).

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
# Remove UserRole import

# Existing require_role calls → require_permission:
# Line 114 (update backup settings):
user: User = Depends(require_permission("BACKUPS_CREATE")),
# Line 124 (update backup settings):
user: User = Depends(require_permission("BACKUPS_CREATE")),
# Line 164 (include items):
_u: User = Depends(require_permission("BACKUPS_CREATE")),
# Line 186 (scheduler config):
user: User = Depends(require_permission("BACKUPS_CREATE")),
```

Note: Backups has destructive endpoints (create_backup, restore_backup, delete_backup) that currently only use `verify_token`. We'll add `require_permission("BACKUPS_CREATE")` etc. to gate them.

- [ ] **Step 10: teams.py**

The teams router has both `Depends(require_role(...))` global gates AND manual `user.role not in (UserRole.admin, UserRole.owner)` inline checks.

```python
# Import change:
from backend.app.core.security import require_permission, verify_token
from backend.app.core.roles import resolve_permissions  # needed for inline checks

# Global gates → require_permission:
# GET /teams/, GET /teams/{name}: any auth (keep verify_token)
# POST /teams/: require_permission("TEAMS_CREATE")
# PUT /teams/{name}: require_permission("TEAMS_MANAGE")
# DELETE /teams/{name}: require_permission("TEAMS_MANAGE")
# POST /teams/{name}/members: require_permission("TEAMS_MANAGE")
# DELETE /teams/{name}/members/{username}: require_permission("TEAMS_MANAGE")
# PUT /teams/{name}/members/{username}: require_permission("TEAMS_MANAGE")

# Replace inline checks user.role not in (UserRole.admin, UserRole.owner) with:
# has_global_manage = user.role == "owner" or "TEAMS_MANAGE" in resolve_permissions(user.role)
```

Create a helper for the team-internal checks:

```python
def _can_manage_team(user: User, team: Team) -> bool:
    """Check if user can manage a specific team."""
    if user.role == "owner":
        return True
    if "TEAMS_MANAGE" in resolve_permissions(user.role):
        return True
    if user.username == team.owner:
        return True
    if user.username in (team.members or {}):
        return team.members[user.username] == "admin"
    return False


def _is_team_owner_or_global(user: User, team: Team) -> bool:
    """Check if user is team owner or global admin/owner."""
    if user.role == "owner":
        return True
    if "TEAMS_MANAGE" in resolve_permissions(user.role):
        return True
    return user.username == team.owner
```

---

### Task 7: Register in main.py

**Covers:** [S2]

**Files:**
- Modify: `backend/app/main.py`

- [ ] **Step 1: Add init_roles() and roles router**

```python
# At top:
from backend.app.core.roles import init_roles
from backend.app.routers import roles

# In lifespan (before yield):
init_roles()

# After other routers:
app.include_router(roles.router, prefix="/api/v1")
```

---

### Task 8: Frontend types, API, and permission store

**Covers:** [S5]

**Files:**
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/lib/api/client.ts`
- Create: `frontend/src/lib/stores/permissions.ts`

- [ ] **Step 1: Add Role type**

In `frontend/src/types/index.ts`:

```typescript
export interface Role {
  name: string;
  display_name: string;
  permissions: string[];
  is_default: boolean;
  created_at: string;
}
```

- [ ] **Step 2: Add API methods to client.ts**

```typescript
// --- Roles ---
export async function listRoles(): Promise<Role[]> {
  return request<Role[]>("/roles/");
}

export async function getRole(name: string): Promise<Role> {
  return request<Role>(`/roles/${name}`);
}

export async function createRole(data: Partial<Role>): Promise<Role> {
  return request<Role>("/roles/", { method: "POST", body: JSON.stringify(data) });
}

export async function updateRole(name: string, data: Partial<Role>): Promise<Role> {
  return request<Role>(`/roles/${name}`, { method: "PUT", body: JSON.stringify(data) });
}

export async function deleteRole(name: string): Promise<void> {
  await request(`/roles/${name}`, { method: "DELETE" });
}

// --- Auth ---
export interface MeResponse {
  user: UserInfo;
  permissions: string[];
}

export async function getMe(): Promise<MeResponse> {
  return request<MeResponse>("/auth/me");
}

// Ensure UserInfo has 'role' as string:
export interface UserInfo {
  username: string;
  display_name: string;
  role: string;
  created_at: string;
  last_login: string;
}
```

- [ ] **Step 3: Create permission store**

`frontend/src/lib/stores/permissions.ts`:

```typescript
import { writable, derived } from "svelte/store";

export const currentPermissions = writable<string[]>([]);

export function hasPermission(perm: string): boolean {
  let perms: string[] = [];
  currentPermissions.subscribe((v) => (perms = v))();
  return perms.includes(perm);
}

export const permissionDerived = derived(currentPermissions, ($p) => ({
  has: (perm: string) => $p.includes(perm),
}));
```

---

### Task 9: Frontend layout + sidebar guards

**Covers:** [S5]

**Files:**
- Modify: `frontend/src/routes/+layout.svelte`
- Modify: `frontend/src/lib/components/layout/Sidebar.svelte`

- [ ] **Step 1: Fetch permissions in +layout.svelte**

On auth state change (or on app load if logged in), call `getMe()` and populate the permissions store:

```typescript
import { currentPermissions } from "$lib/stores/permissions";
import { getToken, getMe } from "$lib/api/client";

// After login / on mount if token exists:
async function loadPermissions() {
  const token = getToken();
  if (!token) return;
  try {
    const me = await getMe();
    currentPermissions.set(me.permissions);
  } catch { /* ignore */ }
}
```

- [ ] **Step 2: Update sidebar nav visibility**

Each sidebar entry gets a `requiredPermission` field. The nav entry is only shown if the user has that permission.

```typescript
// In Sidebar.svelte, add permission field to nav items:
const navItems = [
  { href: "/console", icon: Terminal, label: "Console", requiredPermission: "CONSOLE_VIEW" },
  { href: "/players", icon: Users, label: "Players", requiredPermission: "PLAYERS_VIEW" },
  { href: "/properties", icon: Settings, label: "Properties", requiredPermission: "PROPERTIES_VIEW" },
  { href: "/addons", icon: Puzzle, label: "Addons", requiredPermission: "ADDONS_VIEW" },
  { href: "/backups", icon: HardDrive, label: "Backups", requiredPermission: "BACKUPS_VIEW" },
  { href: "/teams", icon: Group, label: "Teams", requiredPermission: "TEAMS_VIEW" },
  { href: "/users", icon: UserCog, label: "Users", requiredPermission: "USERS_VIEW" },
  { href: "/files", icon: FileText, label: "Files", requiredPermission: "FILES_VIEW" },
  { href: "/audit", icon: ClipboardList, label: "Audit", requiredPermission: "AUDIT_VIEW" },
  { href: "/settings", icon: Cog, label: "Settings", requiredPermission: "SETTINGS_VIEW" },
  { href: "/roles", icon: Shield, label: "Roles", requiredPermission: "USERS_VIEW" },
];

// Template changes:
{#if $currentPermissions.includes(item.requiredPermission)}
  <a href={item.href}>...</a>
{/if}
```

Also subscribe to the permissions store at the top of the component:
```typescript
import { currentPermissions } from "$lib/stores/permissions";
```

---

### Task 10: Roles management page

**Covers:** [S5]

**Files:**
- Create: `frontend/src/routes/roles/+page.svelte`

- [ ] **Step 1: Build the roles page**

A table of roles with inline permission toggle grid. Each row shows:
- Role name + display_name
- Click to expand: grid of all 24 permissions as checkboxes/toggles
- Edit button (inline edit of display_name + permission toggles)
- Delete button (disabled for built-in roles)
- Create button (opens modal to define new role name + permissions)

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { listRoles, createRole, updateRole, deleteRole, getMe } from "$lib/api/client";
  import { currentPermissions } from "$lib/stores/permissions";
  import type { Role } from "$lib/types";
  import { addToast } from "$lib/stores/toast";

  // All known permissions (same order as backend)
  const ALL_PERMS = [
    { id: "CONSOLE_VIEW", label: "View Console" },
    { id: "CONSOLE_SEND", label: "Send Commands" },
    { id: "SERVER_VIEW", label: "View Server" },
    { id: "SERVER_MANAGE", label: "Manage Server" },
    { id: "PLAYERS_VIEW", label: "View Players" },
    { id: "PLAYERS_KICK", label: "Kick Players" },
    { id: "PLAYERS_BAN", label: "Ban Players" },
    { id: "PLAYERS_OP", label: "Op Players" },
    { id: "PROPERTIES_VIEW", label: "View Properties" },
    { id: "PROPERTIES_EDIT", label: "Edit Properties" },
    { id: "ADDONS_VIEW", label: "View Addons" },
    { id: "ADDONS_MANAGE", label: "Manage Addons" },
    { id: "BACKUPS_VIEW", label: "View Backups" },
    { id: "BACKUPS_CREATE", label: "Create Backups" },
    { id: "BACKUPS_RESTORE", label: "Restore Backups" },
    { id: "BACKUPS_DELETE", label: "Delete Backups" },
    { id: "USERS_VIEW", label: "View Users" },
    { id: "USERS_CREATE", label: "Create Users" },
    { id: "USERS_EDIT", label: "Edit Users" },
    { id: "USERS_DELETE", label: "Delete Users" },
    { id: "TEAMS_VIEW", label: "View Teams" },
    { id: "TEAMS_CREATE", label: "Create Teams" },
    { id: "TEAMS_MANAGE", label: "Manage Teams" },
    { id: "FILES_VIEW", label: "View Files" },
    { id: "FILES_EDIT", label: "Edit Files" },
    { id: "AUDIT_VIEW", label: "View Audit" },
    { id: "SETTINGS_VIEW", label: "View Settings" },
    { id: "SETTINGS_EDIT", label: "Edit Settings" },
  ];

  let roles: Role[] = [];
  let loading = true;
  let editing: string | null = null;
  let editPerms: string[] = [];
  let showCreate = false;
  let newRole = { name: "", display_name: "", permissions: [] as string[] };

  onMount(async () => {
    roles = await listRoles();
    loading = false;
  });

  function togglePerm(perm: string) {
    if (editPerms.includes(perm)) {
      editPerms = editPerms.filter((p) => p !== perm);
    } else {
      editPerms = [...editPerms, perm];
    }
  }

  async function saveRole(name: string) {
    await updateRole(name, { permissions: editPerms });
    roles = await listRoles();
    editing = null;
    addToast("Role updated", "success");
  }

  async function handleCreate() {
    await createRole({
      name: newRole.name,
      display_name: newRole.display_name,
      permissions: newRole.permissions,
    });
    roles = await listRoles();
    showCreate = false;
    newRole = { name: "", display_name: "", permissions: [] };
    addToast("Role created", "success");
  }

  async function handleDelete(name: string) {
    if (!confirm(`Delete role "${name}"?`)) return;
    await deleteRole(name);
    roles = await listRoles();
    addToast("Role deleted", "success");
  }
</script>

<template>
  <div class="p-6">
    <div class="flex justify-between items-center mb-6">
      <h1 class="text-2xl font-bold">Roles</h1>
      {#if $currentPermissions.includes("USERS_CREATE")}
        <button onclick={() => showCreate = true} class="btn-primary">Create Role</button>
      {/if}
    </div>

    {#if loading}
      <p class="text-gray-400">Loading roles...</p>
    {:else}
      <div class="space-y-3">
        {#each roles as role (role.name)}
          <div class="bg-gray-800 rounded-lg p-4">
            <div class="flex justify-between items-center">
              <div>
                <h3 class="font-semibold">{role.display_name}</h3>
                <p class="text-sm text-gray-400">{role.name}{role.is_default ? " (default)" : ""}</p>
              </div>
              <div class="flex gap-2">
                {#if $currentPermissions.includes("USERS_EDIT")}
                  <button onclick={() => { editing = role.name; editPerms = [...role.permissions]; }} class="btn-ghost">Edit</button>
                {/if}
                {#if $currentPermissions.includes("USERS_DELETE") && role.name !== "owner"}
                  <button onclick={() => handleDelete(role.name)} class="btn-ghost text-red-400">Delete</button>
                {/if}
              </div>
            </div>

            {#if editing === role.name}
              <div class="mt-4 border-t border-gray-700 pt-4">
                <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-2">
                  {#each ALL_PERMS as perm}
                    <label class="flex items-center gap-2 text-sm cursor-pointer">
                      <input
                        type="checkbox"
                        checked={editPerms.includes(perm.id)}
                        onchange={() => togglePerm(perm.id)}
                      />
                      {perm.label}
                    </label>
                  {/each}
                </div>
                <div class="mt-4 flex gap-2">
                  <button onclick={() => saveRole(role.name)} class="btn-primary">Save</button>
                  <button onclick={() => editing = null} class="btn-ghost">Cancel</button>
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  {#if showCreate}
    <div class="fixed inset-0 bg-black/50 flex items-center justify-center z-50" onclick={() => showCreate = false} role="dialog" aria-modal="true">
      <div class="bg-gray-800 rounded-lg p-6 w-full max-w-md" onclick={(e) => e.stopPropagation()}>
        <h2 class="text-lg font-semibold mb-4">Create Role</h2>
        <div class="space-y-4">
          <div>
            <label class="block text-sm mb-1">Name</label>
            <input bind:value={newRole.name} class="input-field w-full" placeholder="e.g. helper" />
          </div>
          <div>
            <label class="block text-sm mb-1">Display Name</label>
            <input bind:value={newRole.display_name} class="input-field w-full" placeholder="e.g. Helper" />
          </div>
        </div>
        <div class="mt-6 flex gap-2 justify-end">
          <button onclick={() => showCreate = false} class="btn-ghost">Cancel</button>
          <button onclick={handleCreate} class="btn-primary" disabled={!newRole.name}>Create</button>
        </div>
      </div>
    </div>
  {/if}
</template>
```

---

### Task 11: Frontend page-level permission visibility

**Covers:** [S5]

**Files:**
- Modify: `frontend/src/routes/users/+page.svelte`
- Modify: `frontend/src/routes/teams/+page.svelte`

- [ ] **Step 1: Users page — dynamic role selector + permission gates**

In users page:
- Replace hardcoded role dropdown (`<select>` with owner/admin/moderator/viewer) with dynamic roles fetched from API
- Gate the create/edit/delete buttons: only show if user has the corresponding `USERS_CREATE`/`USERS_EDIT`/`USERS_DELETE` permission
- Gate the delete icon per row

```typescript
import { listRoles } from "$lib/api/client";
import { currentPermissions } from "$lib/stores/permissions";

let availableRoles: Role[] = [];

onMount(async () => {
  availableRoles = await listRoles();
});
```

Template:
```svelte
{#if $currentPermissions.includes("USERS_CREATE")}
  <button onclick={() => showCreateModal = true}>Create User</button>
{/if}
```

- [ ] **Step 2: Teams page — permission gates**

In teams page:
- Gate "Create Team" button behind `TEAMS_CREATE`
- Gate edit/delete per team behind `TEAMS_MANAGE`
- Keep team-internal admin/owner checks (those stay as-is — they're about team membership, not global permissions)

---

### Task 12: Console page — send command gate

**Covers:** [S5]

**Files:**
- Modify: `frontend/src/routes/console/+page.svelte`

- [ ] **Step 1: Gate the command input**

```svelte
{#if $currentPermissions.includes("CONSOLE_SEND")}
  <div class="flex gap-2">
    <input bind:value={command} placeholder="Type a command..." class="flex-1 ..." />
    <button onclick={sendCommand} class="btn-primary">Send</button>
  </div>
{:else}
  <p class="text-gray-400 text-sm">You don't have permission to send commands.</p>
{/if}
```

---

### Task 13: Verification

**Covers:** [S4, S5]

**Files:**
- Test: `backend/tests/`

- [ ] **Step 1: Run existing tests to check for regressions**

```bash
uv run pytest -q
```
Expected: 21/21 pass (or same count, no regressions)

- [ ] **Step 2: Fix any test failures**

Key things that may break:
- Tests that create users with `UserRole.admin` → need to use the string `"admin"` instead
- Tests that check `user.role` value → now returns str instead of UserRole enum
- Tests that call `create_user()` with `UserRole` arg → need to pass str

- [ ] **Step 3: Run ruff lint**

```bash
uvx ruff check backend/app/
```
Expected: no new warnings on our code (pre-existing F401 and line-length warnings tolerated)

- [ ] **Step 4: Run svelte-check**

```bash
npm run check
```
Expected: no errors (a11y warnings tolerated)
