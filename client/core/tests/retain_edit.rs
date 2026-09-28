//! What happens to a save the host cannot be told about.
//!
//! The dangerous property in this file is the **base**, and it is dangerous in a specific way:
//! every other failure here makes a test fail or a feature go quiet, whereas a base re-derived from
//! newer local content makes the eventual merge compare local against local and return a clean
//! merge that is simply wrong. Nothing about success notices, and the developer is handed a file
//! nobody wrote. That is why the base is asserted on the stored row rather than on a merge result.

mod common;

use apex_shell::application::ports::workspace_cache::{CacheError, WorkspaceCache};
use apex_shell::application::use_cases::retain_edit::RetainEdit;
use apex_shell::domain::workspace::{
    EntryKind, FsEntry, Location, RelPath, Sha256, Workspace, WorkspaceId,
};
use common::fake_cache::InMemoryCache;
use std::sync::Arc;

fn workspace(id: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(id.into()),
        name: "repo".into(),
        location: Location::Local {
            base: "/repo".into(),
        },
        last_opened_at: 0,
    }
}

fn path(p: &str) -> RelPath {
    RelPath::parse(p).expect("a valid path")
}

/// A retainer over a registered workspace, and the cache behind it.
fn fixture() -> (Arc<InMemoryCache>, RetainEdit, WorkspaceId) {
    let cache = Arc::new(InMemoryCache::new());
    let ws = workspace("w1");
    cache.register(&ws, 0).expect("register");
    let retain = RetainEdit::new(cache.clone());
    (cache, retain, ws.id)
}

/// US2 scenarios 1 and 2, FR-011: a save while disconnected is held, against the host's content.
#[test]
fn a_save_while_disconnected_is_retained_against_the_confirmed_base() {
    let (cache, retain, ws) = fixture();
    let host = b"fn main() {}\n";
    retain
        .save(
            &ws,
            &path("/main.rs"),
            b"fn main() { work(); }\n",
            Some((host, &Sha256::of(host))),
            true,
            7,
        )
        .expect("the save must be held");

    let pending = cache.pending_edits(&ws).expect("read");
    assert_eq!(pending.len(), 1);
    let (p, edit) = &pending[0];
    assert_eq!(p.as_str(), "/main.rs");
    assert_eq!(edit.content, b"fn main() { work(); }\n");
    let (base, hash) = edit.base.as_ref().expect("a base");
    assert_eq!(base, host, "the base is the content the host confirmed");
    assert_eq!(hash, &Sha256::of(host));
    assert_eq!(edit.retained_at, 7);
    assert!(edit.mergeable);
}

/// US2 scenario 5, FR-014: a file created offline is retained with no base at all.
///
/// Asserted on the stored row, because a base of `""` would read as real content everywhere
/// downstream -- a merge against an empty base makes every line look added.
#[test]
fn a_file_created_offline_is_retained_with_no_base() {
    let (cache, retain, ws) = fixture();
    retain
        .save(&ws, &path("/new.rs"), b"fresh\n", None, true, 1)
        .expect("retain");

    let pending = cache.pending_edits(&ws).expect("read");
    assert!(
        pending[0].1.base.is_none(),
        "there is nothing for a file created offline to differ from"
    );
}

/// US2 scenario 6, FR-016: a refusal reaches the caller while the work is still in the buffer.
#[test]
fn a_store_that_refuses_is_reported_and_not_swallowed() {
    let (cache, retain, ws) = fixture();
    cache.fail_writes("no space left on device");

    let outcome = retain.save(&ws, &path("/a.rs"), b"work\n", None, true, 1);

    // Returned, not logged. A use case that logged and returned `Ok` would leave the developer
    // believing the work was held, which is the one outcome FR-016 exists to prevent.
    let err = outcome.expect_err("a refused retain must reach the caller");
    assert!(
        matches!(err, CacheError::Store(ref why) if why.contains("no space")),
        "the reason must survive to the caller: {err:?}"
    );
    assert!(
        cache.pending_edits(&ws).expect("read").is_empty(),
        "nothing was held, and the caller has been told so"
    );
}

/// FR-017: retaining leaves cached content and its hashes untouched.
///
/// Compared by fingerprint before and after, because §5.3 says cache validity is a hash comparison
/// and nothing else. A retainer that quietly marked content stale would force a refetch of every
/// file being worked on -- online, where it would look like a slow network.
#[test]
fn retaining_does_not_disturb_the_cache() {
    let (cache, retain, ws) = fixture();
    let file = path("/main.rs");
    cache
        .put_listing(
            &ws,
            &RelPath::root(),
            &[FsEntry {
                name: "main.rs".into(),
                kind: EntryKind::File,
                size: 4,
                modified: 0,
            }],
        )
        .expect("listing");
    let id = cache
        .file_id(&ws, &file)
        .expect("ask")
        .expect("the file is listed");
    let host = b"host";
    assert!(
        matches!(
            cache.put_content(&id, host, &Sha256::of(host), 0),
            apex_shell::application::ports::workspace_cache::StoreOutcome::Stored
        ),
        "the fixture must actually cache something for this comparison to mean anything"
    );

    let before = cache.lookup(&ws, &file).expect("look").expect("cached");
    let fingerprint = (before.bytes.clone(), before.hash.clone());

    retain
        .save(
            &ws,
            &file,
            b"local",
            Some((host, &Sha256::of(host))),
            true,
            1,
        )
        .expect("retain");

    // **The retain must have happened**, or this test is a negative check that passes for an
    // implementation doing nothing at all. Found by mutation: with `save` stubbed to `Ok(())` the
    // assertion below held perfectly, because an absence of harm is exactly what a no-op achieves.
    assert_eq!(
        cache.pending_edits(&ws).expect("read").len(),
        1,
        "the work must be held before 'the cache is untouched' means anything"
    );

    let after = cache
        .lookup(&ws, &file)
        .expect("look")
        .expect("still cached");
    assert_eq!(
        (after.bytes, after.hash),
        fingerprint,
        "an offline save must not touch the cached copy or its digest (§5.3, FR-017)"
    );
}

/// FR-011b, FR-011c and SC-002a: the base is written once and never moves.
///
/// **The most dangerous property in the feature.** Saving the same file three times offline must
/// leave the base as the host's content throughout. A base re-derived from the newer local content
/// produces a merge that compares local against local -- a clean merge that is wrong, which no
/// assertion about success would catch.
#[test]
fn three_offline_saves_advance_the_content_and_never_the_base() {
    let (cache, retain, ws) = fixture();
    let file = path("/main.rs");
    let host = b"one\n";

    retain
        .save(
            &ws,
            &file,
            b"two\n",
            Some((host, &Sha256::of(host))),
            true,
            1,
        )
        .expect("first");
    // Each later save passes the *previous local* content as its base, which is what an editor
    // holding the file would naturally supply. The store must ignore it.
    retain
        .save(
            &ws,
            &file,
            b"three\n",
            Some((b"two\n", &Sha256::of(b"two\n"))),
            true,
            2,
        )
        .expect("second");
    retain
        .save(
            &ws,
            &file,
            b"four\n",
            Some((b"three\n", &Sha256::of(b"three\n"))),
            true,
            3,
        )
        .expect("third");

    let pending = cache.pending_edits(&ws).expect("read");
    assert_eq!(
        pending.len(),
        1,
        "one row per path, replaced not accumulated"
    );
    let edit = &pending[0].1;
    assert_eq!(
        edit.content, b"four\n",
        "the latest save is what reconciles"
    );
    let (base, hash) = edit.base.as_ref().expect("a base");
    assert_eq!(
        base, host,
        "the base must still be the content the host confirmed, not the previous local save"
    );
    assert_eq!(hash, &Sha256::of(host));
}

// FR-011a -- an unsaved buffer is not retained -- **is not observable here**, and the task that
// placed it in this file was wrong about that.
//
// The core never sees a keystroke. Its only entry point is `save`, so the test this file could
// hold is "nothing happens when nothing is called", which passed against a `save` stubbed to
// `Ok(())` and would pass against any implementation whatsoever. It was written, the mutation
// caught it, and it is gone rather than left looking like coverage.
//
// The property belongs where keystrokes exist: `tests/e2e/offline-state.spec.ts` types into a
// buffer without saving, quits, relaunches, and asserts the text is absent. That is the only place
// an implementation which persisted keystrokes would be caught.

/// FR-016a and edge case EC-02: a file deleted locally while offline produces no pending work.
///
/// Asserted rather than inferred. The alternative -- reading an absence as a deletion -- cannot
/// tell "the developer deleted this" from "this was never cached", and propagating the second
/// would delete a file on the host that nobody touched.
#[test]
fn a_local_deletion_while_offline_produces_no_pending_work() {
    let (cache, retain, ws) = fixture();
    let file = path("/gone.rs");
    retain
        .save(&ws, &file, b"content\n", None, true, 1)
        .expect("retain");
    assert_eq!(cache.pending_edits(&ws).expect("read").len(), 1);

    // Deleting locally while offline is not a save, so nothing calls the retainer. What the
    // developer's deletion must not do is leave a row that reconciliation would write back.
    cache.forget_pending(&ws, &file).expect("forget");

    assert!(
        cache.pending_edits(&ws).expect("read").is_empty(),
        "a deleted file carries no work to propagate (FR-016a)"
    );
}

// ---- FR-013: a file with retained work reads as the developer's content ----

use apex_shell::adapters::inbound::tauri_commands::pending_chunk;

/// The developer's content, and the **host's** digest.
///
/// Two assertions, and the second is the one that matters. Returning a digest of the local text
/// would look right in the editor and be wrong on the next save: the engine compares the base
/// against its own content and would refuse a write as stale against a version it has never held,
/// telling the developer a colleague edited their file when nobody did.
#[test]
fn a_reopened_file_reads_as_the_developers_content_against_the_hosts_digest() {
    let (cache, retain, ws) = fixture();
    let file = path("/main.rs");
    let host = b"host\n";
    retain
        .save(
            &ws,
            &file,
            b"mine\n",
            Some((host, &Sha256::of(host))),
            true,
            1,
        )
        .expect("retain");

    let chunk = pending_chunk(cache.as_ref(), &ws, &file)
        .expect("no failure")
        .expect("a path with work must read as that work");
    assert_eq!(
        chunk.text, "mine\n",
        "the developer's content, not the host's"
    );
    assert_eq!(
        chunk.sha256,
        Sha256::of(host).to_string(),
        "the digest stays the one the host confirmed, because that is what a save is conditional on"
    );
    assert_eq!(chunk.offset, 0, "a pending edit is the whole file");
}

/// A path with no retained work reads normally, so the provider is still asked.
#[test]
fn a_path_with_no_retained_work_is_left_to_the_provider() {
    let (cache, _retain, ws) = fixture();
    assert!(
        pending_chunk(cache.as_ref(), &ws, &path("/untouched.rs"))
            .expect("no failure")
            .is_none(),
        "only a path carrying work short-circuits the read"
    );
}

/// A file created offline reads as its content, with no digest at all.
#[test]
fn a_file_created_offline_reads_with_an_empty_digest() {
    let (cache, retain, ws) = fixture();
    let file = path("/new.rs");
    retain
        .save(&ws, &file, b"fresh\n", None, true, 1)
        .expect("retain");

    let chunk = pending_chunk(cache.as_ref(), &ws, &file)
        .expect("no failure")
        .expect("it has work");
    assert_eq!(chunk.text, "fresh\n");
    // Empty rather than fabricated. `Sha256::parse` refuses an empty string, so a save carrying it
    // is refused at the command boundary rather than sent to the engine to come back as a phantom
    // conflict the developer cannot explain.
    assert_eq!(chunk.sha256, "");
    assert!(
        apex_shell::domain::workspace::Sha256::parse(&chunk.sha256).is_none(),
        "the empty digest must not parse, or the refusal it depends on would not happen"
    );
}
