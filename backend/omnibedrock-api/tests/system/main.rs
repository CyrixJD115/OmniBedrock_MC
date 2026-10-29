//! Crate root of the `system` integration test target: declares the shared
//! harness and the per-API-area test modules. The whole suite is Unix-only.
#![cfg(unix)]

mod addons;
mod auth;
mod backups;
mod bds;
mod broadcaster;
mod console;
mod docker;
mod harness;
mod instances;
mod process;
mod properties;
mod roles;
