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

// ---------------------------------------------------------------------------
// F002: deployment doubles and the suite-level network assertion.

use apex_shell::application::ports::deployer::{ArtifactDeployer, Target};
use apex_shell::domain::artifact::{DeploymentFailure, DeploymentState, EngineArtifact};

/// Produces chosen deployment outcomes without moving a byte.
///
/// The seam that makes every failure path reachable. Provoking a full disk, an unwritable
/// directory and an unsupported architecture from a real host on demand would need three hosts,
/// or one repeatedly broken on purpose, over a network the suite is required not to need.
pub struct ScriptedDeployer {
    pub architecture: Result<String, DeploymentFailure>,
    pub outcome: Result<(), DeploymentFailure>,
    /// Makes retirement fail, which must not fail the update that preceded it.
    pub retire_fails: Option<DeploymentFailure>,
    /// Published as the deployment runs, so a test can assert on progress cadence.
    pub progress: Vec<(u64, u64)>,
    pub calls: Recorded<String>,
    state: tokio::sync::watch::Sender<DeploymentState>,
    keep: tokio::sync::watch::Receiver<DeploymentState>,
}

impl Default for ScriptedDeployer {
    fn default() -> Self {
        let (state, keep) = tokio::sync::watch::channel(DeploymentState::Preparing);
        Self {
            architecture: Ok("x86_64".into()),
            outcome: Ok(()),
            retire_fails: None,
            progress: Vec::new(),
            calls: Arc::new(Mutex::new(Vec::new())),
            state,
            keep,
        }
    }
}

impl ScriptedDeployer {
    pub fn failing(failure: DeploymentFailure) -> Self {
        Self {
            outcome: Err(failure),
            ..Default::default()
        }
    }

    pub fn on_architecture(machine: &str) -> Self {
        Self {
            architecture: Ok(machine.into()),
            ..Default::default()
        }
    }

    /// Every call made, in order — so a test can assert that `retire_previous` was not reached.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl ArtifactDeployer for ScriptedDeployer {
    fn remote_architecture(&self, _t: &Target) -> Result<String, DeploymentFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("remote_architecture".into());
        self.architecture.clone()
    }

    fn deploy(&self, a: &EngineArtifact, _t: &Target) -> Result<(), DeploymentFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("deploy:{}", a.version));
        for (sent, total) in &self.progress {
            let _ = self.state.send(DeploymentState::Transferring {
                sent: *sent,
                total: *total,
            });
        }
        match &self.outcome {
            Ok(()) => {
                let _ = self.state.send(DeploymentState::Complete);
                Ok(())
            }
            Err(f) => {
                let _ = self.state.send(DeploymentState::Failed(f.clone()));
                Err(f.clone())
            }
        }
    }

    fn retire_previous(&self, version: &str, _t: &Target) -> Result<(), DeploymentFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("retire:{version}"));
        match &self.retire_fails {
            Some(f) => Err(f.clone()),
            None => Ok(()),
        }
    }

    fn observe(&self) -> tokio::sync::watch::Receiver<DeploymentState> {
        self.keep.clone()
    }
}

pub fn target() -> Target {
    Target {
        host: "mock.invalid".into(),
        user: "dev".into(),
    }
}

/// Fail the calling test if this process holds any TCP socket.
///
/// One helper called from every integration binary, because each test file compiles to its own
/// process: F001's equivalent lives in a single binary and proves nothing about the others, so
/// SC-010 read as satisfied while three of them were entirely unchecked.
///
/// It proves itself before trusting its own silence. A detector that cannot see a socket reports
/// "none open" exactly as convincingly as one that finds none, and this suite has already
/// produced a check that passed for that reason.
pub fn assert_no_network() {
    #[cfg(target_os = "linux")]
    {
        {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback socket");
            assert!(
                !open_tcp_sockets().is_empty(),
                "the detector cannot see a socket that is demonstrably open, so its silence \
                 means nothing"
            );
            drop(listener);
        }
        let open = open_tcp_sockets();
        assert!(
            open.is_empty(),
            "this suite must need no network; found TCP socket inodes {open:?}"
        );
    }
}

/// Inodes of TCP sockets held by this process.
#[cfg(target_os = "linux")]
fn open_tcp_sockets() -> Vec<u64> {
    let mut tcp = std::collections::HashSet::new();
    for table in ["/proc/self/net/tcp", "/proc/self/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(table) else {
            continue;
        };
        for line in text.lines().skip(1) {
            if let Some(inode) = line.split_whitespace().nth(9) {
                if let Ok(n) = inode.parse::<u64>() {
                    tcp.insert(n);
                }
            }
        }
    }
    let mut ours = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return ours;
    };
    for entry in entries.flatten() {
        let Ok(link) = std::fs::read_link(entry.path()) else {
            continue;
        };
        // Unix sockets land here too, which is why the inode is checked against the TCP
        // tables rather than assumed.
        if let Some(rest) = link.to_string_lossy().strip_prefix("socket:[") {
            if let Ok(inode) = rest.trim_end_matches(']').parse::<u64>() {
                if tcp.contains(&inode) {
                    ours.push(inode);
                }
            }
        }
    }
    ours
}

use apex_protocol::wire::{
    CapabilitySet, HandshakeRequest, HandshakeResponse, SessionId, PROTOCOL_VERSION,
};
use apex_shell::application::ports::handshake::{HandshakeError, HandshakePeer};
use std::sync::atomic::{AtomicUsize, Ordering};

/// An engine that answers the handshake however a test needs.
pub struct ScriptedPeer {
    /// Versions answered in order, then the last repeats. A replacement changes what the
    /// engine reports, so a double that answers identically forever cannot express the
    /// sequence US4 is about.
    pub versions: Mutex<Vec<u32>>,
    pub protocol_version: u32,
    pub capabilities: CapabilitySet,
    pub error: Option<HandshakeError>,
    pub resumed: bool,
    asked: AtomicUsize,
    pub requests: Recorded<HandshakeRequest>,
}

impl Default for ScriptedPeer {
    fn default() -> Self {
        Self {
            versions: Mutex::new(Vec::new()),
            protocol_version: PROTOCOL_VERSION,
            capabilities: CapabilitySet::of(&["session/shutdown"]),
            error: None,
            resumed: false,
            asked: AtomicUsize::new(0),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ScriptedPeer {
    pub fn speaking(protocol_version: u32) -> Self {
        Self {
            protocol_version,
            ..Default::default()
        }
    }

    /// Answers these versions in order — an older engine, then the replacement.
    pub fn answering(versions: &[u32]) -> Self {
        Self {
            versions: Mutex::new(versions.iter().rev().copied().collect()),
            ..Default::default()
        }
    }

    pub fn failing(error: HandshakeError) -> Self {
        Self {
            error: Some(error),
            ..Default::default()
        }
    }

    /// How many handshakes were attempted — the measurement that says whether anything was
    /// exchanged with an engine the client should have refused.
    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

impl HandshakePeer for ScriptedPeer {
    async fn handshake(
        &self,
        request: HandshakeRequest,
    ) -> Result<HandshakeResponse, HandshakeError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().expect("requests lock").push(request);
        let version = {
            let mut q = self.versions.lock().expect("versions lock");
            if q.len() > 1 {
                q.pop().unwrap_or(self.protocol_version)
            } else {
                q.last().copied().unwrap_or(self.protocol_version)
            }
        };
        if let Some(e) = &self.error {
            return Err(match e {
                HandshakeError::TimedOut => HandshakeError::TimedOut,
                HandshakeError::ConnectionLost => HandshakeError::ConnectionLost,
                HandshakeError::Malformed(m) => HandshakeError::Malformed(m.clone()),
            });
        }
        Ok(HandshakeResponse {
            engine_version: format!("0.{version}.0"),
            protocol_version: version,
            capabilities: self.capabilities.clone(),
            session_id: SessionId("session-1".into()),
            resumed: self.resumed,
        })
    }
}

/// An artifact that moves no bytes but has a real digest, for policy tests.
pub fn artifact(arch: apex_shell::domain::artifact::Architecture) -> EngineArtifact {
    use apex_shell::domain::artifact::Digest;
    EngineArtifact {
        version: "0.1.0".into(),
        protocol_version: PROTOCOL_VERSION,
        architecture: arch,
        digest: Digest::parse(&"a".repeat(64)).expect("valid digest"),
        bytes: b"",
    }
}

// ---------------------------------------------------------------------------

/// Spawns the **real** engine instead of the mock.
///
/// The mock cannot serve this feature's tests: a test asserts no §4.8 method name appears in
/// its directory, which is what keeps it a framing double rather than a second engine that
/// would drift. So anything about the handshake has to talk to the real binary.
pub struct EngineSpawner {
    /// Handed to the child, so a test can drive re-execution by pre-seeding an identity.
    pub session_env: Option<String>,
    children: Arc<Mutex<Vec<Child>>>,
}

impl Default for EngineSpawner {
    fn default() -> Self {
        Self {
            session_env: None,
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl EngineSpawner {
    /// Where the engine binary is.
    ///
    /// `CARGO_BIN_EXE_*` is only set for binaries of the *same* package, and the engine is a
    /// different crate — so the path is derived the way `build.rs` derives it, and says what to
    /// run when it is missing rather than failing to spawn something unnamed.
    pub fn binary() -> std::path::PathBuf {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join(if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            })
            .join("ide-engine");
        assert!(
            p.exists(),
            "the engine is not built. Run `cargo build -p apex-engine` first: {}",
            p.display()
        );

        // And that it is not *stale*. `cargo test` does not rebuild another package's binary,
        // so an old engine sits there answering an old protocol — which surfaced as every
        // handshake test failing with ConnectionLost, a symptom that says nothing about the
        // cause. A missing binary is obvious; a stale one is the expensive kind of wrong.
        let built = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        let sources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("engine");
        let newest = newest_source(&sources);
        if let (Some(built), Some(newest)) = (built, newest) {
            assert!(
                built >= newest,
                "the engine binary is older than its sources. Run `cargo build -p apex-engine`: {}",
                p.display()
            );
        }
        p
    }

    pub fn live_children(&self) -> usize {
        let mut g = self.children.lock().expect("children lock");
        g.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        g.len()
    }
}

/// The most recently modified Rust source or manifest under `dir`.
fn newest_source(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs" || x == "toml") {
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    newest = Some(newest.map_or(t, |n: std::time::SystemTime| n.max(t)));
                }
            }
        }
    }
    newest
}

impl ProcessSpawner for EngineSpawner {
    fn preflight(&self) -> Result<String, SpawnError> {
        Ok("OpenSSH_9.6p1 (engine harness)".into())
    }

    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        vec![format!("{}@{}", spec.user, spec.host)]
    }

    fn environment(&self, _spec: &SpawnSpec) -> Vec<(String, String)> {
        vec![("LC_ALL".to_string(), "C".to_string())]
    }

    fn spawn(&self, _spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        let mut cmd = Command::new(Self::binary());
        if let Some(id) = &self.session_env {
            cmd.env("APEX_SESSION_ID", id);
        } else {
            cmd.env_remove("APEX_SESSION_ID");
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SpawnError::Io(e.to_string()))?;

        let stdin = Box::new(child.stdin.take().expect("piped stdin"));
        let stdout = Box::new(child.stdout.take().expect("piped stdout"));
        let stderr = Box::new(child.stderr.take().expect("piped stderr"));
        let children = self.children.clone();
        let out = SpawnedChild {
            stdin,
            stdout,
            stderr,
            wait: Box::new(move || loop {
                let mut g = children.lock().expect("children lock");
                let mut ended = None;
                for c in g.iter_mut() {
                    if let Ok(Some(s)) = c.try_wait() {
                        ended = Some(s.code());
                    }
                }
                drop(g);
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

/// A transport connected to a real engine.
pub fn connected_engine(session: Option<&str>) -> (Arc<SshTransport>, Arc<EngineSpawner>) {
    let spawner = Arc::new(EngineSpawner {
        session_env: session.map(|s| s.to_string()),
        ..Default::default()
    });
    let t = Arc::new(SshTransport::new(spawner.clone(), spec()));
    t.connect().expect("the engine should start");
    (t, spawner)
}
