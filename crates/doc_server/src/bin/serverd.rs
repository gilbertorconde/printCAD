//! `printcad-serverd`: the local document server, one instance per
//! document. Spawned by the app (or by hand for debugging):
//!
//! ```text
//! printcad-serverd --socket /run/user/1000/printcad/<key>.sock [--queue-limit <messages>] [--log-bytes <bytes>]
//! ```
//!
//! Serves every client that connects and exits when the last one leaves.

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let usage = || -> ! {
        eprintln!(
            "usage: printcad-serverd --socket <path> [--queue-limit <messages>] [--log-bytes <bytes>]"
        );
        std::process::exit(2);
    };
    let mut socket = None;
    let mut limits = doc_server::daemon::Limits::default();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--socket", Some(path)) => socket = Some(std::path::PathBuf::from(path)),
            ("--queue-limit", Some(limit)) => {
                limits.queue = limit.parse().unwrap_or_else(|_| usage());
            }
            ("--log-bytes", Some(bytes)) => {
                limits.log_bytes = bytes.parse().unwrap_or_else(|_| usage());
            }
            _ => usage(),
        }
    }
    let Some(socket) = socket else { usage() };
    doc_server::daemon::run_with(&socket, limits)
}
