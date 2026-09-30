//! The reconnection loop §11.5 specifies, against a scripted fake (FR-018a).
//!
//! **The policy is what is under test**, not the transport: how long to wait, what to publish while
//! waiting, and when to stop. A real transport would make every case here slow and dependent on a
//! machine's `ssh`, which is why the loop takes a port.

use apex_shell::application::ports::connection::Reconnectable;
use apex_shell::application::use_cases::reconnect::{jitter, Reconnect, Reconnected};
use apex_shell::application::use_cases::supervise::{BASE_DELAY, MAX_DELAY};
use apex_shell::domain::failure::FailureCondition;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A connection whose attempts the test scripts, recording everything the loop does to it.
#[derive(Default)]
struct Scripted {
    /// Each attempt takes the next answer. `Err(condition)` fails with that classification;
    /// `Err(None)` fails with none, as a signalled child does.
    answers: Mutex<VecDeque<Result<(), Option<FailureCondition>>>>,
    last: Mutex<Option<FailureCondition>>,
    /// In order: `retrying(attempt, secs)` and `attempt`.
    log: Mutex<Vec<String>>,
}

impl Scripted {
    fn answering(answers: Vec<Result<(), Option<FailureCondition>>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            ..Self::default()
        }
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
    fn attempts(&self) -> usize {
        self.log().iter().filter(|l| *l == "attempt").count()
    }
}

impl Reconnectable for Scripted {
    fn reconnect(&self) -> Result<(), String> {
        self.log.lock().unwrap().push("attempt".into());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("the loop made an attempt the test did not script");
        match answer {
            Ok(()) => Ok(()),
            Err(condition) => {
                *self.last.lock().unwrap() = condition;
                Err("failed".into())
            }
        }
    }
    fn last_failure(&self) -> Option<FailureCondition> {
        *self.last.lock().unwrap()
    }
    fn report_retrying(&self, attempt: u32, next_in_secs: u64) {
        self.log
            .lock()
            .unwrap()
            .push(format!("retrying({attempt}, {next_in_secs})"));
    }
}

/// Records every wait instead of sleeping.
fn recorder() -> (Arc<Mutex<Vec<Duration>>>, impl Fn(Duration)) {
    let waits = Arc::new(Mutex::new(Vec::new()));
    let w = waits.clone();
    (waits, move |d| w.lock().unwrap().push(d))
}

/// The top of each window, so a test can assert growth without a random draw.
fn full_window() -> impl FnMut() -> f64 {
    || 1.0
}

#[test]
fn a_transient_failure_is_retried_until_the_connection_returns() {
    let target = Scripted::answering(vec![
        Err(Some(FailureCondition::NetworkDropped)),
        Err(Some(FailureCondition::HostUnreachable)),
        Ok(()),
    ]);
    let (waits, sleep) = recorder();

    let outcome = Reconnect::new()
        .run(&target, &sleep, &mut full_window())
        .expect("no loop was already running");

    assert_eq!(outcome, Reconnected::Connected { attempts: 3 });
    assert_eq!(target.attempts(), 3, "retried twice, then connected");
    let waits = waits.lock().unwrap().clone();
    assert_eq!(waits.len(), 3, "one wait before each attempt");
    // Growing, which is the point of a backoff: a fixed interval either hammers a host that is down
    // or reconnects slowly, and cannot be both.
    assert!(waits.windows(2).all(|w| w[1] > w[0]), "{waits:?}");
    assert_eq!(waits[0], BASE_DELAY, "the first wait is the base delay");
}

#[test]
fn a_signalled_child_is_worth_another_try() {
    // A child killed by a signal has no exit code to classify, and that is what an engine that was
    // stopped looks like. Treating "cannot tell" as "give up" would leave the client offline after
    // the most ordinary outage there is.
    let target = Scripted::answering(vec![Err(None), Ok(())]);
    let (_waits, sleep) = recorder();
    let outcome = Reconnect::new().run(&target, &sleep, &mut full_window());
    assert_eq!(outcome, Some(Reconnected::Connected { attempts: 2 }));
}

#[test]
fn retrying_is_published_before_each_wait() {
    // Before, not after: the status bar should say "trying again in N seconds" for the whole of the
    // wait rather than once it has passed.
    let target = Scripted::answering(vec![Err(Some(FailureCondition::NetworkDropped)), Ok(())]);
    // The wait goes into the same log as everything else. Recording it separately -- as the first
    // version of this test did -- left the log unable to say where the wait fell, and moving the
    // report to after the wait passed. The mutation caught it.
    let sleep = |_d: Duration| target.log.lock().unwrap().push("wait".into());
    Reconnect::new().run(&target, &sleep, &mut full_window());

    assert_eq!(
        target.log(),
        vec![
            "retrying(1, 1)",
            "wait",
            "attempt",
            "retrying(2, 2)",
            "wait",
            "attempt"
        ],
        "each wait is announced before it starts"
    );
}

#[test]
fn a_short_wait_is_never_reported_as_zero_seconds() {
    // A draw near the bottom of the window gives a wait under a second, and "trying again in 0
    // seconds" reads as a hang.
    let target = Scripted::answering(vec![Ok(())]);
    let (_waits, sleep) = recorder();
    Reconnect::new().run(&target, &sleep, &mut || 0.3);
    assert_eq!(target.log()[0], "retrying(1, 1)");
}

#[test]
fn a_changed_host_key_stops_the_loop_without_retrying() {
    // Never retried (`FailureCondition::should_retry`). A loop that kept trying would hide from the
    // developer the one failure that most needs them.
    let target = Scripted::answering(vec![Err(Some(FailureCondition::HostKeyChanged))]);
    let (_waits, sleep) = recorder();
    let outcome = Reconnect::new().run(&target, &sleep, &mut full_window());
    assert_eq!(
        outcome,
        Some(Reconnected::GaveUp(FailureCondition::HostKeyChanged))
    );
    assert_eq!(target.attempts(), 1);
}

#[test]
fn a_refused_credential_stops_rather_than_prompting_from_a_loop() {
    // A loop cannot put a passphrase prompt in front of anyone, so this is a report.
    let target = Scripted::answering(vec![Err(Some(FailureCondition::AuthenticationFailed))]);
    let (_waits, sleep) = recorder();
    let outcome = Reconnect::new().run(&target, &sleep, &mut full_window());
    assert_eq!(
        outcome,
        Some(Reconnected::GaveUp(FailureCondition::AuthenticationFailed))
    );
}

#[test]
fn the_wait_stops_growing_at_the_ceiling() {
    // A laptop shut for an hour should reconnect within half a minute of waking, not back off to
    // hours -- which is what uncapped doubling produces.
    let mut answers: Vec<Result<(), Option<FailureCondition>>> = (0..12)
        .map(|_| Err(Some(FailureCondition::NetworkDropped)))
        .collect();
    answers.push(Ok(()));
    let target = Scripted::answering(answers);
    let (waits, sleep) = recorder();
    Reconnect::new().run(&target, &sleep, &mut full_window());
    let waits = waits.lock().unwrap().clone();
    assert_eq!(*waits.last().unwrap(), MAX_DELAY);
    assert!(waits.iter().all(|w| *w <= MAX_DELAY));
}

#[test]
fn each_loss_starts_its_schedule_from_the_base_delay() {
    // A fresh schedule per loss. The previous outage's backoff says nothing about this one.
    let reconnect = Reconnect::new();
    let first = Scripted::answering(vec![
        Err(Some(FailureCondition::NetworkDropped)),
        Err(Some(FailureCondition::NetworkDropped)),
        Ok(()),
    ]);
    let (_w, sleep) = recorder();
    reconnect.run(&first, &sleep, &mut full_window());

    let second = Scripted::answering(vec![Ok(())]);
    let (waits, sleep) = recorder();
    reconnect.run(&second, &sleep, &mut full_window());
    assert_eq!(waits.lock().unwrap()[0], BASE_DELAY);
}

#[test]
fn a_second_loop_does_not_start_while_one_is_running() {
    // A failed attempt publishes `Disconnected` again, which is the state that starts a loop.
    // Without a guard each failure would start another loop and the backoff would mean nothing.
    let reconnect = Arc::new(Reconnect::new());
    let started = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));

    let target = Arc::new(Scripted::answering(vec![Ok(())]));
    let first = {
        let (reconnect, target, started, release) = (
            reconnect.clone(),
            target.clone(),
            started.clone(),
            release.clone(),
        );
        std::thread::spawn(move || {
            // The sleep is where the first loop is parked while the second one is tried.
            let sleep = move |_d: Duration| {
                started.wait();
                release.wait();
            };
            reconnect.run(target.as_ref(), &sleep, &mut || 1.0)
        })
    };
    started.wait();
    let second = reconnect.run(&Scripted::default(), &|_| {}, &mut || 1.0);
    release.wait();

    assert_eq!(second, None, "the second loop must decline");
    assert_eq!(
        first.join().expect("first loop"),
        Some(Reconnected::Connected { attempts: 1 })
    );
}

#[test]
fn jitter_draws_across_the_unit_interval() {
    // Not a statistical test: it catches a draw stuck at one value, which is the failure that would
    // make every client of a restarted host retry in lockstep.
    let draws: Vec<f64> = (0..64).map(|_| jitter()).collect();
    assert!(draws.iter().all(|d| (0.0..1.0).contains(d)), "{draws:?}");
    let distinct = draws
        .iter()
        .map(|d| (d * 1000.0) as u32)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(distinct.len() > 32, "the draw must vary: {distinct:?}");
}
