//! `diffy` appears in exactly one source file (Principle VIII).
//!
//! plan.md states this as a rule, and prose does not fail a build. The pattern is
//! `engine/tests/inotify_confinement.rs`, pointed at a different dependency for the same reason: if
//! the merge library may be named anywhere then anywhere may decide what a conflict is, and the
//! conflict boundary is the property SC-006b pins down.
//!
//! **Both halves.** Asserting only that no other file names `diffy` is the vacuous form of a
//! structural guard: delete or rename the adapter and it passes by finding nothing to complain
//! about. F011 shipped three checks of that shape and found them only by mutation.

use std::path::{Path, PathBuf};

const PERMITTED: &str = "text_merge.rs";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn diffy_is_named_in_exactly_one_file() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 5, "the walk found almost nothing: {files:?}");

    let mut offenders = Vec::new();
    let mut found_the_adapter = false;
    for file in &files {
        let name = file
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        // Comments are not uses. Several files explain *why* they must not name the library, and a
        // guard that fired on its own rationale would teach people to delete the rationale -- which
        // is what F011's first separation guard did before it learned this.
        let code: String = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.starts_with("//") && !l.starts_with("///") && !l.starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n")
            // The adapter's own module name may appear anywhere -- a `mod` line, a path in the
            // composition root. What may not appear elsewhere is a use of the library, which is
            // what the rule is about. `engine/src/main.rs` names `inotify_watcher::git_watch()` for
            // exactly this reason.
            .replace("text_merge", "")
            // And the adapter's own **type** name is not a use of the library either. Without this
            // the guard was vacuous in a way the mutation caught: `struct DiffyMerge` contains
            // "Diffy", so removing every `diffy::` call from the adapter left the "the adapter does
            // name it" half satisfied by the type's spelling alone.
            .replace("DiffyMerge", "");
        // A *use* of the crate, not a mention of its name: `diffy::` or `use diffy`. This is what
        // makes the guard fail when the call goes away rather than when the word does.
        if !code.contains("diffy::") && !code.contains("use diffy") {
            continue;
        }
        if name == PERMITTED {
            found_the_adapter = true;
        } else {
            offenders.push(name);
        }
    }

    // Both halves. Without the first, deleting the adapter would make this test pass by finding
    // nothing to complain about.
    assert!(
        found_the_adapter,
        "the adapter that is supposed to name diffy does not; this guard is asserting nothing"
    );
    assert!(
        offenders.is_empty(),
        "diffy escaped its adapter into {offenders:?}. If the merge library may be named anywhere \
         then anywhere may decide what a conflict is, and that boundary is what SC-006b pins down"
    );
}

#[test]
fn the_merge_port_names_no_implementation() {
    // The port is where the reconciler's dependency stops. A type from the adapter appearing here
    // would invert the dependency the port exists to create, and the confinement check above would
    // not notice: `DiffyMerge` is not the string `diffy`.
    let port = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/ports/text_merge.rs");
    let text = std::fs::read_to_string(&port).expect("the port exists");
    let code: String = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["DiffyMerge", "diffy", "adapters::"] {
        assert!(
            !code.contains(forbidden),
            "{forbidden} reached the merge port; a port that names its adapter is not a port"
        );
    }
}
