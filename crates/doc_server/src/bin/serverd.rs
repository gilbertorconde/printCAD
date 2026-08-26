//! `printcad-serverd` — the local document server, one instance per
//! document. Spawned by the app (or by hand for debugging):
//!
//! ```text
//! printcad-serverd --socket /run/user/1000/printcad/<key>.sock
//! ```
//!
//! Serves exactly one client and exits when it disconnects.

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let socket = match (args.next().as_deref(), args.next()) {
        (Some("--socket"), Some(path)) => std::path::PathBuf::from(path),
        _ => {
            eprintln!("usage: printcad-serverd --socket <path>");
            std::process::exit(2);
        }
    };
    doc_server::daemon::run(&socket)
}
