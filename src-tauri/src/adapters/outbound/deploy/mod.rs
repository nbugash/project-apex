//! Putting an engine on a remote host.
//!
//! Deployment runs **beside** the protocol channel, not through it: at deployment time there is
//! no engine to talk to, and §4.1 caps a frame at 1 MiB against an artifact measured in tens of
//! megabytes. Every step is an `ssh` invocation multiplexed over the control master F001
//! already holds, so the four commands here cost one authentication between them — measured,
//! not assumed.

pub mod embedded;
pub mod progress;

use crate::adapters::outbound::openssh::control_options;
use crate::application::ports::deployer::{ArtifactDeployer, Target};
use crate::domain::artifact::{DeploymentFailure, DeploymentState, EngineArtifact};
use progress::ProgressTicker;
use std::io::Write;
use std::process::{Command, Stdio};
use tokio::sync::watch;

/// Where deployed engines live. Under the developer's own home, because A-EC2 makes the
/// instance theirs and nothing here needs privilege.
const REMOTE_DIR: &str = "~/.apex/engine";

/// How much is written between progress checks. Large enough that the syscall cost is
/// irrelevant, small enough that a 1-second cadence is achievable on a slow link.
const CHUNK: usize = 256 * 1024;

pub struct SshStreamDeployer {
    state: watch::Sender<DeploymentState>,
    keep: watch::Receiver<DeploymentState>,
}

impl Default for SshStreamDeployer {
    fn default() -> Self {
        let (state, keep) = watch::channel(DeploymentState::Preparing);
        Self { state, keep }
    }
}

/// The `ssh` argument list for one remote command, reusing F001's master.
pub fn remote_command(target: &Target, script: &str) -> Vec<String> {
    let mut v = control_options();
    // The deployer must never prompt. There is no tty behind it and no askpass wired to it,
    // so a prompt would hang a transfer with nobody able to answer.
    v.push("-o".into());
    v.push("BatchMode=yes".into());
    v.push(format!("{}@{}", target.user, target.host));
    v.push(script.to_string());
    v
}

/// Final and staged paths for an artifact.
///
/// The staged name carries the digest, so two deployments of the same artifact converge on one
/// name and two of different artifacts cannot collide. Both live in the **same directory**,
/// which is what makes the rename atomic — a cross-device rename fails, and a fallback copy
/// would reopen the window a staged file exists to close.
pub fn paths(artifact: &EngineArtifact) -> (String, String) {
    let final_path = format!("{REMOTE_DIR}/ide-engine-{}", artifact.version);
    let staged = format!("{REMOTE_DIR}/.staged-{}", artifact.digest);
    (final_path, staged)
}

fn classify(stderr: &str) -> DeploymentFailure {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("no space left") {
        DeploymentFailure::NoSpace
    } else if lower.contains("permission denied") || lower.contains("read-only file system") {
        DeploymentFailure::PermissionDenied {
            path: REMOTE_DIR.into(),
        }
    } else {
        DeploymentFailure::TransferInterrupted
    }
}

impl SshStreamDeployer {
    fn run(&self, target: &Target, script: &str) -> Result<String, DeploymentFailure> {
        let out = Command::new("ssh")
            .args(remote_command(target, script))
            .env("LC_ALL", "C")
            .output()
            .map_err(|e| DeploymentFailure::PermissionDenied {
                path: format!("could not run ssh: {e}"),
            })?;
        if !out.status.success() {
            return Err(classify(&String::from_utf8_lossy(&out.stderr)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

impl ArtifactDeployer for SshStreamDeployer {
    fn remote_architecture(&self, target: &Target) -> Result<String, DeploymentFailure> {
        self.run(target, "uname -m")
    }

    fn deploy(&self, artifact: &EngineArtifact, target: &Target) -> Result<(), DeploymentFailure> {
        let (final_path, staged) = paths(artifact);
        let _ = self.state.send(DeploymentState::Preparing);

        // Idempotence: an artifact already in place with a matching digest is not re-sent.
        // Checked before transferring, because the cheapest transfer is the one that does not
        // happen — and on a reconnect to a host used yesterday, that is every transfer.
        if let Ok(existing) = self.run(
            target,
            &format!("sha256sum {final_path} 2>/dev/null | cut -d' ' -f1"),
        ) {
            if existing == artifact.digest.as_hex() {
                let _ = self.state.send(DeploymentState::Complete);
                return Ok(());
            }
        }

        self.run(target, &format!("mkdir -p {REMOTE_DIR}"))?;

        // Stream to the staged path. Written by us rather than by `scp` or `sftp` because
        // progress has to be observable at least once a second, and only a loop we drive can
        // count bytes as they go — the alternatives print a progress bar meant for a human.
        let mut child = Command::new("ssh")
            .args(remote_command(target, &format!("cat > {staged}")))
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| DeploymentFailure::PermissionDenied {
                path: format!("could not run ssh: {e}"),
            })?;

        let total = artifact.bytes.len() as u64;
        {
            let mut stdin = child.stdin.take().expect("piped stdin");
            let mut ticker = ProgressTicker::new();
            let mut sent: u64 = 0;
            let _ = self
                .state
                .send(DeploymentState::Transferring { sent, total });
            for chunk in artifact.bytes.chunks(CHUNK) {
                if stdin.write_all(chunk).is_err() {
                    break;
                }
                sent += chunk.len() as u64;
                if ticker.should_publish(sent, total) {
                    let _ = self
                        .state
                        .send(DeploymentState::Transferring { sent, total });
                }
            }
            let _ = stdin.flush();
            // Dropping stdin closes it, which is what ends `cat` on the far side.
        }

        let done = child.wait_with_output().map_err(|_| {
            let _ = self.state.send(DeploymentState::Failed(
                DeploymentFailure::TransferInterrupted,
            ));
            DeploymentFailure::TransferInterrupted
        })?;
        if !done.status.success() {
            let f = classify(&String::from_utf8_lossy(&done.stderr));
            let _ = self.state.send(DeploymentState::Failed(f.clone()));
            return Err(f);
        }

        // Verify where the file landed, with the host's own tool. The algorithm is SHA-256
        // because `sha256sum` is what exists there — a faster hash would need a binary
        // deployed in order to verify a deployment.
        let _ = self.state.send(DeploymentState::Verifying);
        let landed = self.run(target, &format!("sha256sum {staged} | cut -d' ' -f1"))?;
        if landed != artifact.digest.as_hex() {
            // Remove the staged file so a retry is not confused by it, then report. Not
            // tampering: the check cannot tell corruption from tampering.
            let _ = self.run(target, &format!("rm -f {staged}"));
            let _ = self
                .state
                .send(DeploymentState::Failed(DeploymentFailure::DigestMismatch));
            return Err(DeploymentFailure::DigestMismatch);
        }

        // Executable only after verification, then renamed into place. Two independent
        // defences: a truncated transfer is both unverified and unrunnable.
        let _ = self.state.send(DeploymentState::Promoting);
        self.run(
            target,
            &format!("chmod 755 {staged} && mv {staged} {final_path}"),
        )?;
        let _ = self.state.send(DeploymentState::Complete);
        Ok(())
    }

    fn retire_previous(&self, version: &str, target: &Target) -> Result<(), DeploymentFailure> {
        // Failing here is not a failure of the update: the new engine is running, and an
        // orphaned binary costs disk rather than correctness.
        self.run(target, &format!("rm -f {REMOTE_DIR}/ide-engine-{version}"))?;
        Ok(())
    }

    fn observe(&self) -> watch::Receiver<DeploymentState> {
        self.keep.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::artifact::{Architecture, Digest};

    fn art() -> EngineArtifact {
        EngineArtifact {
            version: "0.2.0".into(),
            protocol_version: 1,
            architecture: Architecture::LinuxX86_64,
            digest: Digest::parse(&"c".repeat(64)).expect("valid"),
            bytes: b"",
        }
    }

    /// The deployer must reuse the transport's master. A different ControlPath would open a
    /// second connection and authenticate again — everything would still work and quietly
    /// cost twice as much, which is the kind of defect nothing reports.
    #[test]
    fn every_remote_command_reuses_the_transports_control_master() {
        let t = Target {
            host: "build-01".into(),
            user: "dev".into(),
        };
        let args = remote_command(&t, "uname -m");
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-o" && w[1] == "ControlPath=~/.ssh/apex-%C"),
            "the deployer must attach to F001's master: {args:?}"
        );
        assert!(args.windows(2).any(|w| w[1] == "ControlMaster=auto"));
        assert_eq!(args[args.len() - 2], "dev@build-01");
        assert_eq!(args[args.len() - 1], "uname -m");
    }

    /// No tty and no askpass sit behind a deployment, so a prompt would hang a transfer with
    /// nobody able to answer it.
    #[test]
    fn a_deployment_can_never_prompt() {
        let t = Target {
            host: "h".into(),
            user: "u".into(),
        };
        let args = remote_command(&t, "true");
        assert!(args.windows(2).any(|w| w[1] == "BatchMode=yes"), "{args:?}");
    }

    /// Staged and final must share a directory: `rename` is atomic only within a filesystem,
    /// and a cross-device rename silently degrades to a copy, reopening the window.
    #[test]
    fn the_staged_path_sits_beside_the_final_one_and_carries_the_digest() {
        let a = art();
        let (final_path, staged) = paths(&a);
        let dir = |p: &str| p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap();
        assert_eq!(dir(&final_path), dir(&staged), "must share a directory");
        assert!(staged.contains(a.digest.as_hex()), "{staged}");
        assert!(final_path.contains("0.2.0"), "{final_path}");
    }

    /// Two deployments of one artifact converge; two of different artifacts cannot collide.
    #[test]
    fn staged_names_converge_for_one_artifact_and_differ_between_artifacts() {
        let a = art();
        let mut b = art();
        b.digest = Digest::parse(&"d".repeat(64)).expect("valid");
        assert_eq!(paths(&a).1, paths(&a.clone()).1);
        assert_ne!(paths(&a).1, paths(&b).1);
    }

    #[test]
    fn a_full_disk_is_not_reported_as_an_interrupted_transfer() {
        assert_eq!(
            classify("cat: write error: No space left on device"),
            DeploymentFailure::NoSpace
        );
        assert!(matches!(
            classify("bash: /home/dev/.apex: Permission denied"),
            DeploymentFailure::PermissionDenied { .. }
        ));
        assert_eq!(
            classify("client_loop: send disconnect"),
            DeploymentFailure::TransferInterrupted
        );
    }
}
