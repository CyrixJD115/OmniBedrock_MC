use clap::Parser;
use omnibedrock_core::config::Cli;

fn main() -> anyhow::Result<()> {
    // A malformed .env must never pass silently: one bad line aborts the whole
    // file, and defaults would then scatter data dirs relative to the CWD.
    if let Err(e) = dotenvy::dotenv() {
        eprintln!("warning: failed to load .env ({e}); using defaults and process environment");
    }
    let cli = Cli::parse();
    if cli.desktop {
        #[cfg(feature = "desktop")]
        {
            return omnibedrock_api::desktop::run(cli);
        }
        #[cfg(not(feature = "desktop"))]
        {
            anyhow::bail!(
                "This binary was built without the desktop feature.\n\
                 Rebuild with: cargo build --release --features desktop"
            );
        }
    }
    omnibedrock_api::server_proc::run_standalone(cli)
}
