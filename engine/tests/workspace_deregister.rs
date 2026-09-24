//! `workspace/close`'s half of the registry (T023, FR-024, A-WSCLOSE).
//!
//! The port offered `register` and `resolve` only until F010, so closing a workspace could stop
//! its tasks and then leave the engine holding a canonicalised root for a workspace the client
//! had finished with, with no way for the client to say otherwise.

mod common;

use apex_engine::application::ports::roots::{RootError, WorkspaceRoots};
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use common::FakeFileSystem;
use std::sync::Arc;

fn roots_with(dirs: &[&str]) -> InMemoryRoots {
    let fs = Arc::new(FakeFileSystem::new());
    for d in dirs {
        fs.dir(d);
    }
    InMemoryRoots::new(fs)
}

#[test]
fn a_deregistered_workspace_stops_resolving() {
    let roots = roots_with(&["/work/alpha"]);
    roots.register("ws-1", "/work/alpha").expect("register");
    assert!(roots.resolve("ws-1").is_ok());

    roots.deregister("ws-1").expect("deregister");

    // `NotRegistered` and not `Gone`: the directory is fine, the client said it was done.
    assert_eq!(roots.resolve("ws-1"), Err(RootError::NotRegistered));
}

#[test]
fn a_second_deregister_is_refused_rather_than_succeeding_silently() {
    let roots = roots_with(&["/work/alpha"]);
    roots.register("ws-1", "/work/alpha").expect("register");
    roots.deregister("ws-1").expect("first deregister");

    // This is what lets a second `workspace/close` answer -32001. A silent success would make
    // a client that has lost track of its own state look correct (A-WSCLOSE).
    assert_eq!(roots.deregister("ws-1"), Err(RootError::NotRegistered));
}

#[test]
fn deregistering_one_workspace_leaves_another_alone() {
    let roots = roots_with(&["/work/alpha", "/work/beta"]);
    roots.register("ws-1", "/work/alpha").expect("register");
    roots.register("ws-2", "/work/beta").expect("register");

    roots.deregister("ws-1").expect("deregister");

    assert_eq!(roots.resolve("ws-1"), Err(RootError::NotRegistered));
    assert!(
        roots.resolve("ws-2").is_ok(),
        "closing one workspace must not touch another's registration"
    );
}

#[test]
fn deregistering_something_never_registered_is_refused() {
    let roots = roots_with(&["/work/alpha"]);
    assert_eq!(roots.deregister("never"), Err(RootError::NotRegistered));
}

#[test]
fn a_workspace_can_be_registered_again_after_being_closed() {
    let roots = roots_with(&["/work/alpha"]);
    roots.register("ws-1", "/work/alpha").expect("register");
    roots.deregister("ws-1").expect("deregister");

    // Re-opening a workspace the developer closed is ordinary. Nothing about the close should
    // make the identity unusable afterwards.
    roots.register("ws-1", "/work/alpha").expect("re-register");
    assert!(roots.resolve("ws-1").is_ok());
}
