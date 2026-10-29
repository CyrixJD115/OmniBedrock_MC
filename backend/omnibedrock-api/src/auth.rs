//! Authentication and authorization for the OmniBedrock panel, split by
//! responsibility:
//!
//! - `consts`: the permission catalog and the panel settings file name
//! - `types`: user/role records, the shared in-memory store, and small
//!   store-lookup helpers
//! - `password`: Argon2 hashing and verification (with a legacy PBKDF2 path)
//! - `seed`: built-in stock role definitions and idempotent seeding
//! - `store`: the `Auth` service itself — construction, JWT secret bootstrap
//!   and atomic persistence of the user/role stores
//! - `perms`: permission resolution and enforcement gates
//! - `token`: login, JWT issuing/verification, and acting-user resolution
//! - `settings`: login toggle, CurseForge key override, `panel_settings.yaml`
//! - `accounts`: user listing, quota lookup and account CRUD
//! - `roles`: role listing and role CRUD
//!
//! Every previously public item remains reachable from this module path.

mod accounts;
mod consts;
mod password;
mod perms;
mod roles;
mod seed;
mod settings;
mod store;
mod token;
mod types;

pub use consts::{INSTANCE_PERMISSIONS, PERMISSIONS};
pub use store::Auth;
pub use types::{Role, User};
