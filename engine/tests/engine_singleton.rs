//! A-ENGINELIFE rule 1: one engine per host, and a stale socket is not one.
//!
//! The distinction this file exists for is between a socket **something is listening on** and a
//! socket **an engine left behind when it died**. A bind failure alone cannot tell them apart, and
//! guessing either way is a real fault: unlink on every failure and a second engine starts beside
//! a serving one, each holding half the tasks and a client reaching whichever it bound; never
//! unlink and a single crash makes the host permanently unusable, because every later invocation
//! finds a file in the way and becomes a proxy to nothing.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

use apex_engine::adapters::outbound::engine_socket::{claim, release, Claim};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

fn socket_in(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("apex-engine").join("engine.sock")
}

#[test]
fn the_first_process_is_the_engine_and_the_second_is_a_proxy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);

    let first = claim(&path).expect("first claim");
    assert!(
        matches!(first, Claim::Bound(_)),
        "the first process should have become the engine"
    );

    let second = claim(&path).expect("second claim");
    assert!(
        matches!(second, Claim::Proxy(_)),
        "the second process should have found the first, not become a second engine"
    );
}

#[test]
fn a_socket_left_by_a_dead_engine_is_recognised_and_replaced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);

    // An engine that died: the file is there and nothing is listening. Dropping the listener
    // closes it without unlinking, which is exactly what a killed process leaves behind.
    let listener = claim(&path).expect("claim");
    let Claim::Bound(bound) = listener else {
        panic!("expected to bind");
    };
    drop(bound);
    assert!(path.exists(), "the test needs the file to remain");

    let after = claim(&path).expect("claim after death");
    assert!(
        matches!(after, Claim::Bound(_)),
        "a stale socket should be replaced, not proxied to"
    );
}

#[test]
fn a_live_socket_is_never_unlinked() {
    // The other side of the same decision, and the one whose failure is worse. Unlinking here
    // would leave two engines on one host: the first still serving its clients over a socket with
    // no name, the second bound to the name, and each holding tasks the other cannot reach.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);

    let Claim::Bound(_listener) = claim(&path).expect("claim") else {
        panic!("expected to bind");
    };
    let before = std::fs::metadata(&path).expect("metadata");

    let second = claim(&path).expect("second claim");
    assert!(matches!(second, Claim::Proxy(_)));

    let after = std::fs::metadata(&path).expect("metadata after");
    // Same inode: the file was not removed and recreated.
    use std::os::unix::fs::MetadataExt;
    assert_eq!(
        before.ino(),
        after.ino(),
        "the live socket was replaced by a second engine's"
    );
}

#[test]
fn the_socket_and_its_directory_are_private_to_this_user() {
    // Read from the filesystem, not from the constants that set them. The socket is a full
    // control channel -- anything that can connect runs commands as this user -- and what
    // enforces that is the mode the kernel sees, not the number in the source.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);
    let Claim::Bound(_listener) = claim(&path).expect("claim") else {
        panic!("expected to bind");
    };

    let socket_mode = std::fs::metadata(&path)
        .expect("socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        socket_mode, 0o600,
        "the socket is readable or writable by somebody other than its owner"
    );

    let parent = path.parent().expect("a parent");
    let dir_mode = std::fs::metadata(parent)
        .expect("directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        dir_mode, 0o700,
        "the socket's directory is reachable by somebody other than its owner"
    );
}

#[test]
fn a_directory_left_open_by_an_earlier_version_is_tightened() {
    // `DirBuilder`'s mode does not apply to a directory that already exists, so a directory left
    // 0755 by an earlier build would stay a control channel anyone on the host could reach. The
    // mode is set either way, and this is what says so.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);
    let parent = path.parent().expect("a parent");
    std::fs::create_dir_all(parent).expect("mkdir");
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let Claim::Bound(_listener) = claim(&path).expect("claim") else {
        panic!("expected to bind");
    };
    let mode = std::fs::metadata(parent)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "an existing directory was left world-readable");
}

#[test]
fn releasing_removes_the_socket_so_the_next_invocation_does_not_pay_for_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);
    let Claim::Bound(listener) = claim(&path).expect("claim") else {
        panic!("expected to bind");
    };
    drop(listener);
    release(&path);
    assert!(!path.exists(), "release left the socket behind");
}

#[test]
fn a_path_that_cannot_be_bound_is_an_error_rather_than_a_silent_proxy() {
    // A failure that is neither "in use" nor stale -- a directory where the socket should be --
    // must be reported. Treating every failure as "somebody else is the engine" would make a
    // misconfigured host look like a busy one, and the developer would be told to look in the
    // wrong place.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);
    std::fs::create_dir_all(&path).expect("mkdir where the socket goes");
    assert!(claim(&path).is_err(), "a directory in the way was ignored");
}

#[test]
fn an_unrelated_listener_on_the_path_makes_this_a_proxy() {
    // The positive case for "something is listening": a socket bound by anything at all is a
    // reason not to unlink. This process cannot know it is an engine, only that it is not alone.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = socket_in(&dir);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    let _other = UnixListener::bind(&path).expect("bind");

    assert!(matches!(claim(&path).expect("claim"), Claim::Proxy(_)));
}
