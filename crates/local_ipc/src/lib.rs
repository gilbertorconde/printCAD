//! Local sockets named by a path, on every platform: the document server,
//! the MCP relay and the agent bridge all speak over one.
//!
//! On Unix these are the standard library's UNIX sockets; on Windows the
//! system's own AF_UNIX sockets (Windows 10 and later), which take the same
//! calls. A path is at most about a hundred bytes on every system, so
//! socket names are kept short.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};
#[cfg(windows)]
pub use uds_windows::{UnixListener as Listener, UnixStream as Stream};

/// Where the application's sockets and their logs live: the user's runtime
/// directory where the system has one, else the temporary directory.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("printcad")
}

/// The program that hands a file to the application the system has for its
/// type.
pub const SYSTEM_OPENER: &str = if cfg!(target_os = "macos") {
    "open"
} else if cfg!(windows) {
    "explorer"
} else {
    "xdg-open"
};

/// Opens `path` with the application the system has for its type (a folder
/// in the file manager).
pub fn open_with_system(path: &Path) -> std::io::Result<()> {
    background(Command::new(SYSTEM_OPENER).arg(path))
        .spawn()
        .map(|_| ())
}

/// Readies a command for a helper process the application talks to over
/// pipes or sockets: on Windows it gets no console window of its own.
pub fn background(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW`.
        const NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(NO_WINDOW);
    }
    command
}

/// The program a command names. On Windows a bare name also finds the
/// scripts (`.cmd`, `.bat`) package managers install as commands, through
/// `PATHEXT`, which starting a process by name alone passes over; elsewhere
/// the name is kept for the system to look up.
pub fn find_program(command: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let bare = Path::new(command);
        if bare.extension().is_none() && bare.components().count() == 1 {
            let extensions =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
            let dirs = std::env::var_os("PATH").unwrap_or_default();
            for dir in std::env::split_paths(&dirs) {
                for extension in extensions.split(';').filter(|e| !e.is_empty()) {
                    let candidate =
                        dir.join(format!("{command}{}", extension.to_ascii_lowercase()));
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }
        }
    }
    PathBuf::from(command)
}

/// The file name of one of the application's own programs on this system.
pub fn program_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    #[test]
    fn a_listener_takes_a_client_by_its_path() {
        let dir = std::env::temp_dir().join(format!("local-ipc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let _ = std::fs::remove_file(&path);
        let listener = Listener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut got = [0u8; 5];
            stream.read_exact(&mut got).unwrap();
            stream.write_all(&got).unwrap();
        });
        let mut client = Stream::connect(&path).unwrap();
        client.write_all(b"hello").unwrap();
        let mut back = [0u8; 5];
        client.read_exact(&mut back).unwrap();
        assert_eq!(&back, b"hello");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pair_carries_both_ways() {
        let (mut a, mut b) = Stream::pair().unwrap();
        a.write_all(b"x").unwrap();
        let mut got = [0u8; 1];
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"x");
    }
}
