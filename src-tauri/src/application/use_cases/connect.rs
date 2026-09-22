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
