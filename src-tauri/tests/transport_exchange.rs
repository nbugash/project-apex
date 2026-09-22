//! User Story 3: exchange requests without losing or crossing them.
//!
//! The failures this story is about are the quiet ones. A misdelivered reply does not throw
//! — it returns the wrong answer to a caller who has no way to know it is wrong. A leaked
//! registry entry does not fail a short test — it grows with session length. So the
//! assertions here are on identity and on size, not on "did it finish".

mod common;

use apex_shell::application::ports::transport::{Request, RequestTransport};
use apex_shell::domain::request::{RequestOutcome, MAX_FRAME_BYTES};
use common::connected;
use std::time::Duration;

fn answered(outcome: &RequestOutcome) -> bool {
    matches!(outcome, RequestOutcome::Answered(_))
}

/// The reply body carries the id it answers, so a misdelivery is detectable rather than
/// merely suspected.
fn id_in(outcome: &RequestOutcome) -> Option<String> {
    let RequestOutcome::Answered(body) = outcome else {
        return None;
    };
    let at = body.find("\"id\"")?;
    let rest = &body[at + 4..];
    let open = rest.find('"')?;
    let after = &rest[open + 1..];
    let close = after.find('"')?;
    Some(after[..close].to_string())
}

/// T047 — SC-004. Replies deliberately reversed, every outcome still reaching its own
/// request.
///
/// The mock holds ten replies and emits them backwards, so the first request asked is the
/// last answered. Positional correlation would pass a "did everything finish" test and fail
/// this one on every single request.
#[tokio::test]
async fn reordered_replies_each_reach_their_own_request() {
    let (t, _s) = connected("reorder=10");

    let mut work = Vec::new();
    for n in 0..10 {
        let (id, pending) = t.begin(Request::interactive(
            "engine/echo",
            format!(r#"{{"n":{n}}}"#),
        ));
        work.push((id, pending));
    }

    for (id, pending) in work {
        let outcome = pending.await;
        assert!(answered(&outcome), "{id} did not complete: {outcome:?}");
        assert_eq!(
            id_in(&outcome).as_deref(),
            Some(id.to_string().as_str()),
            "a reply reached the wrong request"
        );
    }
    t.shutdown();
}

/// T048 — FR-011. Registration precedes transmission.
///
/// There is no way to hold a reply in the exact microsecond after a write completes, so
/// this drives the race as hard as it goes: a mock that answers instantly, many requests at
/// once, and a deadline short enough that a dropped correlation fails in a second rather
/// than stalling the suite. If an entry were installed after the write, a reply landing in
/// that window would be discarded as unknown and its request would time out.
#[tokio::test]
async fn a_reply_arriving_immediately_is_still_matched() {
    let (t, _s) = connected("echo");

    let mut work = Vec::new();
    for n in 0..200 {
        let mut r = Request::interactive("engine/echo", format!(r#"{{"n":{n}}}"#));
        r.timeout = Some(Duration::from_secs(2));
        work.push(t.begin(r));
    }

    let mut timed_out = 0;
    for (id, pending) in work {
        match pending.await {
            RequestOutcome::TimedOut => timed_out += 1,
            other => assert!(answered(&other), "{id}: {other:?}"),
        }
    }
    assert_eq!(
        timed_out, 0,
        "a reply was discarded as unknown, which means an entry was installed after its write"
    );
    t.shutdown();
}

/// T049 — SC-005. A leak does not fail a short test, so this asserts on the registry's size
/// after a sustained mixed run rather than on the run completing.
#[tokio::test]
async fn nothing_accumulates_across_answered_timed_out_and_withdrawn_requests() {
    let (t, _s) = connected("drop=3"); // every third reply never arrives

    for n in 0..60 {
        let mut r = Request::interactive("engine/echo", format!(r#"{{"n":{n}}}"#));
        r.timeout = Some(Duration::from_millis(150));
        let (id, pending) = t.begin(r);
        if n % 7 == 0 {
            t.withdraw(&id);
        }
        let _ = pending.await;
    }

    assert_eq!(
        t.outstanding(),
        0,
        "the registry retained entries for finished requests"
    );
    t.shutdown();
}

/// T050 — SC-006. A request nobody answers gives up, and stops occupying the connection.
#[tokio::test]
async fn an_unanswered_request_times_out_within_its_limit() {
    let (t, _s) = connected("drop=1"); // nothing is ever answered

    let mut r = Request::interactive("engine/echo", "{}");
    r.timeout = Some(Duration::from_millis(200));

    let started = std::time::Instant::now();
    let outcome = t.send(r).await;
    let elapsed = started.elapsed();

    assert_eq!(outcome, RequestOutcome::TimedOut);
    assert!(
        elapsed < Duration::from_secs(2),
        "a request must give up at its limit, not at the connection's: {elapsed:?}"
    );
    assert_eq!(
        t.outstanding(),
        0,
        "a timed-out request must stop occupying the connection"
    );

    t.shutdown();
}

/// T051 — FR-014. A withdrawn request resolves rather than being silently dropped, and
/// losing the race against a reply is harmless.
#[tokio::test]
async fn a_withdrawn_request_resolves_and_withdrawing_twice_is_harmless() {
    let (t, _s) = connected("drop=1"); // never answered, so the withdrawal always wins

    let mut r = Request::interactive("engine/index", r#"{"path":"/repo"}"#);
    r.timeout = Some(Duration::from_secs(5));
    let (id, pending) = t.begin(r);

    t.withdraw(&id);
    assert_eq!(
        pending.await,
        RequestOutcome::Withdrawn,
        "a withdrawn request must resolve, not vanish"
    );

    // Withdrawing again, and withdrawing something that never existed. The caller races the
    // reply by nature, so losing that race must not be an error.
    t.withdraw(&id);
    t.withdraw(&apex_shell::domain::request::RequestId(
        "never-existed".into(),
    ));
    assert_eq!(t.outstanding(), 0);
    t.shutdown();
}

/// T052 — SC-013, FR-021. Interactive traffic goes ahead of queued background work.
///
/// The queue has to be saturated for the ordering to be observable at all: with a fast
/// writer and an empty pipe, everything is written the moment it is pushed and there is
/// nothing to overtake. So the background requests carry a payload large enough to fill the
/// pipe and block the writer — which is the situation FR-021 exists for, an indexing run
/// that would otherwise delay every keystroke behind it.
///
/// The measurement is arrival order, not elapsed time. The mock answers in receipt order,
/// so the order replies come back *is* the order frames were written. If interactive
/// traffic queued behind background work, the urgent request would arrive last of 41.
#[tokio::test]
async fn interactive_traffic_overtakes_a_saturated_background_queue() {
    use std::sync::{Arc, Mutex};

    let (t, _s) = connected("delay=2");
    let arrivals: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Saturate first. 40 × 16 KiB is far beyond a pipe buffer, so the writer blocks with
    // most of this still queued.
    let bulk = "x".repeat(16 * 1024);
    let mut tasks = Vec::new();
    for n in 0..40 {
        let mut r = Request::background("engine/index", format!(r#"{{"n":{n},"blob":"{bulk}"}}"#));
        r.timeout = Some(Duration::from_secs(30));
        let (id, pending) = t.begin(r);
        let arrivals = arrivals.clone();
        tasks.push(tokio::spawn(async move {
            let outcome = pending.await;
            arrivals.lock().expect("arrivals").push(id.to_string());
            outcome
        }));
    }

    // The keystroke that arrives while the indexer is mid-run.
    let mut urgent = Request::interactive("textDocument/completion", r#"{"line":1}"#);
    urgent.timeout = Some(Duration::from_secs(30));
    let (urgent_id, urgent_pending) = t.begin(urgent);
    let urgent_outcome = {
        let arrivals = arrivals.clone();
        let id = urgent_id.clone();
        tokio::spawn(async move {
            let outcome = urgent_pending.await;
            arrivals.lock().expect("arrivals").push(id.to_string());
            outcome
        })
    };

    let outcome = urgent_outcome.await.expect("the urgent task");
    assert!(answered(&outcome), "{urgent_id}: {outcome:?}");
    for task in tasks {
        let _ = task.await;
    }

    let order = arrivals.lock().expect("arrivals").clone();
    let position = order
        .iter()
        .position(|id| id == &urgent_id.to_string())
        .expect("the urgent request must have arrived");
    assert_eq!(order.len(), 41, "every request must have completed");
    assert!(
        position < 20,
        "the interactive request arrived {position} of {} — it queued behind background work",
        order.len()
    );
    // Not "first", by design: a frame already being written completes before anything
    // overtakes it, because Content-Length has promised exactly that many bytes follow and
    // interrupting it corrupts the stream. The 1 MiB cap is what bounds that delay.
    t.shutdown();
}

/// T053 — a payload over the cap is refused before transmission, and the stream stays
/// aligned: a normal request afterwards still succeeds.
///
/// Alignment is the point. A refused frame that had been half-written would leave the next
/// `Content-Length` header being read as body, and every request after it would fail for a
/// reason that has nothing to do with the request.
#[tokio::test]
async fn an_oversized_payload_is_refused_without_breaking_the_stream() {
    let (t, _s) = connected("echo");

    let huge = "x".repeat(MAX_FRAME_BYTES + 1);
    let refused = t
        .send(Request::interactive(
            "engine/echo",
            format!(r#"{{"blob":"{huge}"}}"#),
        ))
        .await;
    match &refused {
        RequestOutcome::Failed { code, message } => {
            assert!(
                message.contains("frame limit"),
                "the caller must learn the cap: {message}"
            );
            assert_ne!(*code, 0);
        }
        other => panic!("an oversized payload must be refused, got {other:?}"),
    }

    let after = t
        .send(Request::interactive("engine/echo", r#"{"n":1}"#))
        .await;
    assert!(
        answered(&after),
        "the stream did not stay aligned after a refusal: {after:?}"
    );
    assert_eq!(t.outstanding(), 0);
    t.shutdown();
}

// ---------------------------------------------------------------------------
// User Story 5: verify the transport without a remote machine.

/// T069 — the feature map's link profile: 250 ms round trip, 5% loss. Nothing lost that was
/// not dropped, nothing crossed.
#[tokio::test]
async fn a_slow_lossy_link_loses_nothing_it_was_not_given_to_lose() {
    let (t, _s) = connected("delay=250,drop=20");

    let mut work = Vec::new();
    for n in 0..20 {
        let mut r = Request::interactive("engine/echo", format!(r#"{{"n":{n}}}"#));
        // Comfortably past the 250 ms round trip, so a timeout means a dropped reply
        // rather than an impatient deadline.
        r.timeout = Some(Duration::from_secs(10));
        work.push(t.begin(r));
    }

    let (mut answered_count, mut dropped) = (0, 0);
    for (id, pending) in work {
        match pending.await {
            outcome @ RequestOutcome::Answered(_) => {
                assert_eq!(
                    id_in(&outcome).as_deref(),
                    Some(id.to_string().as_str()),
                    "latency must not cross replies"
                );
                answered_count += 1;
            }
            RequestOutcome::TimedOut => dropped += 1,
            other => panic!("{id}: unexpected {other:?}"),
        }
    }
    assert_eq!(answered_count + dropped, 20);
    assert!(
        answered_count >= 18,
        "5% loss should cost about one reply in twenty, not {dropped}"
    );
    assert_eq!(t.outstanding(), 0);
    t.shutdown();
}

fn percentile(mut samples: Vec<Duration>, p: f64) -> Duration {
    samples.sort();
    let idx = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[idx]
}

/// T070 — SC-011. The transport's **added** overhead, at the 99th percentile.
///
/// Measuring wall clock would pass regardless of what the transport does, because the
/// harness's own delay dominates it: a transport that took 200 ms per request would still
/// look fine next to a 250 ms round trip. So the round trip is subtracted, and the first
/// measurement removes it entirely — with the mock answering immediately, everything
/// measured *is* overhead.
#[tokio::test]
async fn the_transport_adds_little_to_a_round_trip() {
    const BUDGET: Duration = Duration::from_millis(15);

    // Pure overhead: no simulated round trip at all.
    let (t, _s) = connected("echo");
    let mut samples = Vec::new();
    for n in 0..200 {
        let started = std::time::Instant::now();
        let outcome = t
            .send(Request::interactive(
                "engine/echo",
                format!(r#"{{"n":{n}}}"#),
            ))
            .await;
        samples.push(started.elapsed());
        assert!(answered(&outcome));
    }
    let p99 = percentile(samples.clone(), 0.99);
    // Printed, not only asserted. A budget that is only ever compared against tells nobody
    // how much headroom is left, and headroom is what says whether the next feature's work
    // can be afforded.
    eprintln!(
        "SC-011 pure overhead: p50 {:?}, p99 {p99:?} (budget {BUDGET:?})",
        percentile(samples.clone(), 0.5)
    );
    assert!(
        p99 < BUDGET,
        "added overhead at p99 was {p99:?}, over the {BUDGET:?} budget (median {:?})",
        percentile(samples, 0.5)
    );
    t.shutdown();

    // And with a round trip in the way, subtracted. This is the form SC-011 states, and it
    // catches overhead that only appears once replies are not instantaneous.
    const SIMULATED: Duration = Duration::from_millis(100);
    let (t, _s) = connected("delay=100");
    let mut added = Vec::new();
    for n in 0..30 {
        let started = std::time::Instant::now();
        let outcome = t
            .send(Request::interactive(
                "engine/echo",
                format!(r#"{{"n":{n}}}"#),
            ))
            .await;
        added.push(started.elapsed().saturating_sub(SIMULATED));
        assert!(answered(&outcome));
    }
    let p99 = percentile(added.clone(), 0.99);
    eprintln!(
        "SC-011 beyond a {SIMULATED:?} round trip: p50 {:?}, p99 {p99:?} (budget {BUDGET:?})",
        percentile(added, 0.5)
    );
    assert!(
        p99 < BUDGET,
        "overhead beyond the simulated round trip was {p99:?} at p99, over {BUDGET:?}"
    );
    t.shutdown();
}

/// T071 — SC-010. No test in this suite opens a network socket or needs a remote host.
///
/// Asserted rather than asserted-about: the claim "runs with the network off" is the kind
/// that stays true until someone adds a convenience, and a comment does not notice. This
/// walks the process's own descriptors and cross-references the kernel's TCP tables, so a
/// socket opened by anything in this binary shows up.
#[tokio::test]
async fn the_suite_opens_no_network_sockets() {
    let (t, _s) = connected("echo");
    let _ = t.send(Request::interactive("engine/echo", "{}")).await;

    // Prove the detector works before trusting its silence. A check that cannot find a
    // socket reports "no sockets" exactly as convincingly as one that finds none, and this
    // suite has already produced one test that passed for that reason.
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback socket");
        assert!(
            !open_tcp_sockets().is_empty(),
            "the detector cannot see a socket that is demonstrably open, so its silence means nothing"
        );
        drop(listener);
    }

    let open = open_tcp_sockets();
    assert!(
        open.is_empty(),
        "this suite must need no network; found TCP socket inodes {open:?}"
    );
    t.shutdown();
}

/// Inodes of TCP sockets held by this process, if the platform can tell us.
#[cfg(target_os = "linux")]
fn open_tcp_sockets() -> Vec<u64> {
    // The kernel's tables list every socket in the namespace; /proc/self/fd says which of
    // them are ours. The intersection is what this process opened.
    let mut tcp_inodes = std::collections::HashSet::new();
    for table in ["/proc/self/net/tcp", "/proc/self/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(table) else {
            continue;
        };
        for line in text.lines().skip(1) {
            if let Some(inode) = line.split_whitespace().nth(9) {
                if let Ok(n) = inode.parse::<u64>() {
                    tcp_inodes.insert(n);
                }
            }
        }
    }

    let mut ours = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return ours;
    };
    for entry in entries.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let target = target.to_string_lossy().to_string();
        // "socket:[12345]" — the unix sockets the askpass channel uses land here too, which
        // is why the inode is checked against the TCP tables rather than assumed.
        if let Some(rest) = target.strip_prefix("socket:[") {
            if let Ok(inode) = rest.trim_end_matches(']').parse::<u64>() {
                if tcp_inodes.contains(&inode) {
                    ours.push(inode);
                }
            }
        }
    }
    ours
}

/// Other platforms have no equivalent cheap check. Returning nothing makes the assertion
/// vacuous there, which is stated rather than hidden: CI runs Linux, where it is real.
#[cfg(not(target_os = "linux"))]
fn open_tcp_sockets() -> Vec<u64> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Hostile input from the far side (Principle VI, contracts/framing.md).
//
// The assertion that matters is not "the bad frame was rejected" — a reader that silently
// desynchronised would pass that. It is that a *normal request afterwards* is still
// answered, which is only true if the stream stayed aligned.

/// A reply whose body is not JSON. The transport cannot correlate it, so the request it was
/// meant for times out — and everything after it must still work.
#[tokio::test]
async fn a_malformed_reply_costs_one_request_and_not_the_stream() {
    let (t, _s) = connected("malformed=1"); // only the first reply is junk

    let mut first = Request::interactive("engine/echo", r#"{"n":1}"#);
    first.timeout = Some(Duration::from_millis(300));
    assert_eq!(
        t.send(first).await,
        RequestOutcome::TimedOut,
        "a reply that cannot be correlated cannot resolve its request"
    );

    let after = t
        .send(Request::interactive("engine/echo", r#"{"n":2}"#))
        .await;
    assert!(
        answered(&after),
        "the stream did not stay aligned after a malformed frame: {after:?}"
    );
    assert_eq!(t.outstanding(), 0);
    t.shutdown();
}

/// A header declaring more than the cap, with no body behind it. The transport must refuse
/// it without allocating and without consuming the bytes that follow as if they were body.
#[tokio::test]
async fn an_oversized_declared_length_is_refused_and_the_stream_survives() {
    let (t, _s) = connected("oversized=1");

    let mut first = Request::interactive("engine/echo", r#"{"n":1}"#);
    first.timeout = Some(Duration::from_millis(300));
    assert_eq!(t.send(first).await, RequestOutcome::TimedOut);

    let after = t
        .send(Request::interactive("engine/echo", r#"{"n":2}"#))
        .await;
    assert!(
        answered(&after),
        "a refused oversized frame must not consume what follows it: {after:?}"
    );
    t.shutdown();
}

/// The stream ends halfway through a frame. There is no aligned position to recover to, so
/// the connection is over — and the requirement is that everything in flight resolves
/// rather than waiting for the rest of a frame that will never arrive.
#[tokio::test]
async fn a_truncated_frame_ends_the_connection_without_leaving_anyone_waiting() {
    let (t, _s) = connected("close-mid-frame");

    let mut r = Request::interactive("engine/echo", r#"{"n":1}"#);
    r.timeout = Some(Duration::from_secs(5));
    let outcome = t.send(r).await;

    assert_eq!(
        outcome,
        RequestOutcome::ConnectionLost,
        "a half-written frame must end the connection, not hang the request"
    );
    assert_eq!(t.outstanding(), 0);
    t.shutdown();
}
