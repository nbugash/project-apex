// This module is compiled into each integration test binary separately, so every double
// the *other* binaries use reads as dead code here. Silencing it is the standard cost of
// `tests/common/`; the alternative is a helper crate for six doubles.
#![allow(dead_code)]
// These doubles block dedicated threads on purpose — reaping a child, waking a parked
// writer. The crate-wide ban on `std::thread::sleep` guards the interaction path, and
// nothing here runs on it.
#![allow(clippy::disallowed_methods)]

//! Shared test doubles.
//!
//! Placed here rather than in `tests/mock_daemon/` as tasks.md first said: that target is
//! `harness = false`, so it is a binary the transport spawns, and nothing can import from
//! it. Integration tests are separate crates, and `tests/common/` is how Rust shares code
//! between them.

use apex_shell::application::ports::spawner::{
    ProcessSpawner, SpawnError, SpawnSpec, SpawnedChild,
};
use apex_shell::domain::request::Secret;
use std::sync::{Arc, Mutex};

/// Produces a chosen exit code and chosen stderr without running `ssh`.
///
/// This is the seam that makes failure classification testable. Provoking a changed host
/// key, a refused credential and a missing engine from a real `ssh` on demand would need a
/// remote host that will do each of those things — three hosts, or one repeatedly
/// reconfigured, over a network the suite is required not to need.
/// Recorded invocations or environments, one entry per attempt.
pub type Recorded<T> = Arc<Mutex<Vec<T>>>;

pub struct ScriptedSpawner {
    pub exit_code: i32,
    pub stderr: String,
    pub version: Result<String, SpawnError>,
    /// Every passphrase handed to it, so a test can assert one was or was not requested.
    pub passphrases: Arc<Mutex<Vec<usize>>>,
    /// Records each invocation, so a test can assert the §3.1 flags are present.
    pub invocations: Recorded<Vec<String>>,
    /// Records each environment, so a test can assert §3.3's askpass settings.
    pub environments: Recorded<Vec<(String, String)>>,
}

impl Default for ScriptedSpawner {
    fn default() -> Self {
        Self {
            exit_code: 0,
            stderr: String::new(),
            version: Ok("OpenSSH_9.6p1".into()),
            passphrases: Arc::new(Mutex::new(Vec::new())),
            invocations: Arc::new(Mutex::new(Vec::new())),
            environments: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ScriptedSpawner {
    pub fn failing(exit_code: i32, stderr: &str) -> Self {
        Self {
            exit_code,
            stderr: stderr.into(),
            ..Default::default()
        }
    }

    pub fn with_version(version: Result<String, SpawnError>) -> Self {
        Self {
            version,
            ..Default::default()
        }
    }

    pub fn passphrase_count(&self) -> usize {
        self.passphrases.lock().expect("passphrase lock").len()
    }

    /// Every attempt's invocation, oldest first.
    pub fn invocations(&self) -> Vec<Vec<String>> {
        self.invocations.lock().expect("invocation lock").clone()
    }

    /// Every attempt's environment, oldest first.
    pub fn environments(&self) -> Vec<Vec<(String, String)>> {
        self.environments.lock().expect("environment lock").clone()
    }
}

impl ProcessSpawner for ScriptedSpawner {
    fn preflight(&self) -> Result<String, SpawnError> {
        match &self.version {
            Ok(v) => Ok(v.clone()),
            Err(SpawnError::NotFound) => Err(SpawnError::NotFound),
            Err(SpawnError::TooOld { found, required }) => Err(SpawnError::TooOld {
                found: found.clone(),
                required: required.clone(),
            }),
            Err(SpawnError::Io(m)) => Err(SpawnError::Io(m.clone())),
        }
    }

    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        self.invocations
            .lock()
            .expect("invocation lock")
            .push(self.invocation(spec));
        self.environments
            .lock()
            .expect("environment lock")
            .push(self.environment(spec));

        // A child that started and then failed, which is what actually happens: `ssh`
        // spawns fine and then exits 255. Returning a spawn error instead would model a
        // missing binary, and no amount of stderr would ever be classified.
        let (code, stderr) = (self.exit_code, self.stderr.clone());
        Ok(SpawnedChild {
            stdin: Box::new(std::io::sink()),
            // Immediate EOF: the engine never spoke.
            stdout: Box::new(std::io::empty()),
            stderr: Box::new(std::io::Cursor::new(stderr.into_bytes())),
            wait: Box::new(move || Some(code)),
        })
    }

    fn environment(&self, spec: &SpawnSpec) -> Vec<(String, String)> {
        let mut env = vec![("LC_ALL".to_string(), "C".to_string())];
        if spec.assisted {
            env.push((
                "SSH_ASKPASS".into(),
                "/opt/apex/libexec/apex-askpass".into(),
            ));
            env.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
        }
        env
    }

    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        // Mirrors the real spawner's shape closely enough for flag assertions.
        let mut v: Vec<String> = [
            "ssh",
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPath=~/.ssh/apex-%C",
            "-o",
            "ControlPersist=1h",
            "-o",
            "IPQoS=throughput",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "StrictHostKeyChecking=accept-new",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if !spec.assisted {
            v.push("-o".into());
            v.push("BatchMode=yes".into());
        }
        v.push(format!("{}@{}", spec.user, spec.host));
        v
    }

    fn supply_passphrase(&self, secret: Secret) -> Result<(), SpawnError> {
        self.passphrases
            .lock()
            .expect("passphrase lock")
            .push(secret.len());
        Ok(())
    }
}

/// Scripted stderr fixtures, as OpenSSH emits them under `LC_ALL=C`.
pub mod stderr {
    pub const TIMED_OUT: &str = "ssh: connect to host example.com port 22: Connection timed out";
    pub const REFUSED: &str = "ssh: connect to host example.com port 22: Connection refused";
    pub const DENIED: &str = "user@example.com: Permission denied (publickey,password).";
    pub const HOST_KEY_CHANGED: &str = concat!(
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n",
        "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n",
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@"
    );
    pub const NOT_FOUND: &str = "bash: /usr/local/bin/ide-engine: No such file or directory";
    /// The same refusal a French system emits. Classification must be identical (SC-008),
    /// which is what `LC_ALL=C` in the invocation is for — if it were ever dropped, this is
    /// what the transport would be asked to parse.
    pub const DENIED_FR: &str = "user@example.com: Permission refusée (publickey,password).";
}

// ---------------------------------------------------------------------------

use apex_shell::adapters::outbound::openssh::SshTransport;
use std::process::{Child, Command, Stdio};

/// Spawns the mock daemon instead of `ssh`.
///
/// The transport under test is the real one: a real child process, a real pipe, a real
/// framing codec. Only what is on the other end differs, which is the point of the
/// `ProcessSpawner` port.
pub struct MockSpawner {
    pub script: String,
    children: Arc<Mutex<Vec<Child>>>,
}

impl MockSpawner {
    pub fn new(script: &str) -> Self {
        Self {
            script: script.into(),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// How many mock processes are still running. SC-003 asserts this reaches zero.
    pub fn live_children(&self) -> usize {
        let mut guard = self.children.lock().expect("children lock");
        guard.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        guard.len()
    }
}

impl ProcessSpawner for MockSpawner {
    fn preflight(&self) -> Result<String, SpawnError> {
        Ok("OpenSSH_9.6p1 (mock)".into())
    }

    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        vec![format!("{}@{}", spec.user, spec.host)]
    }

    fn environment(&self, _spec: &SpawnSpec) -> Vec<(String, String)> {
        vec![("LC_ALL".to_string(), "C".to_string())]
    }

    fn spawn(&self, _spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apex-mock-daemon"))
            .env("APEX_MOCK_SCRIPT", &self.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SpawnError::Io(e.to_string()))?;

        let stdin = Box::new(child.stdin.take().expect("piped stdin"));
        let stdout = Box::new(child.stdout.take().expect("piped stdout"));
        let stderr = Box::new(child.stderr.take().expect("piped stderr"));

        // The handle stays with the spawner so `live_children` can answer SC-003, so the
        // wait is a poll of that shared list rather than ownership of the `Child`.
        let children = self.children.clone();
        let out = SpawnedChild {
            stdin,
            stdout,
            stderr,
            wait: Box::new(move || loop {
                let mut guard = children.lock().expect("children lock");
                let mut ended = None;
                for c in guard.iter_mut() {
                    if let Ok(Some(status)) = c.try_wait() {
                        ended = Some(status.code());
                    }
                }
                drop(guard);
                if let Some(code) = ended {
                    return code;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }),
        };
        self.children.lock().expect("children lock").push(child);
        Ok(out)
    }

    fn supply_passphrase(&self, _secret: Secret) -> Result<(), SpawnError> {
        Ok(())
    }
}

pub fn spec() -> SpawnSpec {
    SpawnSpec {
        host: "mock.invalid".into(),
        user: "dev".into(),
        assisted: false,
    }
}

/// A connected transport speaking to a mock driven by `script`.
pub fn connected(script: &str) -> (Arc<SshTransport>, Arc<MockSpawner>) {
    let spawner = Arc::new(MockSpawner::new(script));
    let t = Arc::new(SshTransport::new(spawner.clone(), spec()));
    t.connect().expect("the mock should connect");
    (t, spawner)
}

// ---------------------------------------------------------------------------

/// Fails the silent phase, and connects for real on the assisted one.
///
/// The shape User Story 2 is about: a key the agent does not hold, whose passphrase the
/// user supplies. Phase one gets a scripted `ssh` failure; phase two gets the mock daemon,
/// so the connection that results is a real child on a real pipe.
///
/// The alternative — a fake whose stdout simply never yields — was tried and rejected: the
/// transport joins its reader thread on teardown, so a reader blocked in a fake that never
/// ends deadlocks the test suite rather than failing it.
pub struct AssistedSpawner {
    pub silent_exit: i32,
    pub silent_stderr: String,
    /// Set when phase two should fail too — the identity-picker case (FR-009).
    pub assisted_failure: Option<(i32, String)>,
    mock: MockSpawner,
    invocations: Recorded<Vec<String>>,
    environments: Recorded<Vec<(String, String)>>,
}

impl AssistedSpawner {
    pub fn new(silent_exit: i32, silent_stderr: &str) -> Self {
        Self {
            silent_exit,
            silent_stderr: silent_stderr.into(),
            assisted_failure: None,
            mock: MockSpawner::new(""),
            invocations: Arc::new(Mutex::new(Vec::new())),
            environments: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Both automatic routes fail: the agent has no key and the passphrase is wrong.
    pub fn also_failing_assisted(mut self, exit: i32, stderr: &str) -> Self {
        self.assisted_failure = Some((exit, stderr.into()));
        self
    }

    pub fn invocations(&self) -> Vec<Vec<String>> {
        self.invocations.lock().expect("invocation lock").clone()
    }

    pub fn environments(&self) -> Vec<Vec<(String, String)>> {
        self.environments.lock().expect("environment lock").clone()
    }

    pub fn live_children(&self) -> usize {
        self.mock.live_children()
    }
}

fn scripted_child(code: i32, stderr: String) -> SpawnedChild {
    SpawnedChild {
        stdin: Box::new(std::io::sink()),
        stdout: Box::new(std::io::empty()),
        stderr: Box::new(std::io::Cursor::new(stderr.into_bytes())),
        wait: Box::new(move || Some(code)),
    }
}

impl ProcessSpawner for AssistedSpawner {
    fn preflight(&self) -> Result<String, SpawnError> {
        Ok("OpenSSH_9.6p1 (mock)".into())
    }

    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        let mut v = vec!["ssh".to_string()];
        if !spec.assisted {
            // The flag that makes phase one incapable of prompting anywhere, tty included.
            v.push("-o".into());
            v.push("BatchMode=yes".into());
        }
        v.push(format!("{}@{}", spec.user, spec.host));
        v
    }

    fn environment(&self, spec: &SpawnSpec) -> Vec<(String, String)> {
        let mut env = vec![("LC_ALL".to_string(), "C".to_string())];
        if spec.assisted {
            env.push((
                "SSH_ASKPASS".into(),
                "/opt/apex/libexec/apex-askpass".into(),
            ));
            env.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
        }
        env
    }

    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        self.invocations
            .lock()
            .expect("invocation lock")
            .push(self.invocation(spec));
        self.environments
            .lock()
            .expect("environment lock")
            .push(self.environment(spec));

        if !spec.assisted {
            return Ok(scripted_child(self.silent_exit, self.silent_stderr.clone()));
        }
        match &self.assisted_failure {
            Some((code, stderr)) => Ok(scripted_child(*code, stderr.clone())),
            None => self.mock.spawn(spec),
        }
    }

    fn supply_passphrase(&self, _secret: Secret) -> Result<(), SpawnError> {
        Ok(())
    }
}

/// One log file for the whole test binary, so a test can assert what was never written to
/// it. `logging::init` takes effect once per process, which is why this is shared.
pub fn log_file() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static LOG: OnceLock<std::path::PathBuf> = OnceLock::new();
    LOG.get_or_init(|| {
        let p = std::env::temp_dir().join(format!("apex-test-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        apex_shell::logging::init(p.clone());
        p
    })
    .clone()
}
