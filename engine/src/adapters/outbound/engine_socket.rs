//! Which process is the engine, and how a returning client reaches it.
//!
//! A-ENGINELIFE: an engine that loses its client does not exit, so a later invocation on the same
//! host must find it rather than become a second one. A unix socket is the smallest thing that
//! does: local to the instance, needing no port -- §1.2 allows only 22 -- and already how
//! `ControlPath` works for ssh itself.
//!
//! **The whole of the decision is in `claim`.** Binding succeeds: this process is the engine.
//! Binding fails against a socket something answers: this process is a proxy. Binding fails
//! against a socket nothing answers: the previous engine died and left it behind.
//!
//! Telling the last two apart is why this connects before it unlinks. A bind failure alone says
//! only that a file is in the way, and unlinking on that basis would let a second engine start
//! while the first is serving -- two engines on one host, each with half the tasks, and a client
//! reaching whichever it happened to bind.

use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The directory the socket lives in, created `0700`.
///
/// The socket is a **full control channel**: anything that can connect to it runs commands as
/// this user. The filesystem is what enforces that boundary, so the directory is the user's and
/// the mode is not decoration. A-EC2's single tenancy means there is no second user on the
/// instance to defend against today, which bounds the exposure without making the permissions
/// optional -- a future multi-tenant instance finds them already correct rather than needing them
/// added afterwards.
const DIR_MODE: u32 = 0o700;
/// The socket itself, `0600`, for the same reason.
const SOCKET_MODE: u32 = 0o600;

/// What this process turned out to be.
pub enum Claim {
    /// This process is the engine. It serves its own stdio and anything that connects here.
    Bound(UnixListener),
    /// Another process is the engine, and this is the connection to it.
    Proxy(UnixStream),
}

/// Where the socket lives, honouring `XDG_RUNTIME_DIR` when the host sets one.
///
/// `XDG_RUNTIME_DIR` is the right home for a socket -- it is per user, already `0700`, and cleaned
/// up on logout -- and it is frequently absent on a minimal instance, which is why there is a
/// fallback rather than a failure.
/// Overrides the path entirely. One engine per host is the production rule; a test suite is many
/// engines on one host, each of which must be its own.
pub const SOCKET_ENV: &str = "APEX_ENGINE_SOCKET";

pub fn default_path() -> PathBuf {
    // An explicit path wins. Without it every engine a suite spawns proxies to the first, which
    // is the singleton behaving correctly and the tests measuring something else entirely.
    if let Some(explicit) = std::env::var_os(SOCKET_ENV) {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".apex")
        });
    base.join("apex-engine").join("engine.sock")
}

/// Become the engine, or find the one that already is.
///
/// Retries the bind **once** after removing a stale socket, and not in a loop: a loop against a
/// socket somebody else is racing to create never terminates, and the second failure is better
/// reported than retried.
pub fn claim(socket: &Path) -> io::Result<Claim> {
    if let Some(dir) = socket.parent() {
        prepare_directory(dir)?;
    }

    match bind(socket) {
        Ok(listener) => Ok(Claim::Bound(listener)),
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => match UnixStream::connect(socket) {
            // Something is listening: it is the engine and this is not.
            Ok(stream) => Ok(Claim::Proxy(stream)),
            // Nothing is listening, so the file is what an engine that died left behind.
            Err(_) => {
                std::fs::remove_file(socket)?;
                bind(socket).map(Claim::Bound)
            }
        },
        Err(e) => Err(e),
    }
}

/// Make sure the socket's directory exists and is private, **without changing one we did not
/// create**.
///
/// The distinction is not fussiness. An earlier version of this chmod'd the parent
/// unconditionally, and a socket path of `/tmp/x.sock` made that `chmod 0700 /tmp` -- which fails
/// as an ordinary user and, running as root, breaks every other program on the host. A process
/// does not get to tighten a directory it does not own just because it would like to put
/// something in it.
///
/// So: created here, and this sets the mode. Already there, and this **checks** it and refuses
/// rather than modifying. Refusing is the safe half of that trade -- the directory the engine
/// made for itself is already `0700`, so the only thing refused is a location somebody chose that
/// would expose a full control channel.
fn prepare_directory(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(DIR_MODE).create(dir) {
        // We made it, so we set its mode, and `DirBuilder` already did.
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let mode = std::fs::metadata(dir)?.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "{} is mode {mode:o}; the engine's socket directory must not be readable \
                         or writable by anyone but its owner",
                        dir.display()
                    ),
                ));
            }
            Ok(())
        }
        // A missing parent of the parent. Create the chain, then retry so the leaf still gets its
        // own mode rather than inheriting whatever `recursive` would have used.
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Some(above) = dir.parent() {
                std::fs::create_dir_all(above)?;
            }
            std::fs::DirBuilder::new().mode(DIR_MODE).create(dir)
        }
        Err(e) => Err(e),
    }
}

fn bind(socket: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(SOCKET_MODE))?;
    Ok(listener)
}

/// Remove the socket on the way out.
///
/// An engine that exited leaving its socket behind would make the next invocation pay for a
/// connect-then-unlink before it could start. That path exists because a crash cannot run this,
/// not because an ordinary exit should rely on it.
pub fn release(socket: &Path) {
    let _ = std::fs::remove_file(socket);
}
