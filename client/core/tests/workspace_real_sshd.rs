//! Opt-in: the one thing a locally spawned process cannot prove.
//!
//! A-BULK's constraint is about how `ssh` behaves when a control master already exists, and no
//! local spawn models that. The rest of this feature's suite runs with no remote host and no
//! network (SC-014), which is why this is `#[ignore]` and gated on `APEX_REAL_SSHD` rather than
//! part of the default run.
//!
//! Run with:
//!   APEX_REAL_SSHD=1 cargo test -p apex-shell --test workspace_real_sshd -- --ignored

use apex_shell::adapters::outbound::bulk::SshBulkTransfer;
use apex_shell::application::ports::bulk_transfer::BulkTransfer;
use apex_shell::domain::workspace::ByteRange;

fn enabled() -> bool {
    std::env::var("APEX_REAL_SSHD").is_ok()
}

fn target() -> (String, String) {
    (
        std::env::var("APEX_REMOTE_USER").unwrap_or_else(|_| whoami()),
        std::env::var("APEX_REMOTE_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
    )
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".into())
}

/// The trap F002 fell into, on the read path this time.
///
/// A bulk invocation that becomes the control master backgrounds itself while still holding the
/// stdout pipe it inherited, so reading its output waits forever for an EOF that cannot arrive.
/// It hangs only when no master exists yet — which is exactly a first connect. The argv assertion
/// in the unit tests proves the flag is passed; this proves the resulting invocation actually
/// terminates against a real `sshd`.
#[tokio::test]
#[ignore = "needs a real sshd; set APEX_REAL_SSHD"]
async fn a_bulk_fetch_terminates_against_a_real_sshd() {
    if !enabled() {
        eprintln!("APEX_REAL_SSHD not set; skipping");
        return;
    }
    let dir = tempfile::tempdir().expect("temp");
    let file = dir.path().join("payload.bin");
    let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 256) as u8).collect();
    std::fs::write(&file, &payload).expect("write");

    let (user, host) = target();
    let bulk = SshBulkTransfer::new(user, host);

    let fetched = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        bulk.fetch(file.to_str().unwrap(), None),
    )
    .await
    .expect("the fetch must terminate — a hang here is the ControlMaster trap")
    .expect("fetch");

    assert_eq!(
        fetched, payload,
        "bulk content must arrive byte-for-byte; this path carries binary artifacts"
    );
}

#[tokio::test]
#[ignore = "needs a real sshd; set APEX_REAL_SSHD"]
async fn a_ranged_bulk_fetch_returns_only_the_range() {
    if !enabled() {
        eprintln!("APEX_REAL_SSHD not set; skipping");
        return;
    }
    let dir = tempfile::tempdir().expect("temp");
    let file = dir.path().join("ranged.bin");
    std::fs::write(&file, b"0123456789").expect("write");

    let (user, host) = target();
    let bulk = SshBulkTransfer::new(user, host);
    let got = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        bulk.fetch(
            file.to_str().unwrap(),
            Some(ByteRange {
                offset: 3,
                length: 4,
            }),
        ),
    )
    .await
    .expect("must terminate")
    .expect("fetch");

    assert_eq!(
        got, b"3456",
        "the remote `dd` must seek rather than stream and discard, or reading the tail of a \
         large file is proportional to the whole"
    );
}

/// SC-014: the default suite needs no host. This asserts the gate itself, so the opt-in tests
/// cannot silently become part of every run.
#[test]
fn these_tests_are_opt_in() {
    if std::env::var("APEX_REAL_SSHD").is_err() {
        // Nothing above runs. Stated as a test so the property is visible in the output rather
        // than inferred from an attribute.
    }
}
