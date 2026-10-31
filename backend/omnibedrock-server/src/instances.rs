//! Multi-instance management: instance records, resource policies, deployment
//! backends, per-instance service bundles, and the legacy→default migration.
//!
//! The panel used to manage exactly one server pointed at
//! `Config::bedrock_server_dir`. On boot that server becomes the `default`
//! instance and keeps every legacy path, so existing installations start
//! working unchanged. Instances created afterwards live under
//! `<data_dir>/instances/<id>/` with their own server files, settings,
//! console journal, and backups.
//!
//! Submodules, by responsibility:
//! - [`types`]: the persisted data model (records, policies, quotas, access).
//! - [`instance`]: the running `Instance` service bundle.
//! - [`manager`]: the `InstanceManager` registry and lifecycle operations.
//! - [`builder`]: constructing an instance's services from its record.
//! - [`capabilities`]: host backend capability probing.
//! - [`ports`]: port-pair model and server.properties synchronization.
//! - [`transport`]: `server-udp-ports` validation and transport guidance.
//! - [`helpers`]: id/slug rules, container naming, capacity warnings.
//! - [`consts`]: shared constants.

mod builder;
mod capabilities;
mod consts;
mod helpers;
mod instance;
mod manager;
mod ports;
mod transport;
mod types;

pub use builder::broadcaster_docker_spec;
pub use capabilities::DeployCapabilities;
pub use consts::DEFAULT_INSTANCE;
pub use helpers::{container_name, overcommit_status, slug, valid_id};
pub use instance::Instance;
pub use manager::InstanceManager;
pub use ports::{default_properties_text, enforce_instance_port, enforce_port_in_text};
pub(crate) use ports::{port_bindable, tcp_port_bindable};
pub use transport::{
    NETHERNET_MULTIPLEXER_VERSION, advise_transport, endstone_has_multiplexer,
    validate_udp_ports_value,
};
pub use types::{
    AccessEntry, Deployment, DockerConfig, InstanceRecord, ResourcePolicy, UserQuotas,
};
