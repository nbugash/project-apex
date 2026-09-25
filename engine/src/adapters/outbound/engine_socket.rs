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
pub fn default_path() -> PathBuf {
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
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(dir)
            .or_else(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        // `recursive(true)` does not apply the mode to a directory that already existed, and a
        // directory left `0755` by an earlier version would be a control channel anyone on the
        // host could reach. Set it either way.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(DIR_MODE))?;
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
