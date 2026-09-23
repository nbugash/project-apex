//! The §3.1 invocation, in one place.
//!
//! Those flags are normative and each carries a reason the system specification records.
//! They appear here and nowhere else, so changing the invocation is one edit in one file
//! rather than a hunt.

use crate::application::ports::spawner::{ProcessSpawner, SpawnError, SpawnSpec, SpawnedChild};
use crate::domain::request::Secret;
use std::process::{Command, Stdio};

/// §3.1: `ControlPath=%C` needs 6.7. Below it the hashed form is unavailable and the
/// expanded form overflows macOS's 104-byte socket path limit on long hostnames.
const MIN_VERSION: (u32, u32) = (6, 7);
/// §3.3: `SSH_ASKPASS_REQUIRE=force` needs 8.4. Below it, prompting inside the application
/// is unavailable and the connect sequence degrades to the identity picker.
pub const ASKPASS_MIN_VERSION: (u32, u32) = (8, 4);

pub struct OpenSshSpawner {
    /// The engine's path on the remote host. `[OPEN: H-BOOT]` owns how it gets there; this
    /// feature only names it and classifies exit 127 when it is absent.
    pub remote_engine: String,
    /// Absolute path to the bundled askpass helper. §3.3 requires absolute: OpenSSH execs it
    /// with an unpredictable working directory, so a relative path fails exactly when a user
    /// needs the prompt.
    pub askpass_path: Option<String>,
    /// Where the helper reaches back to.
    pub askpass_socket: Option<String>,
}

impl Default for OpenSshSpawner {
    fn default() -> Self {
        Self {
            remote_engine: "/usr/local/bin/ide-engine --mode=pipe".into(),
            askpass_path: None,
            askpass_socket: None,
        }
    }
}

impl OpenSshSpawner {
    pub fn new(remote_engine: impl Into<String>) -> Self {
        Self {
            remote_engine: remote_engine.into(),
            ..Default::default()
        }
    }
}

/// The options that decide **which connection** an invocation uses, and how it fails.
///
/// Shared with the deployer rather than duplicated. A deployer that named a different
/// `ControlPath` would open a master of its own and authenticate a second time — silently
/// defeating the reuse A-B1 chose this design for, and in a way nothing would notice, because
/// everything would still work and merely cost twice as much.
///
/// Every flag here is §3.1's; none is this module's idea.
pub fn control_options() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut opt = |k: &str| {
        v.push("-o".into());
        v.push(k.into());
    };
    // A master connection later invocations attach to without re-authenticating; what makes
    // preview forwarding and bulk transfer cheap.
    opt("ControlMaster=auto");
    // The hashed form: the expanded one overflows macOS's socket path limit.
    opt("ControlPath=~/.ssh/apex-%C");
    opt("ControlPersist=1h");
    // Tuned for frequent small interactive packets.
    opt("IPQoS=throughput");
    // Surface a dropped network within ~45 s. Without these the pipe hangs indefinitely and
    // the reconnection loop has nothing to react to — the single most important pair of flags
    // in this list, and the one no integration test can check, because the mock has no socket.
    opt("ServerAliveInterval=15");
    opt("ServerAliveCountMax=3");
    opt("StrictHostKeyChecking=accept-new");
    v
}

/// Parse `OpenSSH_9.6p1, OpenSSL ...` into a comparable pair.
pub fn parse_version(banner: &str) -> Option<(u32, u32)> {
    let at = banner.find("OpenSSH_")? + "OpenSSH_".len();
    let rest = &banner[at..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    let mut parts = rest[..end].split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor))
}

impl ProcessSpawner for OpenSshSpawner {
    /// FR-005. Checked at startup rather than discovered at the first connection failure,
    /// because "ssh is too old" and "the host refused you" need very different responses and
    /// a user who sees the second when the first is true will debug the wrong thing.
    fn preflight(&self) -> Result<String, SpawnError> {
        let out = Command::new("ssh")
            .arg("-V")
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => SpawnError::NotFound,
                _ => SpawnError::Io(e.to_string()),
            })?;

        // `ssh -V` writes its banner to stderr.
        let banner = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let Some(found) = parse_version(&banner) else {
            return Err(SpawnError::Io(format!(
                "could not read a version from {banner:?}"
            )));
        };
        if found < MIN_VERSION {
            return Err(SpawnError::TooOld {
                found: banner,
                required: format!("OpenSSH_{}.{}", MIN_VERSION.0, MIN_VERSION.1),
            });
        }
        Ok(banner)
    }

    /// The normative invocation. Every flag here is §3.1's; none is this module's idea.
    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        let mut v = control_options();
        if !spec.assisted {
            // Phase one. Prevents ssh blocking on a tty prompt no GUI user can answer, and
            // disables SSH_ASKPASS — which is why connecting is two phases (§3.3).
            v.push("-o".into());
            v.push("BatchMode=yes".into());
        }
        v.push(format!("{}@{}", spec.user, spec.host));
        v.push(self.remote_engine.clone());
        v
    }

    /// The environment §3.1 and §3.3 require. One source of truth with `spawn`, which
    /// applies exactly this list — a second copy would drift, and the drift would be
    /// invisible until a user needed a prompt.
    fn environment(&self, spec: &SpawnSpec) -> Vec<(String, String)> {
        // §3.4: stable stderr across locales, which is what makes classification
        // locale-independent rather than merely hopeful.
        let mut env = vec![("LC_ALL".to_string(), "C".to_string())];
        if spec.assisted {
            if let (Some(path), Some(socket)) = (&self.askpass_path, &self.askpass_socket) {
                env.push(("SSH_ASKPASS".into(), path.clone()));
                // Without force, OpenSSH consults askpass only when it finds no tty, and
                // that varies by platform and by DISPLAY.
                env.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
                env.push(("APEX_ASKPASS_SOCKET".into(), socket.clone()));
            }
        }
        env
    }

    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        if spec.assisted && (self.askpass_path.is_none() || self.askpass_socket.is_none()) {
            // Refuse rather than spawn a process that will block on a prompt nobody can
            // answer: without the helper, an assisted attempt is an assisted attempt in
            // name only.
            return Err(SpawnError::Io(
                "an assisted attempt needs an askpass helper and socket".into(),
            ));
        }
        if let Some(path) = &self.askpass_path {
            if spec.assisted && !std::path::Path::new(path).is_absolute() {
                // §3.3. OpenSSH execs the helper with an unpredictable working directory,
                // so a relative path fails exactly when a user needs the prompt.
                return Err(SpawnError::Io(format!(
                    "SSH_ASKPASS must be an absolute path, got {path:?}"
                )));
            }
        }

        let mut cmd = Command::new("ssh");
        cmd.args(self.invocation(spec))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in self.environment(spec) {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SpawnError::NotFound,
            _ => SpawnError::Io(e.to_string()),
        })?;

        let stdin = Box::new(child.stdin.take().expect("piped stdin"));
        let stdout = Box::new(child.stdout.take().expect("piped stdout"));
        let stderr = Box::new(child.stderr.take().expect("piped stderr"));

        Ok(SpawnedChild {
            stdin,
            stdout,
            stderr,
            // Owning the handle here is what makes the child reapable. A `Child` dropped
            // without a wait leaves a zombie until the process itself exits.
            wait: Box::new(move || child.wait().ok().and_then(|s| s.code())),
        })
    }

    fn supply_passphrase(&self, _secret: Secret) -> Result<(), SpawnError> {
        // The helper reaches the application over the askpass socket; nothing is pushed
        // through the spawner. The `Secret` is dropped here, which zeroes it.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(assisted: bool) -> SpawnSpec {
        SpawnSpec {
            host: "build-01.euw1".into(),
            user: "dev".into(),
            assisted,
        }
    }

    /// T026 — FR-004, SC-009.
    ///
    /// These two flags are what bound the time to detecting a silently dropped network:
    /// without them `ssh` never gives up, the pipe never ends, and the transport waits
    /// forever. No integration test can catch their absence, because the mock has no socket
    /// and no keepalive — which is exactly why this assertion exists here.
    #[test]
    fn the_invocation_carries_the_keepalive_flags() {
        let inv = OpenSshSpawner::default().invocation(&spec(false));
        assert!(
            inv.iter().any(|a| a == "ServerAliveInterval=15"),
            "without ServerAliveInterval a pulled cable hangs forever: {inv:?}"
        );
        assert!(
            inv.iter().any(|a| a == "ServerAliveCountMax=3"),
            "without ServerAliveCountMax the interval never concludes anything: {inv:?}"
        );
    }

    #[test]
    fn the_invocation_carries_the_connection_reuse_flags() {
        let inv = OpenSshSpawner::default().invocation(&spec(false));
        for flag in [
            "ControlMaster=auto",
            "ControlPath=~/.ssh/apex-%C",
            "ControlPersist=1h",
        ] {
            assert!(inv.iter().any(|a| a == flag), "missing {flag}: {inv:?}");
        }
    }

    /// §3.3 makes these mutually exclusive: BatchMode disables all interactive querying,
    /// askpass included. An assisted attempt that still carried BatchMode would silently
    /// never prompt.
    #[test]
    fn batch_mode_is_present_when_silent_and_absent_when_assisted() {
        assert!(OpenSshSpawner::default()
            .invocation(&spec(false))
            .iter()
            .any(|a| a == "BatchMode=yes"));
        assert!(!OpenSshSpawner::default()
            .invocation(&spec(true))
            .iter()
            .any(|a| a == "BatchMode=yes"));
    }

    #[test]
    fn the_invocation_ends_with_the_target_and_the_engine() {
        let inv = OpenSshSpawner::new("/opt/engine --mode=pipe").invocation(&spec(false));
        assert_eq!(inv[inv.len() - 2], "dev@build-01.euw1");
        assert_eq!(inv[inv.len() - 1], "/opt/engine --mode=pipe");
    }

    #[test]
    fn versions_are_parsed_and_compared() {
        assert_eq!(parse_version("OpenSSH_9.6p1, OpenSSL 3.0"), Some((9, 6)));
        assert_eq!(parse_version("OpenSSH_6.7p1"), Some((6, 7)));
        assert_eq!(parse_version("OpenSSH_8.2p1 Ubuntu"), Some((8, 2)));
        assert_eq!(parse_version("not a banner"), None);

        assert!((6, 6) < MIN_VERSION, "6.6 lacks ControlPath=%C");
        assert!((6, 7) >= MIN_VERSION);
        assert!(
            (8, 2) < ASKPASS_MIN_VERSION,
            "Ubuntu 20.04 cannot prompt in-app"
        );
        assert!((8, 4) >= ASKPASS_MIN_VERSION);
    }

    /// An assisted attempt with nowhere to send the prompt must refuse rather than spawn a
    /// process that will block on a prompt nobody can answer.
    /// §3.3 makes the absolute path normative. A relative one fails only in the moment a
    /// user is waiting for a prompt, which is the worst possible time to discover it.
    #[test]
    fn a_relative_askpass_path_is_refused_before_anything_is_spawned() {
        let s = OpenSshSpawner {
            askpass_path: Some("apex-askpass".into()),
            askpass_socket: Some("/tmp/apex.sock".into()),
            ..Default::default()
        };
        match s.spawn(&spec(true)) {
            Err(SpawnError::Io(m)) => assert!(m.contains("absolute"), "{m}"),
            Err(other) => panic!("expected an Io refusal, got {other:?}"),
            Ok(_) => panic!("a relative SSH_ASKPASS must be refused"),
        }
    }

    /// The two settings §3.3 makes normative, asserted where their absence is visible.
    /// An integration test cannot catch them: without them OpenSSH simply never prompts,
    /// which is indistinguishable from an ordinary authentication failure.
    #[test]
    fn an_assisted_attempt_forces_the_helper_and_names_it_absolutely() {
        let s = OpenSshSpawner {
            askpass_path: Some("/opt/apex/bin/apex-askpass".into()),
            askpass_socket: Some("/run/apex/askpass.sock".into()),
            ..Default::default()
        };
        let env = s.environment(&spec(true));
        let get = |k: &str| env.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());

        assert_eq!(get("SSH_ASKPASS_REQUIRE").as_deref(), Some("force"));
        let path = get("SSH_ASKPASS").expect("assisted attempts must name a helper");
        assert!(std::path::Path::new(&path).is_absolute(), "{path}");

        // And the silent phase must set neither, or phase one would prompt.
        let silent = s.environment(&spec(false));
        assert!(!silent.iter().any(|(k, _)| k.starts_with("SSH_ASKPASS")));
    }

    #[test]
    fn every_invocation_pins_the_c_locale() {
        for assisted in [false, true] {
            let env = OpenSshSpawner::default().environment(&spec(assisted));
            assert!(
                env.contains(&("LC_ALL".to_string(), "C".to_string())),
                "without LC_ALL=C, classification reads a language we did not ask for"
            );
        }
    }

    #[test]
    fn an_assisted_spawn_without_a_helper_is_refused() {
        match OpenSshSpawner::default().spawn(&spec(true)) {
            Err(SpawnError::Io(_)) => {}
            Err(other) => panic!("expected an Io refusal, got {other:?}"),
            Ok(_) => panic!("an assisted spawn with no helper must be refused"),
        }
    }
}
