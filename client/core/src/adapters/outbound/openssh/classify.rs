//! Turning an exit code and some stderr into a named condition (§3.4).
//!
//! Classification runs against stderr produced under `LC_ALL=C`, which the §3.1 invocation
//! pins. That is what makes SC-008 achievable: the patterns below are English because
//! OpenSSH's C locale is English, not because the user's machine is.
//!
//! Matching is deliberately narrow. A pattern set that tries to be clever produces confident
//! wrong answers, and a wrong classification is worse than `Unknown` — it sends the user
//! down a remedy that cannot work.

use crate::domain::failure::{FailureCondition, MAX_STDERR_BYTES};

/// Classify a terminated attempt.
///
/// `exit_code` is the child's status; `stderr` is whatever it wrote, already bounded by the
/// caller. Only the last `MAX_STDERR_BYTES` are considered even if more is handed in, so a
/// noisy engine degrades classification rather than exhausting memory.
pub fn classify(exit_code: i32, stderr: &str) -> FailureCondition {
    let tail = bounded_tail(stderr);
    let lower = tail.to_ascii_lowercase();

    // Order matters: a changed host key also exits 255, and must be recognised before the
    // generic authentication patterns claim it.
    if tail.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        return FailureCondition::HostKeyChanged;
    }

    match exit_code {
        127 => FailureCondition::EngineMissing,
        255 => {
            if lower.contains("permission denied") || lower.contains("too many authentication") {
                FailureCondition::AuthenticationFailed
            } else if lower.contains("connection timed out")
                || lower.contains("connection refused")
                || lower.contains("no route to host")
                || lower.contains("could not resolve hostname")
                || lower.contains("network is unreachable")
            {
                FailureCondition::HostUnreachable
            } else if lower.contains("connection closed")
                || lower.contains("broken pipe")
                || lower.contains("timeout, server")
            {
                // "Timeout, server ... not responding" is what OpenSSH prints when
                // ServerAliveCountMax is exceeded — the keepalive giving up.
                FailureCondition::NetworkDropped
            } else {
                FailureCondition::Unknown
            }
        }
        0 => FailureCondition::NetworkDropped, // a clean exit mid-session is still a loss
        _ => FailureCondition::EngineCrashed,
    }
}

/// Keep only the last `MAX_STDERR_BYTES`, on a character boundary.
///
/// The tail rather than the head: OpenSSH's decisive line is the last thing it writes, and a
/// verbose engine would otherwise push it out of a head-bounded buffer.
fn bounded_tail(stderr: &str) -> &str {
    if stderr.len() <= MAX_STDERR_BYTES {
        return stderr;
    }
    let mut start = stderr.len() - MAX_STDERR_BYTES;
    while start < stderr.len() && !stderr.is_char_boundary(start) {
        start += 1;
    }
    &stderr[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DENIED: &str = "user@host: Permission denied (publickey,password).";
    const TIMED_OUT: &str = "ssh: connect to host h port 22: Connection timed out";
    const REFUSED: &str = "ssh: connect to host h port 22: Connection refused";
    const CHANGED: &str = "@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @";
    const NOT_FOUND: &str = "bash: /usr/local/bin/ide-engine: No such file or directory";

    #[test]
    fn each_condition_classifies_as_itself() {
        assert_eq!(
            classify(255, DENIED),
            FailureCondition::AuthenticationFailed
        );
        assert_eq!(classify(255, TIMED_OUT), FailureCondition::HostUnreachable);
        assert_eq!(classify(255, REFUSED), FailureCondition::HostUnreachable);
        assert_eq!(classify(255, CHANGED), FailureCondition::HostKeyChanged);
        assert_eq!(classify(127, NOT_FOUND), FailureCondition::EngineMissing);
        assert_eq!(
            classify(1, "engine panicked"),
            FailureCondition::EngineCrashed
        );
        assert_eq!(classify(0, ""), FailureCondition::NetworkDropped);
    }

    /// A changed host key exits 255 like an auth failure. If the generic patterns ever claim
    /// it first, the user gets a passphrase prompt in response to a possible attack.
    #[test]
    fn a_changed_host_key_is_recognised_before_authentication_patterns() {
        let both = format!("{CHANGED}\nPermission denied (publickey).");
        assert_eq!(classify(255, &both), FailureCondition::HostKeyChanged);
    }

    #[test]
    fn an_unrecognised_failure_is_unknown_not_guessed() {
        assert_eq!(
            classify(255, "ssh: something nobody has seen before"),
            FailureCondition::Unknown
        );
    }

    #[test]
    fn keepalive_expiry_reads_as_a_dropped_network() {
        assert_eq!(
            classify(255, "Timeout, server host not responding."),
            FailureCondition::NetworkDropped
        );
    }

    /// SC-008. The invocation pins `LC_ALL=C`, so a localised message means that pin was
    /// lost — and the honest answer is `Unknown`, not a guess from a language we did not
    /// ask for. What must never happen is a *different* wrong classification.
    #[test]
    fn a_localised_message_does_not_produce_a_wrong_classification() {
        let french = "user@host: Permission refusée (publickey,password).";
        let got = classify(255, french);
        assert_ne!(
            got,
            FailureCondition::HostUnreachable,
            "a localised refusal must not be mistaken for an unreachable host"
        );
        assert_eq!(
            got,
            FailureCondition::Unknown,
            "without the C locale the text is not ours to interpret"
        );
        // And the exit code alone still carries the important distinctions.
        assert_eq!(
            classify(127, "fichier introuvable"),
            FailureCondition::EngineMissing
        );
    }

    /// The same failure under `LC_ALL=C` — which is what the invocation guarantees — is
    /// classified identically regardless of the user's system language.
    #[test]
    fn the_c_locale_message_classifies_identically_whatever_the_system_language() {
        assert_eq!(
            classify(255, DENIED),
            FailureCondition::AuthenticationFailed
        );
        assert_eq!(
            classify(255, DENIED),
            FailureCondition::AuthenticationFailed
        );
    }

    #[test]
    fn stderr_beyond_the_bound_degrades_rather_than_exhausting_memory() {
        let noise = "x".repeat(MAX_STDERR_BYTES * 4);
        assert_eq!(classify(255, &noise), FailureCondition::Unknown);

        // The decisive line survives when it is the last thing written, which is where
        // OpenSSH puts it.
        let noisy_then_denied = format!("{noise}\n{DENIED}");
        assert_eq!(
            classify(255, &noisy_then_denied),
            FailureCondition::AuthenticationFailed
        );
    }

    #[test]
    fn bounding_never_splits_a_character() {
        let s = "é".repeat(MAX_STDERR_BYTES);
        let _ = bounded_tail(&s); // must not panic on a non-boundary slice
    }
}
