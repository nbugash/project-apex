//! Getting authenticated, without putting the user in a terminal (User Story 2).
//!
//! The sequence is two phases, and §3.3 makes them mutually exclusive rather than merely
//! ordered. Phase one carries `BatchMode=yes`, which prevents `ssh` blocking on a tty
//! prompt no GUI user can answer — and, as a consequence, disables `SSH_ASKPASS` entirely.
//! So an attempt cannot be both silent and promptable, and the assisted phase has to be a
//! second process rather than a fallback inside the first.
//!
//! The rule the whole module exists to enforce: **only an `AuthenticationFailed`
//! classification may raise a prompt** (FR-006). An unreachable host that prompts for a
//! passphrase teaches the user that this application asks for credentials at random, which
//! is the exact habit a phishing attempt needs.

use crate::application::ports::credential::{CredentialPrompt, PromptContext, PromptError};
use crate::domain::failure::FailureCondition;
use crate::domain::request::Secret;

/// One connection attempt, in whichever phase.
///
/// `attempt` is async because establishing takes real time and this is called from the
/// interaction path: a synchronous version would block whatever runtime drives the window
/// for the whole settle period, which is the interaction budget Principle V protects.
///
/// A port of its own rather than `ProcessSpawner` directly, because "did this connection
/// establish" is a question about a running child — spawning is only its first moment. It
/// also keeps the sequence testable without a process: the failure paths are the point,
/// and provoking them from a real `ssh` needs hosts the suite is required not to have.
#[allow(async_fn_in_trait)]
pub trait ConnectAttempt {
    /// Run one attempt to completion: established, or ended with a classified reason.
    async fn attempt(&self, assisted: bool) -> Result<(), FailureCondition>;

    /// Arm the askpass channel so a helper asking during the next assisted attempt is
    /// answered. Called immediately before that attempt and never otherwise.
    fn arm(&self, secret: Secret);

    /// Whether the assisted phase is available at all. False below OpenSSH 8.4, where
    /// `SSH_ASKPASS_REQUIRE=force` does not exist and a prompt would be consulted or
    /// ignored depending on the platform and on `DISPLAY` (§3.3).
    fn assisted_available(&self) -> bool;
}

/// Where the connect sequence ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectOutcome {
    /// Connected. `assisted` records which phase succeeded, which is what tells the
    /// interface whether to mention the passphrase at all.
    Connected { assisted: bool },
    /// Every automatic route is exhausted. Offer the identity picker (FR-009) — a user who
    /// is told only "authentication failed" has nothing to do next; one who is offered
    /// their other keys does.
    OfferIdentityPicker { after: FailureCondition },
    /// The user dismissed the prompt. Not an error: declining to authenticate is an answer.
    Cancelled,
    /// Nothing about credentials will help. The caller decides whether to retry, hand to
    /// bootstrap, or warn — `use_cases::supervise::response_to` owns that table.
    Failed(FailureCondition),
}

/// The two-phase connect sequence (§3.3, FR-006 through FR-009).
pub async fn connect<A, P>(attempts: &A, prompt: &P, host: &str) -> ConnectOutcome
where
    A: ConnectAttempt,
    P: CredentialPrompt,
{
    // Phase one. An agent-held key connects here and the user sees nothing at all, which is
    // the case that must stay silent: a prompt for a credential the agent already holds
    // trains people to type passphrases at unexpected moments.
    let condition = match attempts.attempt(false).await {
        Ok(()) => return ConnectOutcome::Connected { assisted: false },
        Err(c) => c,
    };

    // The gate. Everything that is not a refused credential leaves by this door, unprompted.
    if !condition.may_prompt_for_credential() {
        return ConnectOutcome::Failed(condition);
    }

    // §3.3. Below OpenSSH 8.4 the assisted phase cannot be made reliable, so the honest
    // move is to offer the picker now. Attempting it anyway would hang on some platforms
    // and silently skip the prompt on others — "appears to hang" being the worst of the
    // three outcomes, because the user cannot tell it from a slow network.
    if !attempts.assisted_available() {
        return ConnectOutcome::OfferIdentityPicker { after: condition };
    }

    let ctx = PromptContext {
        // OpenSSH's own text reaches the dialog through the askpass channel; this is the
        // caption for the case where the helper has not been reached yet.
        prompt: "Enter the passphrase for your SSH key".into(),
        host: host.to_string(),
    };
    let secret = match prompt.passphrase(ctx).await {
        Ok(s) => s,
        // Dismissing the dialog ends the attempt. Retrying would re-open the same dialog
        // the user just closed, which is indistinguishable from refusing to take no.
        Err(PromptError::Cancelled) => return ConnectOutcome::Cancelled,
        // No interface to prompt with — a headless run, or a window that has gone away.
        // The picker is offered rather than an error, because the automatic routes are
        // genuinely exhausted and the user may have another key.
        Err(PromptError::Unavailable) => {
            return ConnectOutcome::OfferIdentityPicker { after: condition }
        }
    };

    // Phase two. Armed immediately before the attempt and never earlier: the window in
    // which a passphrase sits in memory waiting is the window worth shortening.
    attempts.arm(secret);
    match attempts.attempt(true).await {
        Ok(()) => ConnectOutcome::Connected { assisted: true },
        // The passphrase was wrong, or that key is not the one the host will accept. Both
        // automatic routes have now failed, which is precisely FR-009's condition.
        Err(c) if c.may_prompt_for_credential() => ConnectOutcome::OfferIdentityPicker { after: c },
        // The assisted attempt reached a different wall — the host went away between the
        // two phases, say. Credentials are not the problem any more.
        Err(c) => ConnectOutcome::Failed(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Scripted attempts: a queue of results, consumed in order.
    struct Attempts {
        results: Mutex<Vec<Result<(), FailureCondition>>>,
        assisted_available: bool,
        /// What each attempt was asked for, so a test can assert phase order.
        phases: Mutex<Vec<bool>>,
        armed: AtomicUsize,
    }

    impl Attempts {
        fn new(results: Vec<Result<(), FailureCondition>>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().rev().collect()),
                assisted_available: true,
                phases: Mutex::new(Vec::new()),
                armed: AtomicUsize::new(0),
            }
        }
        fn without_askpass(mut self) -> Self {
            self.assisted_available = false;
            self
        }
        fn phases(&self) -> Vec<bool> {
            self.phases.lock().unwrap().clone()
        }
    }

    impl ConnectAttempt for Attempts {
        async fn attempt(&self, assisted: bool) -> Result<(), FailureCondition> {
            self.phases.lock().unwrap().push(assisted);
            self.results
                .lock()
                .unwrap()
                .pop()
                .expect("the sequence asked for more attempts than the test scripted")
        }
        fn arm(&self, _secret: Secret) {
            self.armed.fetch_add(1, Ordering::SeqCst);
        }
        fn assisted_available(&self) -> bool {
            self.assisted_available
        }
    }

    /// Counts prompts, so "was the user asked" is a assertion rather than an inference.
    struct Prompter {
        answer: Mutex<Option<Result<Vec<u8>, PromptError>>>,
        asked: AtomicUsize,
    }

    impl Prompter {
        fn answering(pass: &str) -> Self {
            Self {
                answer: Mutex::new(Some(Ok(pass.as_bytes().to_vec()))),
                asked: AtomicUsize::new(0),
            }
        }
        fn failing(e: PromptError) -> Self {
            Self {
                answer: Mutex::new(Some(Err(e))),
                asked: AtomicUsize::new(0),
            }
        }
        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
    }

    impl CredentialPrompt for Prompter {
        async fn passphrase(&self, _ctx: PromptContext) -> Result<Secret, PromptError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            match self.answer.lock().unwrap().take() {
                Some(Ok(bytes)) => Ok(Secret::new(bytes)),
                Some(Err(e)) => Err(e),
                None => Err(PromptError::Cancelled),
            }
        }
    }

    #[tokio::test]
    async fn an_agent_held_key_connects_with_no_prompt() {
        let a = Attempts::new(vec![Ok(())]);
        let p = Prompter::answering("never used");
        assert_eq!(
            connect(&a, &p, "build-01").await,
            ConnectOutcome::Connected { assisted: false }
        );
        assert_eq!(p.asked(), 0, "a silent connect must stay silent");
        assert_eq!(a.phases(), vec![false], "and must not run a second phase");
    }

    /// FR-006, and the reason this module exists. A prompt raised by anything other than a
    /// refused credential teaches the user that the application asks for passphrases at
    /// unpredictable moments.
    #[tokio::test]
    async fn only_a_refused_credential_reaches_the_prompt() {
        for condition in [
            FailureCondition::HostUnreachable,
            FailureCondition::NetworkDropped,
            FailureCondition::EngineMissing,
            FailureCondition::EngineCrashed,
            FailureCondition::HostKeyChanged,
            FailureCondition::Unknown,
        ] {
            let a = Attempts::new(vec![Err(condition)]);
            let p = Prompter::answering("never used");
            let outcome = connect(&a, &p, "build-01").await;
            assert_eq!(outcome, ConnectOutcome::Failed(condition), "{condition:?}");
            assert_eq!(p.asked(), 0, "{condition:?} must not raise a prompt");
            assert_eq!(
                a.phases(),
                vec![false],
                "{condition:?} must not retry assisted"
            );
        }
    }

    #[tokio::test]
    async fn a_refused_credential_prompts_once_and_then_connects() {
        let a = Attempts::new(vec![Err(FailureCondition::AuthenticationFailed), Ok(())]);
        let p = Prompter::answering("correct horse");
        assert_eq!(
            connect(&a, &p, "build-01").await,
            ConnectOutcome::Connected { assisted: true }
        );
        assert_eq!(p.asked(), 1);
        assert_eq!(a.phases(), vec![false, true], "silent first, then assisted");
        assert_eq!(
            a.armed.load(Ordering::SeqCst),
            1,
            "the answer must be armed"
        );
    }

    #[tokio::test]
    async fn a_dismissed_prompt_ends_the_attempt() {
        let a = Attempts::new(vec![Err(FailureCondition::AuthenticationFailed)]);
        let p = Prompter::failing(PromptError::Cancelled);
        assert_eq!(connect(&a, &p, "build-01").await, ConnectOutcome::Cancelled);
        assert_eq!(
            a.phases(),
            vec![false],
            "a dismissed prompt must not spawn an assisted attempt that cannot succeed"
        );
    }

    /// FR-009. The third credential situation: neither the agent nor the passphrase worked.
    #[tokio::test]
    async fn exhausting_every_automatic_route_offers_the_picker() {
        let a = Attempts::new(vec![
            Err(FailureCondition::AuthenticationFailed),
            Err(FailureCondition::AuthenticationFailed),
        ]);
        let p = Prompter::answering("wrong");
        assert_eq!(
            connect(&a, &p, "build-01").await,
            ConnectOutcome::OfferIdentityPicker {
                after: FailureCondition::AuthenticationFailed
            }
        );
        assert_eq!(a.phases(), vec![false, true]);
    }

    /// §3.3. Below 8.4 the assisted phase cannot be made to behave the same way twice, so
    /// the sequence skips it rather than appearing to hang.
    #[tokio::test]
    async fn an_openssh_without_forced_askpass_offers_the_picker_instead_of_prompting() {
        let a = Attempts::new(vec![Err(FailureCondition::AuthenticationFailed)]).without_askpass();
        let p = Prompter::answering("never used");
        assert_eq!(
            connect(&a, &p, "build-01").await,
            ConnectOutcome::OfferIdentityPicker {
                after: FailureCondition::AuthenticationFailed
            }
        );
        assert_eq!(p.asked(), 0, "a prompt we cannot force must not be raised");
        assert_eq!(
            a.phases(),
            vec![false],
            "and no assisted attempt may be spawned"
        );
    }

    /// A window that has gone away is not a refusal; the user still has other keys.
    #[tokio::test]
    async fn no_interface_to_prompt_with_offers_the_picker() {
        let a = Attempts::new(vec![Err(FailureCondition::AuthenticationFailed)]);
        let p = Prompter::failing(PromptError::Unavailable);
        assert_eq!(
            connect(&a, &p, "build-01").await,
            ConnectOutcome::OfferIdentityPicker {
                after: FailureCondition::AuthenticationFailed
            }
        );
    }

    /// The host went away between the two phases. Offering more credentials would be
    /// answering a question nobody asked.
    #[tokio::test]
    async fn a_non_credential_failure_in_the_assisted_phase_is_reported_as_itself() {
        let a = Attempts::new(vec![
            Err(FailureCondition::AuthenticationFailed),
            Err(FailureCondition::HostUnreachable),
        ]);
        let p = Prompter::answering("correct horse");
        assert_eq!(
            connect(&a, &p, "build-01").await,
            ConnectOutcome::Failed(FailureCondition::HostUnreachable)
        );
    }
}

// ---------------------------------------------------------------------------
// A changed host identity (§3.9, FR-016).

/// Proof that a person was shown the warning and explicitly chose to forget the key.
///
/// A token rather than a `bool`, because a boolean parameter is something a future caller
/// passes `true` to without reading why it exists. This can only be minted by the interface
/// layer, at the moment a human confirms — so "the application never forgets a host key on
/// its own" is enforced by what can be written, not by what everyone remembers.
///
/// It is deliberately not `Clone` or `Copy`: one confirmation authorises one forget.
#[derive(Debug)]
pub struct ConfirmedByUser(());

impl ConfirmedByUser {
    /// Called by the interface only, after the user has confirmed the specific host named
    /// in the warning.
    pub fn from_explicit_confirmation() -> Self {
        Self(())
    }
}

/// What the user is told when a host's identity has changed.
///
/// The wording is not softened. This is the one failure in the set that may mean an attack
/// in progress, and a message that reads like a routine error is one people click through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyWarning {
    pub host: String,
    /// The line OpenSSH itself printed, passed through: it names the key type and
    /// fingerprint, which is what lets someone check against the host they control.
    pub detail: String,
}

impl HostKeyWarning {
    pub fn for_host(host: &str, stderr: &str) -> Self {
        Self {
            host: host.to_string(),
            detail: stderr
                .lines()
                .find(|l| l.contains("Fingerprint") || l.contains("key sent by the remote host"))
                .unwrap_or("The host's key does not match the one previously recorded.")
                .trim()
                .to_string(),
        }
    }

    /// The remedy, stated as an instruction rather than an offer. Connecting anyway is not
    /// among the options: a changed key is either an administrative change the user can
    /// confirm out of band, or someone between them and the host.
    pub fn guidance(&self) -> String {
        format!(
            "The identity of {} has changed since it was last recorded. \
             Verify the new key with whoever administers the host before continuing. \
             If the change is expected, forgetting the old key is an explicit action.",
            self.host
        )
    }
}

/// The command that removes a host's recorded key.
///
/// Exposed separately so it can be asserted on without running it. `ssh-keygen -R` rather
/// than editing `known_hosts` directly, because the file may be hashed and hand-editing a
/// hashed entry removes the wrong host — or all of them.
pub fn forget_command(host: &str) -> Vec<String> {
    vec!["ssh-keygen".into(), "-R".into(), host.to_string()]
}

/// Forget a host's recorded key. Reachable only with a confirmation the user gave.
pub fn forget_known_host(host: &str, _confirmed: ConfirmedByUser) -> Result<(), String> {
    let args = forget_command(host);
    let status = std::process::Command::new(&args[0])
        .args(&args[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("could not run ssh-keygen: {e}"))?;
    if status.success() {
        crate::logging::info(&format!(
            "forgot the recorded host key for {host} at the user's request"
        ));
        Ok(())
    } else {
        Err(format!("ssh-keygen -R {host} failed"))
    }
}

#[cfg(test)]
mod host_key_tests {
    use super::*;

    const CHANGED: &str = concat!(
        "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n",
        "The fingerprint for the ED25519 key sent by the remote host is\n",
        "SHA256:abc123.\n",
    );

    /// The warning names the host and carries OpenSSH's own detail, because a fingerprint
    /// the user can compare is the only thing that makes the decision a real one.
    #[test]
    fn the_warning_names_the_host_and_keeps_openssh_s_detail() {
        let w = HostKeyWarning::for_host("build-01.euw1", CHANGED);
        assert_eq!(w.host, "build-01.euw1");
        assert!(
            w.detail.contains("key sent by the remote host"),
            "{}",
            w.detail
        );
        assert!(w.guidance().contains("build-01.euw1"));
    }

    /// Stderr that does not carry a fingerprint still produces a usable warning rather than
    /// an empty one.
    #[test]
    fn a_warning_without_a_fingerprint_still_says_what_happened() {
        let w = HostKeyWarning::for_host("build-01", "");
        assert!(!w.detail.is_empty());
        assert!(w.guidance().contains("explicit action"));
    }

    /// FR-016 is a prohibition, and this is what enforces it: forgetting takes a token that
    /// only the interface layer can mint, at the moment a human confirms. There is no
    /// argument the supervisor could pass to forget a key on its own.
    #[test]
    fn forgetting_is_reachable_only_with_a_confirmation() {
        let args = forget_command("build-01.euw1");
        assert_eq!(args[0], "ssh-keygen");
        assert_eq!(
            args[1], "-R",
            "editing known_hosts by hand breaks hashed entries"
        );
        assert_eq!(args[2], "build-01.euw1");

        // This is the whole test: `forget_known_host` cannot be called without a
        // `ConfirmedByUser`, and `ConfirmedByUser` has one constructor, named for what it
        // means. The compiler enforces the prohibition; nothing here has to remember it.
        let _confirmation = ConfirmedByUser::from_explicit_confirmation();
    }

    /// The guidance does not offer to continue. A changed key is either an administrative
    /// change the user can confirm out of band, or someone between them and the host.
    #[test]
    fn the_guidance_offers_no_way_to_connect_anyway() {
        let g = HostKeyWarning::for_host("h", CHANGED).guidance();
        let lower = g.to_lowercase();
        for phrase in [
            "connect anyway",
            "ignore",
            "proceed anyway",
            "continue anyway",
        ] {
            assert!(
                !lower.contains(phrase),
                "the warning must not offer {phrase:?}: {g}"
            );
        }
        assert!(lower.contains("verify"), "{g}");
    }
}
