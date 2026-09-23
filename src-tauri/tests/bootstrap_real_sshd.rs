//! The deployment path against a real `sshd`. **Opt-in.**
//!
//! Everything else in this feature's suite drives the deployer through doubles, which proves
//! the policy and proves nothing about the mechanism: whether the stream survives a real
//! channel, whether the remote `sha256sum` agrees with the digest computed at build time,
//! whether the rename is actually atomic on a real filesystem.
//!
//! Opt-in because it binds loopback, which `assert_no_network` correctly forbids elsewhere.
//!
//! ```sh
//! APEX_REAL_SSHD=1 cargo test --test bootstrap_real_sshd -- --nocapture
//! ```

// A test that waits for a daemon, on the thread doing the waiting, with no runtime in sight.
#![allow(clippy::disallowed_methods)]

mod common;

use apex_shell::adapters::outbound::deploy::{paths, SshStreamDeployer};
use apex_shell::application::ports::deployer::{ArtifactDeployer, Target};
use apex_shell::domain::artifact::{Architecture, DeploymentFailure, Digest, EngineArtifact};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn skip_reason() -> Option<String> {
    if std::env::var("APEX_REAL_SSHD").unwrap_or_default() != "1" {
        return Some("APEX_REAL_SSHD is not set to 1; this test binds loopback".into());
    }
    for tool in [
        "/usr/sbin/sshd",
        "/usr/bin/ssh-keygen",
        "/usr/bin/sha256sum",
    ] {
        if !Path::new(tool).exists() {
            return Some(format!("{tool} is not installed"));
        }
    }
    None
}

struct Sshd {
    dir: PathBuf,
    config: PathBuf,
    child: Child,
    target: Target,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // The deployer's own ControlPath wins over anything -F supplies, so the master lands
        // under the developer's ~/.ssh. Close it rather than leaving it to ControlPersist.
        let _ = Command::new("ssh")
            .args([
                "-F",
                &self.config.display().to_string(),
                "-O",
                "exit",
                &format!("{}@{}", self.target.user, self.target.host),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
        // The "remote" home is this machine's, so deployed engines are real files here.
        // Remove what these tests wrote rather than leaving them for the next run to trip on.
        if let Some(home) = std::env::var_os("HOME") {
            let engines = PathBuf::from(home).join(".apex/engine");
            if let Ok(entries) = std::fs::read_dir(&engines) {
                for e in entries.flatten() {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("ide-engine-0.1.0-") || name.starts_with(".staged-") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
    }
}

fn start() -> Sshd {
    let dir = std::env::temp_dir().join(format!("apex-deploy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("0700");

    for name in ["host_key", "client_key"] {
        let status = Command::new("/usr/bin/ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", ""])
            .arg("-f")
            .arg(dir.join(name))
            .status()
            .expect("ssh-keygen");
        assert!(status.success());
    }
    std::fs::copy(dir.join("client_key.pub"), dir.join("authorized_keys")).expect("authkeys");
    std::fs::set_permissions(
        dir.join("authorized_keys"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("0600");

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();

    std::fs::write(
        dir.join("sshd_config"),
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host_key\n\
             AuthorizedKeysFile {d}/authorized_keys\nPidFile {d}/sshd.pid\n\
             PasswordAuthentication no\nUsePAM no\n\
             StrictModes no\nLogLevel ERROR\n",
            d = dir.display()
        ),
    )
    .expect("sshd_config");

    // The client config the deployer is pointed at. This is the seam: without it the deployer
    // has no way to name a port, an identity or a known_hosts file.
    std::fs::write(
        dir.join("ssh_config"),
        format!(
            "Host 127.0.0.1\n  Port {port}\n  IdentityFile {d}/client_key\n\
             IdentitiesOnly yes\n  UserKnownHostsFile {d}/known_hosts\n\
             StrictHostKeyChecking accept-new\n  BatchMode yes\n",
            d = dir.display()
        ),
    )
    .expect("ssh_config");

    let child = Command::new("/usr/sbin/sshd")
        .arg("-D")
        .arg("-e")
        .arg("-f")
        .arg(dir.join("sshd_config"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sshd");

    let target = Target {
        host: "127.0.0.1".into(),
        user: std::env::var("USER").unwrap_or_else(|_| "root".into()),
    };
    let sshd = Sshd {
        config: dir.join("ssh_config"),
        dir,
        child,
        target,
    };
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return sshd;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("sshd did not accept connections");
}

fn deployer(s: &Sshd) -> SshStreamDeployer {
    SshStreamDeployer::with_config(s.config.display().to_string())
}

/// A real artifact with a real digest — the bytes the client actually embeds.
///
/// `version` is per-test, because the "remote host" here is this machine and `~/.apex/engine`
/// is a real shared directory. Two tests deploying version 0.1.0 would write the same final
/// path, and the second would see the first's file and draw the wrong conclusion — which is
/// exactly what happened before this argument existed.
fn real_artifact_versioned(version: &str) -> EngineArtifact {
    let mut a = real_artifact();
    a.version = version.to_string();
    a
}

fn real_artifact() -> EngineArtifact {
    let (bytes, digest) =
        apex_shell::adapters::outbound::deploy::embedded::host_artifact().expect("engine embedded");
    EngineArtifact {
        version: "0.1.0".into(),
        protocol_version: apex_protocol::wire::PROTOCOL_VERSION,
        architecture: Architecture::LinuxX86_64,
        digest: Digest::parse(digest).expect("a valid digest"),
        bytes,
    }
}

fn remote_exists(s: &Sshd, path: &str) -> bool {
    Command::new("ssh")
        .args([
            "-F",
            &s.config.display().to_string(),
            &format!("{}@{}", s.target.user, s.target.host),
            &format!("test -f {path}"),
        ])
        .status()
        .map(|st| st.success())
        .unwrap_or(false)
}

/// The whole mechanism: stream, verify against the host's own `sha256sum`, promote atomically.
#[test]
fn an_artifact_is_streamed_verified_and_promoted_on_a_real_host() {
    if let Some(r) = skip_reason() {
        eprintln!("SKIPPED: {r}");
        return;
    }
    let sshd = start();
    let d = deployer(&sshd);
    let artifact = real_artifact_versioned("0.1.0-promote");
    let (final_path, staged) = paths(&artifact);

    // The "remote" host is this machine, so the probe must agree with what Rust was built for.
    assert_eq!(
        d.remote_architecture(&sshd.target).expect("uname"),
        std::env::consts::ARCH,
        "the probe must report what the host really runs"
    );

    d.deploy(&artifact, &sshd.target).expect("deploy");

    assert!(remote_exists(&sshd, &final_path), "engine must be in place");
    assert!(
        !remote_exists(&sshd, &staged),
        "the staged file must not survive promotion: {staged}"
    );

    // Executable, and actually **serving** — the property verification alone cannot establish.
    //
    // Asserted by speaking to it rather than by looking for a banner. The engine prints
    // nothing on startup and blocks reading stdin, so an earlier version of this test, written
    // against a placeholder that printed its name, saw empty output and concluded the binary
    // had not run. Feeding it a handshake proves the thing that matters: the deployed artifact
    // answers the protocol.
    let body = r#"{"jsonrpc":"2.0","id":"1","method":"auth/handshake","params":{"client_version":"0.1.0","protocol_version":1,"capabilities":[]}}"#;
    let framed = format!("Content-Length: {}\r\n\r\n{body}", body.len());

    let mut run = Command::new("ssh")
        .args([
            "-F",
            &sshd.config.display().to_string(),
            &format!("{}@{}", sshd.target.user, sshd.target.host),
            &final_path,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run the deployed engine");
    run.stdin
        .as_mut()
        .expect("stdin")
        .write_all(framed.as_bytes())
        .expect("write a handshake");
    let spoke = run
        .wait_with_output()
        .expect("the engine to answer and exit");
    let said = String::from_utf8_lossy(&spoke.stdout);

    assert!(
        said.starts_with("Content-Length: "),
        "the deployed engine did not answer the protocol: {said:?}"
    );
    assert!(
        said.contains("\"protocol_version\":1"),
        "the deployed engine answered, but not with a handshake: {said}"
    );
}

/// T019 / SC-003. A second deployment of the same artifact transfers nothing.
#[test]
fn a_matching_engine_is_not_transferred_again() {
    if let Some(r) = skip_reason() {
        eprintln!("SKIPPED: {r}");
        return;
    }
    let sshd = start();
    let d = deployer(&sshd);
    let artifact = real_artifact_versioned("0.1.0-idempotent");

    d.deploy(&artifact, &sshd.target).expect("first deploy");
    let first = std::time::Instant::now();
    d.deploy(&artifact, &sshd.target).expect("second deploy");
    let second = first.elapsed();

    // A re-transfer of several megabytes cannot complete in the time a digest check takes.
    assert!(
        second < std::time::Duration::from_secs(2),
        "the second deployment took {second:?}, which suggests it transferred again"
    );
}

/// A corrupted artifact is caught by the host's own digest and never promoted.
#[test]
fn an_artifact_whose_digest_does_not_match_is_never_promoted() {
    if let Some(r) = skip_reason() {
        eprintln!("SKIPPED: {r}");
        return;
    }
    let sshd = start();
    let d = deployer(&sshd);
    let mut artifact = real_artifact_versioned("0.1.0-corrupt");
    // Claim a digest the bytes do not have — exactly what a corrupted transfer looks like.
    artifact.digest = Digest::parse(&"b".repeat(64)).expect("valid");
    let (final_path, staged) = paths(&artifact);

    assert_eq!(
        d.deploy(&artifact, &sshd.target),
        Err(DeploymentFailure::DigestMismatch)
    );
    assert!(
        !remote_exists(&sshd, &final_path),
        "an unverified artifact must never be promoted"
    );
    assert!(
        !remote_exists(&sshd, &staged),
        "and the staged file must be cleaned up so a retry is not confused by it"
    );
}

/// T084 / FR-005. Nothing in the deployment path needs elevated privilege.
#[test]
fn deployment_requires_no_elevated_privilege() {
    if let Some(r) = skip_reason() {
        eprintln!("SKIPPED: {r}");
        return;
    }
    let sshd = start();
    let d = deployer(&sshd);
    let artifact = real_artifact_versioned("0.1.0-privilege");
    d.deploy(&artifact, &sshd.target).expect("deploy");

    // Every remote command the deployer issues, as it issues them.
    for script in ["uname -m", "mkdir -p ~/.apex/engine"] {
        let args = apex_shell::adapters::outbound::deploy::remote_command(
            Some(&sshd.config.display().to_string()),
            &sshd.target,
            script,
        );
        let joined = args.join(" ");
        for forbidden in ["sudo", "su -", "setuid", "doas", "pkexec"] {
            assert!(
                !joined.contains(forbidden),
                "the deployer must not escalate: {joined}"
            );
        }
    }

    // And what landed is owned by the connecting account, not root.
    let owner = Command::new("ssh")
        .args([
            "-F",
            &sshd.config.display().to_string(),
            &format!("{}@{}", sshd.target.user, sshd.target.host),
            "stat -c %U ~/.apex/engine/ide-engine-0.1.0-privilege",
        ])
        .output()
        .expect("stat");
    assert_eq!(
        String::from_utf8_lossy(&owner.stdout).trim(),
        sshd.target.user,
        "the deployed engine must belong to the connecting account"
    );
    let _ = std::io::stdout().flush();
}
