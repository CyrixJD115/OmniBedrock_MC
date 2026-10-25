use clap::Parser;
use omnibedrock_core::config::Cli;

fn main() -> anyhow::Result<()> {
    // Hidden subcommand executed inside systemd instance units (see
    // omnibedrock-server::systemd). Intercepted before clap so the public
    // CLI surface stays unchanged.
    let mut args = std::env::args_os();
    if args.nth(1).as_deref() == Some(std::ffi::OsStr::new("__unit-helper")) {
        let rest: Vec<std::ffi::OsString> = args.collect();
        let code = omnibedrock_server::unit_helper::run(&rest).unwrap_or_else(|e| {
            eprintln!("__unit-helper: {e:#}");
            127
        });
        std::process::exit(code);
    }
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
