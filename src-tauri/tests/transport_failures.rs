//! User Story 2: authenticate without leaving the application.
//!
//! Three credential situations, and the story is only satisfied when all three reach an
//! outcome the user can act on without a terminal: a key the agent holds, a key with a
//! passphrase, and neither.
//!
//! Everything here runs against the mock or a scripted `ssh` — no remote host, no network
//! (FR-019). Driving a real `ssh` into a refused credential, a changed host key and a
//! missing engine on demand would need three hosts or one repeatedly reconfigured.

mod common;

use apex_shell::adapters::outbound::askpass::ipc::AskpassChannel;
use apex_shell::adapters::outbound::openssh::SshTransport;
use apex_shell::application::ports::credential::{CredentialPrompt, PromptContext, PromptError};
use apex_shell::application::use_cases::connect::{connect, ConnectOutcome};
use apex_shell::domain::failure::FailureCondition;
use apex_shell::domain::request::Secret;
use common::{stderr, AssistedSpawner, MockSpawner};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const HOST: &str = "build-01.euw1";

/// Counts prompts, so "was the user asked" is an assertion rather than an inference.
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
    fn refusing(e: PromptError) -> Self {
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
        match self.answer.lock().expect("answer lock").take() {
            Some(Ok(bytes)) => Ok(Secret::new(bytes)),
            Some(Err(e)) => Err(e),
            None => Err(PromptError::Cancelled),
        }
    }
}

fn transport_over(
    spawner: Arc<dyn apex_shell::application::ports::spawner::ProcessSpawner>,
) -> (Arc<SshTransport>, Arc<AskpassChannel>) {
    let channel = Arc::new(AskpassChannel::bind().expect("bind the askpass socket"));
    let t = Arc::new(SshTransport::new(spawner, common::spec()));
    t.with_askpass(channel.clone(), true);
    (t, channel)
}

/// T036 — a credential the agent holds connects with no prompt at all.
///
/// The case that must stay silent. A prompt for a credential the agent already holds is
/// worse than an inconvenience: it teaches people to type passphrases at moments they did
/// not expect, which is the exact habit a phishing attempt needs.
#[tokio::test]
async fn an_agent_held_credential_connects_with_no_prompt() {
    let spawner = Arc::new(MockSpawner::new(""));
    let (t, _c) = transport_over(spawner.clone());
    let p = Prompter::answering("never used");

    assert_eq!(
        connect(&*t, &p, HOST).await,
        ConnectOutcome::Connected { assisted: false }
    );
    assert_eq!(p.asked(), 0, "the silent phase must not raise a prompt");
    t.shutdown();
}

/// T037 — FR-006. The assisted phase is entered only from `AuthenticationFailed`.
///
/// This is the single rule the connect sequence exists to enforce. An unreachable host or a
/// missing engine that asks for a passphrase is asking the user to solve a problem that
/// their passphrase cannot touch.
#[tokio::test]
async fn nothing_but_a_refused_credential_raises_a_passphrase_prompt() {
    for (exit, text, expected) in [
        (255, stderr::TIMED_OUT, FailureCondition::HostUnreachable),
        (255, stderr::REFUSED, FailureCondition::HostUnreachable),
        (127, stderr::NOT_FOUND, FailureCondition::EngineMissing),
        (
            255,
            stderr::HOST_KEY_CHANGED,
            FailureCondition::HostKeyChanged,
        ),
        (1, "engine panicked", FailureCondition::EngineCrashed),
    ] {
        let spawner = Arc::new(AssistedSpawner::new(exit, text));
        let (t, _c) = transport_over(spawner.clone());
        let p = Prompter::answering("never used");

        assert_eq!(
            connect(&*t, &p, HOST).await,
            ConnectOutcome::Failed(expected),
            "{text}"
        );
        assert_eq!(p.asked(), 0, "{expected:?} must not raise a prompt");
        assert_eq!(
            spawner.invocations().len(),
            1,
            "{expected:?} must not spawn an assisted attempt"
        );
    }
}

/// T038 — FR-008, SC-001. A passphrase appears in no log, no error and no panic payload.
///
/// Redaction is the kind of property that is true until someone adds one `{:?}`, so this
/// asserts on the artifacts a developer actually reaches for when something breaks.
#[tokio::test]
async fn a_passphrase_reaches_no_log_no_error_and_no_panic() {
    const SENTINEL: &str = "correct-horse-battery-staple-9c1f";
    let log = common::log_file();

    let spawner = Arc::new(AssistedSpawner::new(255, stderr::DENIED));
    let (t, channel) = transport_over(spawner.clone());
    let p = Prompter::answering(SENTINEL);
    let outcome = connect(&*t, &p, HOST).await;
    t.shutdown();

    // Everything a developer would print while debugging this flow.
    let printed = format!(
        "{outcome:?} {:?} {:?} {:?} {:?}",
        Secret::new(SENTINEL),
        Secret::new(SENTINEL).to_string(),
        spawner.invocations(),
        spawner.environments(),
    );
    assert!(
        !printed.contains(SENTINEL),
        "a passphrase reached a debug rendering: {printed}"
    );

    // And a panic carrying a Secret must not carry its contents either.
    let panicked = std::panic::catch_unwind(|| {
        let s = Secret::new(SENTINEL);
        panic!("something went wrong with {s:?}");
    });
    let payload = match panicked {
        Err(e) => e
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "<non-string payload>".into()),
        Ok(()) => unreachable!("the closure panics"),
    };
    assert!(
        !payload.contains(SENTINEL),
        "a passphrase reached a panic payload: {payload}"
    );

    // The log file, last. A marker goes through the same sink first, because "the
    // passphrase is not in the log" is satisfied just as well by a log that was never
    // written — and a redaction test that passes because nothing was logged is the kind of
    // green check this project has been caught by before.
    apex_shell::logging::info("askpass flow completed");
    let contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        contents.contains("askpass flow completed"),
        "the log sink is not live, so this test proves nothing about redaction: {}",
        log.display()
    );
    assert!(
        !contents.contains(SENTINEL),
        "a passphrase reached {}",
        log.display()
    );
    drop(channel);
}

/// T039 — a dismissed prompt ends the attempt cleanly rather than waiting forever.
///
/// Dismissing a dialog is an answer, not an absence of one. Retrying would reopen the
/// dialog the user just closed.
#[tokio::test]
async fn a_cancelled_prompt_ends_the_attempt() {
    let spawner = Arc::new(AssistedSpawner::new(255, stderr::DENIED));
    let (t, _c) = transport_over(spawner.clone());
    let p = Prompter::refusing(PromptError::Cancelled);

    let started = std::time::Instant::now();
    assert_eq!(connect(&*t, &p, HOST).await, ConnectOutcome::Cancelled);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "a dismissed prompt must not leave the attempt waiting"
    );
    assert_eq!(
        spawner.invocations().len(),
        1,
        "no assisted attempt may be spawned for a passphrase nobody gave"
    );
    assert_eq!(
        spawner.live_children(),
        0,
        "and nothing may be left running"
    );
}

/// T040 — FR-009. The third credential situation: the agent has no key and the passphrase
/// is wrong. A user told only "authentication failed" has nothing to do next; one offered
/// their other identities does.
#[tokio::test]
async fn exhausting_every_automatic_route_offers_the_identity_picker() {
    let spawner = Arc::new(
        AssistedSpawner::new(255, stderr::DENIED).also_failing_assisted(255, stderr::DENIED),
    );
    let (t, _c) = transport_over(spawner.clone());
    let p = Prompter::answering("the wrong passphrase");

    assert_eq!(
        connect(&*t, &p, HOST).await,
        ConnectOutcome::OfferIdentityPicker {
            after: FailureCondition::AuthenticationFailed
        }
    );
    assert_eq!(p.asked(), 1, "the user is asked once, not repeatedly");
    assert_eq!(
        spawner.invocations().len(),
        2,
        "both automatic routes must have been attempted before the picker is offered"
    );
}

/// The middle situation, and the one the story is named for: a passphrase-protected key
/// that connects once the user supplies the passphrase in the application's own window.
#[tokio::test]
async fn a_passphrase_supplied_in_the_application_completes_the_connection() {
    let spawner = Arc::new(AssistedSpawner::new(255, stderr::DENIED));
    let (t, _c) = transport_over(spawner.clone());
    let p = Prompter::answering("correct horse");

    assert_eq!(
        connect(&*t, &p, HOST).await,
        ConnectOutcome::Connected { assisted: true }
    );
    assert_eq!(p.asked(), 1);
    t.shutdown();
}

/// T041 — FR-007. No prompt reaches a tty in any phase.
///
/// The two settings that guarantee it sit in different places: `BatchMode=yes` in phase
/// one's argument list, and `SSH_ASKPASS_REQUIRE=force` in phase two's environment. Both
/// are asserted here because their absence is silent — OpenSSH would either block on a
/// terminal nobody is watching or skip the prompt entirely, and the second looks exactly
/// like an ordinary authentication failure.
#[tokio::test]
async fn no_phase_can_prompt_on_a_terminal() {
    let spawner = Arc::new(AssistedSpawner::new(255, stderr::DENIED));
    let (t, _c) = transport_over(spawner.clone());
    let p = Prompter::answering("correct horse");
    connect(&*t, &p, HOST).await;
    t.shutdown();

    let invocations = spawner.invocations();
    let environments = spawner.environments();
    assert_eq!(invocations.len(), 2, "both phases must have run");

    // Phase one: BatchMode makes ssh incapable of asking anyone anything.
    assert!(
        invocations[0].iter().any(|a| a == "BatchMode=yes"),
        "the silent phase must not be able to reach a terminal: {:?}",
        invocations[0]
    );
    assert!(
        !environments[0]
            .iter()
            .any(|(k, _)| k.starts_with("SSH_ASKPASS")),
        "and must not name a helper, which BatchMode disables anyway"
    );

    // Phase two: askpass, forced. Without force OpenSSH consults the helper only when it
    // finds no tty — which is a decision that varies by platform and by DISPLAY.
    let assisted: std::collections::HashMap<_, _> = environments[1].iter().cloned().collect();
    assert_eq!(
        assisted.get("SSH_ASKPASS_REQUIRE").map(String::as_str),
        Some("force"),
        "a prompt that is merely offered is a prompt that sometimes reaches a tty"
    );
    let helper = assisted
        .get("SSH_ASKPASS")
        .expect("the assisted phase must name a helper");
    assert!(
        std::path::Path::new(helper).is_absolute(),
        "ssh execs the helper with an unpredictable working directory: {helper}"
    );

    // And the passphrase was requested through the port, not by any other route.
    assert_eq!(p.asked(), 1, "the prompt must come from CredentialPrompt");
}

/// §3.3. Below OpenSSH 8.4 there is no way to force the helper, so the sequence offers the
/// picker rather than attempting a phase that would hang on some platforms and silently
/// skip the prompt on others.
#[tokio::test]
async fn an_openssh_too_old_to_force_a_prompt_offers_the_picker() {
    let spawner = Arc::new(AssistedSpawner::new(255, stderr::DENIED));
    let channel = Arc::new(AskpassChannel::bind().expect("bind"));
    let t = Arc::new(SshTransport::new(spawner.clone(), common::spec()));
    t.with_askpass(channel, false); // OpenSSH 8.2, say
    let p = Prompter::answering("never used");

    assert_eq!(
        connect(&*t, &p, HOST).await,
        ConnectOutcome::OfferIdentityPicker {
            after: FailureCondition::AuthenticationFailed
        }
    );
    assert_eq!(
        p.asked(),
        0,
        "a prompt that cannot be forced must not be raised"
    );
    assert_eq!(spawner.invocations().len(), 1);
}

// ---------------------------------------------------------------------------
// User Story 4: understand why a connection failed.
//
// classify() has its own unit tests. These make a different claim: that the wiring
// *delivers* the classification — through a spawn, through the stderr drain, through the
// reaper — and that the response attached to each condition is the one the data model says.
// A correct classifier reached by nothing is worth as much as no classifier.

use apex_shell::application::use_cases::connect::ConnectAttempt;
use apex_shell::application::use_cases::connect::{
    forget_command, ConfirmedByUser, HostKeyWarning,
};
use apex_shell::application::use_cases::supervise::{response_to, Response};

/// Drive the transport into a scripted ending and report what it concluded.
async fn condition_from(exit: i32, text: &str) -> FailureCondition {
    let spawner = Arc::new(AssistedSpawner::new(exit, text));
    let t = Arc::new(SshTransport::new(spawner, common::spec()));
    t.attempt(false)
        .await
        .expect_err("a scripted failure must not report success")
}

/// T060 — SC-007. Each condition names itself; none collapses into a generic failure.
#[tokio::test]
async fn every_condition_reaches_the_caller_as_itself() {
    let cases = [
        (255, stderr::DENIED, FailureCondition::AuthenticationFailed),
        (255, stderr::TIMED_OUT, FailureCondition::HostUnreachable),
        (255, stderr::REFUSED, FailureCondition::HostUnreachable),
        (
            255,
            "Timeout, server build-01 not responding.",
            FailureCondition::NetworkDropped,
        ),
        (
            255,
            stderr::HOST_KEY_CHANGED,
            FailureCondition::HostKeyChanged,
        ),
        (127, stderr::NOT_FOUND, FailureCondition::EngineMissing),
        (1, "engine panicked", FailureCondition::EngineCrashed),
    ];
    assert_eq!(cases.len(), 7, "all seven conditions must be covered");

    for (exit, text, expected) in cases {
        assert_eq!(
            condition_from(exit, text).await,
            expected,
            "exit {exit}: {text}"
        );
    }
}

/// T061 — SC-008. The invocation pins `LC_ALL=C`, which is what makes the English patterns
/// legitimate. If that pin were ever dropped, this is the text the transport would face —
/// and the honest answer is `Unknown`, never a different wrong condition that sends the
/// user down a remedy that cannot work.
#[tokio::test]
async fn a_localised_failure_is_never_misclassified() {
    let got = condition_from(255, stderr::DENIED_FR).await;
    assert_ne!(got, FailureCondition::HostUnreachable);
    assert_ne!(got, FailureCondition::HostKeyChanged);
    assert_eq!(got, FailureCondition::Unknown);

    // The exit code alone still carries the distinctions that do not need language.
    assert_eq!(
        condition_from(127, "fichier introuvable").await,
        FailureCondition::EngineMissing
    );
}

/// T062 — a changed host key refuses and never retries.
///
/// If this ever fails, it is a security defect and not a flaky test: retrying means
/// repeatedly offering credentials to a host that is not the one previously recorded.
#[tokio::test]
async fn a_changed_host_key_refuses_and_never_retries() {
    let condition = condition_from(255, stderr::HOST_KEY_CHANGED).await;
    assert_eq!(condition, FailureCondition::HostKeyChanged);
    assert!(
        !condition.should_retry(),
        "retrying a changed host key offers credentials to an unverified host"
    );
    for assisted in [true, false] {
        assert_eq!(
            response_to(condition, assisted),
            Response::StopAndReport,
            "a changed host key must stop, whatever else is available"
        );
    }
    assert!(!condition.may_prompt_for_credential());
}

/// T063 — FR-017. A missing engine is its own condition, handed to the feature that
/// installs it, and never reported as a connection problem the user would debug as one.
#[tokio::test]
async fn a_missing_engine_is_not_reported_as_a_connection_failure() {
    let condition = condition_from(127, stderr::NOT_FOUND).await;
    assert_eq!(condition, FailureCondition::EngineMissing);
    assert_ne!(condition, FailureCondition::HostUnreachable);
    assert_ne!(condition, FailureCondition::NetworkDropped);
    assert_eq!(
        response_to(condition, true),
        Response::HandToBootstrap,
        "the remedy is installing the engine, which is another feature's job"
    );
}

/// T064 — stderr beyond the retained bound degrades rather than exhausting memory, and the
/// decisive line survives when it is the last thing written, which is where OpenSSH puts it.
#[tokio::test]
async fn unbounded_stderr_degrades_rather_than_misclassifying() {
    let noise = "x".repeat(64 * 1024);
    assert_eq!(condition_from(255, &noise).await, FailureCondition::Unknown);

    let noisy_then_denied = format!("{noise}\n{}", stderr::DENIED);
    assert_eq!(
        condition_from(255, &noisy_then_denied).await,
        FailureCondition::AuthenticationFailed,
        "the last line is the decisive one and must survive the bound"
    );
}

/// T065 — FR-016, §3.9. Forgetting a changed host key is an explicit action.
///
/// The requirement is a prohibition, so the assertion is about what cannot be expressed:
/// `forget_known_host` takes a `ConfirmedByUser`, and that token has exactly one
/// constructor, named for the human act it represents. No supervisor, retry loop or
/// classifier can reach it, because none of them has anything to pass.
#[tokio::test]
async fn forgetting_a_host_key_cannot_happen_without_the_user() {
    let condition = condition_from(255, stderr::HOST_KEY_CHANGED).await;
    assert_eq!(condition, FailureCondition::HostKeyChanged);

    // The automatic path does not exist: the response to this condition is to stop and
    // report, and nothing in that path constructs a confirmation.
    assert_eq!(response_to(condition, true), Response::StopAndReport);

    let warning = HostKeyWarning::for_host("build-01.euw1", stderr::HOST_KEY_CHANGED);
    assert!(warning.guidance().contains("build-01.euw1"));
    assert!(
        !warning.guidance().to_lowercase().contains("connect anyway"),
        "the warning must not offer a way past itself"
    );

    // The remedy exists and is well formed — it is the reaching of it that is gated.
    assert_eq!(
        forget_command("build-01.euw1"),
        vec!["ssh-keygen", "-R", "build-01.euw1"]
    );
    let _only_the_interface_can_make_this = ConfirmedByUser::from_explicit_confirmation();
}
