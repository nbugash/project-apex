//! `workspace/register` (§4.8) — what a `workspaceId` means, and what it never means.
mod common;

use apex_engine::application::ports::roots::{RootError, WorkspaceRoots};
use apex_engine::application::use_cases::workspace::{
    resolve_request, InMemoryRoots, RequestRefusal,
};
use apex_protocol::wire::codes;
use common::FakeFileSystem;
use std::sync::Arc;

fn roots_with(paths: &[(&str, bool)]) -> (Arc<FakeFileSystem>, InMemoryRoots) {
    let fs = Arc::new(FakeFileSystem::new());
    for (p, is_dir) in paths {
        if *is_dir {
            fs.dir(p);
        } else {
            fs.file(p, b"x");
        }
    }
    let roots = InMemoryRoots::new(fs.clone());
    (fs, roots)
}

#[test]
fn registering_the_same_id_against_the_same_path_is_idempotent() {
    let (_fs, roots) = roots_with(&[("/w", true)]);
    let first = roots.register("ws-1", "/w").expect("first registration");
    let again = roots
        .register("ws-1", "/w")
        .expect("re-registering must succeed");
    assert_eq!(
        first, again,
        "FR-011: attach rather than build a second projection"
    );
}

#[test]
fn registering_one_id_against_a_different_path_is_a_conflict() {
    let (_fs, roots) = roots_with(&[("/w", true), ("/other", true)]);
    roots.register("ws-1", "/w").unwrap();
    assert_eq!(
        roots.register("ws-1", "/other"),
        Err(RootError::Conflict {
            existing: "/w".into()
        }),
        "two meanings for one identity is exactly what workspaceId exists to prevent, so this \
         is an error rather than a silent re-point"
    );
}

#[test]
fn a_path_that_is_not_a_directory_is_refused_at_registration() {
    let (_fs, roots) = roots_with(&[("/w", true)]);
    roots
        .register("ws-file", "/w/nope")
        .expect_err("missing path");
    let (fs, roots2) = roots_with(&[("/w", true)]);
    fs.file("/w/a-file", b"x");
    assert_eq!(
        roots2.register("ws-file", "/w/a-file"),
        Err(RootError::Unusable),
        "refused here rather than on the first read, so the failure names the workspace instead \
         of a file inside it"
    );
}

#[test]
fn an_unregistered_id_is_not_registered_rather_than_not_found() {
    let (fs, roots) = roots_with(&[("/w", true)]);
    let refusal =
        resolve_request(&roots, fs.as_ref(), "never-seen", "/anything").expect_err("must refuse");
    assert_eq!(refusal, RequestRefusal::Root(RootError::NotRegistered));
    assert_eq!(refusal.wire().0, codes::WORKSPACE_NOT_REGISTERED);
}

/// FR-038, SC-015. The distinction this test exists for is the whole point of `-32009`.
#[test]
fn a_registered_root_that_has_been_deleted_is_gone_and_not_merely_missing() {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    fs.file("/w/src/main.rs", b"fn main() {}");
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("ws-1", "/w").expect("registers");

    // A missing path *inside* a live root: an ordinary not-found.
    let missing =
        resolve_request(&roots, fs.as_ref(), "ws-1", "/src/absent.rs").expect_err("must refuse");
    assert_eq!(missing.wire().0, codes::NOT_FOUND);

    // Now the root itself is deleted underneath the engine.
    fs.remove("/w");
    fs.remove("/w/src");
    fs.remove("/w/src/main.rs");

    let gone =
        resolve_request(&roots, fs.as_ref(), "ws-1", "/src/main.rs").expect_err("must refuse");
    assert_eq!(
        gone.wire().0,
        codes::WORKSPACE_GONE,
        "must be -32009, not -32003 and not -32001. -32003 reports a deleted workspace as a \
         missing file. -32001 means 're-register', which sends the client into a registration \
         that then fails because the root is no longer a directory, surfacing a registration \
         error for a deletion."
    );
    assert_ne!(gone.wire().0, codes::NOT_FOUND);
    assert_ne!(gone.wire().0, codes::WORKSPACE_NOT_REGISTERED);
}
