//! What the client carries to a remote host, and how a deployment of it goes.
//!
//! Field rules are in specs/004-daemon-bootstrap/data-model.md and are not restated.

use std::fmt;

/// A target the engine can run on. Closed, because the client can only carry builds it was
/// built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    LinuxX86_64,
    LinuxAarch64,
}

impl Architecture {
    /// Parse what `uname -m` reports on the remote host.
    ///
    /// Unknown values yield `None` rather than a guess. A wrong guess deploys a binary that
    /// cannot execute, which fails later and far less clearly than a refusal naming the
    /// architecture.
    pub fn from_uname(machine: &str) -> Option<Self> {
        match machine.trim() {
            "x86_64" | "amd64" => Some(Self::LinuxX86_64),
            "aarch64" | "arm64" => Some(Self::LinuxAarch64),
            _ => None,
        }
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::LinuxAarch64 => "linux-aarch64",
        })
    }
}

/// A SHA-256 hash as lowercase hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest(String);

impl Digest {
    pub fn parse(hex: &str) -> Option<Self> {
        let hex = hex.trim().to_ascii_lowercase();
        let valid = hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit());
        valid.then_some(Self(hex))
    }

    pub fn as_hex(&self) -> &str {
        &self.0
    }

    /// Exact comparison. Never a prefix: a prefix test is a weaker check that looks identical
    /// in a passing test and accepts a different artifact in production.
    pub fn matches(&self, other: &Digest) -> bool {
        self.0 == other.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One engine build the client carries.
#[derive(Debug, Clone)]
pub struct EngineArtifact {
    pub version: String,
    pub protocol_version: u32,
    pub architecture: Architecture,
    pub digest: Digest,
    pub bytes: &'static [u8],
}

/// Where one deployment attempt has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentState {
    Preparing,
    /// Carries counts because a state without them cannot satisfy a progress requirement —
    /// "still transferring" with no number is indistinguishable from a stall.
    Transferring {
        sent: u64,
        total: u64,
    },
    Verifying,
    Promoting,
    Complete,
    Failed(DeploymentFailure),
}

/// Why a deployment did not complete.
///
/// Six causes rather than one error, because each needs a different response from the
/// developer: a full disk, an unwritable directory, an unsupported architecture and a corrupt
/// transfer are four different problems, and collapsing them sends people to debug the wrong
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentFailure {
    UnsupportedArchitecture {
        found: String,
    },
    TransferInterrupted,
    NoSpace,
    /// What landed is not what was sent. Reported as a failed deployment, never as tampering:
    /// the check cannot distinguish those, and claiming the stronger one would be a guess.
    DigestMismatch,
    NotExecutable,
    PermissionDenied {
        path: String,
    },
}

impl fmt::Display for DeploymentFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedArchitecture { found } => {
                write!(f, "no engine build for this host's architecture ({found})")
            }
            Self::TransferInterrupted => write!(f, "the transfer was interrupted"),
            Self::NoSpace => write!(f, "the remote host has no space left"),
            Self::DigestMismatch => {
                write!(f, "the deployed engine failed verification and was not run")
            }
            Self::NotExecutable => write!(f, "the deployed engine will not run on this host"),
            Self::PermissionDenied { path } => write!(f, "cannot write to {path}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_compares_exactly_and_never_by_prefix() {
        let full = Digest::parse(&"a".repeat(64)).expect("valid");
        let other = Digest::parse(&format!("{}b", "a".repeat(63))).expect("valid");
        assert!(full.matches(&full.clone()));
        assert!(
            !full.matches(&other),
            "digests differing in one character must not match"
        );
    }

    #[test]
    fn a_digest_is_rejected_unless_it_is_a_full_sha256() {
        assert!(Digest::parse("abc").is_none(), "too short");
        assert!(Digest::parse(&"a".repeat(63)).is_none(), "one short");
        assert!(Digest::parse(&"z".repeat(64)).is_none(), "not hex");
        assert!(
            Digest::parse(&"A".repeat(64)).is_some(),
            "case is normalised"
        );
    }

    #[test]
    fn an_unknown_architecture_is_none_rather_than_a_guess() {
        assert_eq!(
            Architecture::from_uname("x86_64"),
            Some(Architecture::LinuxX86_64)
        );
        assert_eq!(
            Architecture::from_uname("aarch64"),
            Some(Architecture::LinuxAarch64)
        );
        assert_eq!(Architecture::from_uname("riscv64"), None);
        assert_eq!(Architecture::from_uname(""), None);
    }

    /// Each cause must say something different. A failure set where two members produce the
    /// same sentence is a failure set that could have been one member.
    #[test]
    fn every_failure_cause_reads_differently() {
        let all = [
            DeploymentFailure::UnsupportedArchitecture {
                found: "riscv64".into(),
            },
            DeploymentFailure::TransferInterrupted,
            DeploymentFailure::NoSpace,
            DeploymentFailure::DigestMismatch,
            DeploymentFailure::NotExecutable,
            DeploymentFailure::PermissionDenied {
                path: "/opt".into(),
            },
        ];
        let messages: std::collections::HashSet<String> =
            all.iter().map(|f| f.to_string()).collect();
        assert_eq!(messages.len(), all.len(), "two causes read the same");
    }

    /// A digest mismatch must not accuse the host. The check cannot tell corruption from
    /// tampering, and the spec's threat model says so.
    #[test]
    fn a_digest_mismatch_does_not_claim_tampering() {
        let m = DeploymentFailure::DigestMismatch.to_string().to_lowercase();
        for word in ["tamper", "attack", "compromis", "malicious"] {
            assert!(!m.contains(word), "{m}");
        }
        assert!(m.contains("verification"));
    }
}
