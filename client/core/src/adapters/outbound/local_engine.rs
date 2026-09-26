//! Run the engine as a child process on this machine, and speak its stdio.
//!
//! # Why this is a spawner and not a transport
//!
//! The engine speaks JSON-RPC over stdin and stdout and has no idea what carries them. `ssh`
//! is a pipe with a network in the middle; a child process is the same pipe without one. So
//! the whole of `SshTransport` -- framing, the correlation registry, timeouts, cancellation,
//! connection state, the send queue -- is exactly right for both, and the only thing that
//! differs is how the child is started. That difference is already a port
//! (`ProcessSpawner`), so this is an adapter behind it rather than a second transport.
//!
//! Writing a `LocalTransport` instead would have duplicated every one of those behaviours,
//! and the duplicate would have been the one without F001's tests.
//!
//! # What this is not
//!
//! **Not F015 `local-mode`.** That feature is about running a developer's tasks on their own
//! machine with no engine in the picture at all, and `LocalTasks` refuses on its behalf. This
//! runs the real engine, unchanged, and talks to it over the real protocol; the only thing
//! that is local is which machine the process is on. A task started through here gets the same
//! pseudo-terminal, the same limits and the same lifecycle as one on the instance.
//!
//! **Not a deployment path.** A-BOOT owns getting a binary onto a remote host. This one is
//! already here.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::application::ports::spawner::{ProcessSpawner, SpawnError, SpawnSpec, SpawnedChild};
use crate::domain::request::Secret;

/// The variable that names an engine binary to run locally.
///
/// Read by the composition root rather than here, on the same rule the remote target follows:
/// an adapter that reads its own configuration cannot be constructed two ways, which is the
/// property the composition root exists to protect.
pub const ENGINE_BINARY_VAR: &str = "APEX_LOCAL_ENGINE";

pub struct LocalEngineSpawner {
    binary: PathBuf,
    /// The live child, so `supply_passphrase` can refuse against something real and a dropped
    /// spawner does not leave an engine behind.
    child: Mutex<Option<std::process::Child>>,
}

impl LocalEngineSpawner {
    pub fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            child: Mutex::new(None),
        }
    }

    /// The binary the environment names, if it names one.
    ///
    /// No default, deliberately. The first version fell back to `target/debug/ide-engine` when
    /// the file existed, which meant every developer with a built workspace silently acquired a
    /// live transport -- and with it a status bar reporting a real connection instead of the
    /// stub, which is what F001's `connection-status` spec drives. It failed on a five-second
    /// budget naming neither the engine nor the cause. An engine is too large a thing to infer
    /// from a file being on disk.
    pub fn named() -> Option<PathBuf> {
        std::env::var(ENGINE_BINARY_VAR)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
}

impl ProcessSpawner for LocalEngineSpawner {
    /// Refuse at startup rather than at the first request, which is what FR-005 asks of the
    /// ssh spawner and is worth just as much here: "the engine is not built" and "the engine
    /// crashed" are different problems with different remedies, and a missing file discovered
    /// on the first keystroke looks like the second.
    fn preflight(&self) -> Result<String, SpawnError> {
        if !self.binary.exists() {
            return Err(SpawnError::NotFound);
        }
        Ok(format!("local engine at {}", self.binary.display()))
    }

    fn spawn(&self, _spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        // `spec` is ignored, and that is the honest thing rather than an oversight: host, user
        // and the assisted phase are all properties of reaching another machine. There is no
        // second machine and no credential, so there is nothing here for them to mean.
        let mut child = Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => SpawnError::NotFound,
                _ => SpawnError::Io(e.to_string()),
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            SpawnError::Io("the engine was started without a usable stdin".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            SpawnError::Io("the engine was started without a usable stdout".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            SpawnError::Io("the engine was started without a usable stderr".into())
        })?;

        // The handle is kept so `wait` has something to wait on and so dropping this spawner
        // does not orphan an engine. Taken back out here because `wait` consumes it.
        let mut held = self.child.lock().expect("child lock");
        if let Some(previous) = held.take() {
            // One engine at a time. A second spawn without the first having ended would leave
            // a process nothing reaps and nothing talks to.
            let mut previous = previous;
            let _ = previous.kill();
            let _ = previous.wait();
        }
        *held = Some(child);
        drop(held);

        let handle = self.child.lock().expect("child lock").take();
        Ok(SpawnedChild {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            wait: Box::new(move || {
                let mut handle = handle?;
                handle.wait().ok().and_then(|status| status.code())
            }),
        })
    }

    fn invocation(&self, _spec: &SpawnSpec) -> Vec<String> {
        vec![self.binary.display().to_string()]
    }

    /// Nothing. The engine inherits this process's environment, which is what a child on the
    /// same machine should do, and none of §3.3's askpass variables mean anything without an
    /// `ssh` to read them.
    fn environment(&self, _spec: &SpawnSpec) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Refused, and refused loudly rather than accepted and dropped.
    ///
    /// A passphrase offered to a local pipe has no recipient. Returning `Ok` would let a
    /// credential be handed over, zeroed and forgotten while the caller believed it had been
    /// used -- the one outcome worse than refusing.
    fn supply_passphrase(&self, _secret: Secret) -> Result<(), SpawnError> {
        Err(SpawnError::Io(
            "a local engine has no authentication to answer".into(),
        ))
    }
}

impl Drop for LocalEngineSpawner {
    fn drop(&mut self) {
        if let Ok(mut held) = self.child.lock() {
            if let Some(mut child) = held.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
