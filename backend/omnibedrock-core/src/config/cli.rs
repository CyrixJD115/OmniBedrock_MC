//! Command-line interface definition: every flag the panel binary accepts,
//! including the environment-variable fallbacks clap resolves for each one.

use clap::Parser;
use std::net::IpAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about = "OmniBedrock native server control panel")]
pub struct Cli {
    /// Explicit configuration file (.toml or legacy .yaml). Overrides the
    /// auto-discovered one; the file must exist when given.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Portable mode: keep all application data under `<executable dir>/data/`
    /// and read only `<executable dir>/data/config.toml`. Mutually exclusive
    /// with --global-data-dir.
    #[arg(long)]
    pub portable: bool,
    /// Use this directory as the global data root (overriding the platform
    /// app-data location). Mutually exclusive with --portable.
    #[arg(long, env = "OMNI_GLOBAL_DATA_DIR")]
    pub global_data_dir: Option<PathBuf>,
    #[arg(long, env = "OMNI_HOST")]
    pub host: Option<IpAddr>,
    #[arg(long, env = "OMNI_PORT")]
    pub port: Option<u16>,
    #[arg(long, env = "BEDROCK_SERVER_DIR")]
    pub bedrock_server_dir: Option<PathBuf>,
    #[arg(long, env = "DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    #[arg(long, env = "BACKUPS_DIR")]
    pub backups_dir: Option<PathBuf>,
    #[arg(long, env = "LOGS_DIR")]
    pub logs_dir: Option<PathBuf>,
    /// Cache root (CurseForge image cache; default `<data dir>/cache`).
    #[arg(long, env = "CACHE_DIR")]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, env = "ENDSTONE_BIN")]
    pub endstone_bin: Option<PathBuf>,
    #[arg(long, env = "ENDSTONE_PYTHON")]
    pub endstone_python: Option<PathBuf>,
    #[arg(long, env = "BROADCASTER_DIR")]
    pub broadcaster_dir: Option<PathBuf>,
    #[arg(long, env = "BROADCASTER_JAR")]
    pub broadcaster_jar: Option<PathBuf>,
    #[arg(long, env = "JAVA_BIN")]
    pub java_bin: Option<PathBuf>,
    /// Docker image used for instances with the `docker` deployment backend.
    /// Missing local images are built automatically from the generated
    /// Dockerfiles (see deploy/docker).
    #[arg(long, env = "DOCKER_IMAGE_VANILLA")]
    pub docker_image_vanilla: Option<String>,
    #[arg(long, env = "DOCKER_IMAGE_ENDSTONE")]
    pub docker_image_endstone: Option<String>,
    /// Docker image used for the broadcaster sidecar container of `docker`
    /// instances (JRE + MCXboxBroadcastStandalone entrypoint). Missing local
    /// images are built automatically like the game images.
    #[arg(long, env = "DOCKER_IMAGE_BROADCASTER")]
    pub docker_image_broadcaster: Option<String>,
    /// Memory (MiB) kept free for the OS and the panel itself; instance
    /// allocations beyond total − reserve trigger overcommit warnings.
    #[arg(long, env = "HOST_RESERVE_MB")]
    pub host_reserve_mb: Option<u64>,
    /// Open the panel in a Tauri desktop window instead of the plain web
    /// server (requires the binary to be built with `--features desktop`).
    #[arg(long)]
    pub desktop: bool,
}
