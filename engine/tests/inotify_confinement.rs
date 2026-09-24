//! `inotify` appears in exactly one source file (Principle VIII).
//!
//! plan.md states this as a rule, and prose does not fail a build. The pattern is already
//! established in this repository: the mock daemon's guard reads its own directory and fails if
//! a forbidden name appears, which is what stops a test double acquiring engine behaviour one
//! reasonable-looking method at a time. This is the same idea pointed at a dependency.
//!
//! The rule is what makes the volume requirements testable at all. If `inotify` may be named
//! anywhere, then anywhere may decide something, and deciding requires a filesystem to test.

use std::path::{Path, PathBuf};

const PERMITTED: &str = "inotify_watcher.rs";

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
fn inotify_is_named_in_exactly_one_file() {
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
        // Comments are not uses. Several files explain *why* they must not name the library,
        // and a guard that fired on its own rationale would teach people to delete the
        // rationale. The module declaration is likewise a wiring line, not a use.
        let code: String = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.starts_with("//") && !l.starts_with("///") && !l.starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n")
            // The adapter's own name may be referenced anywhere -- a module declaration, a
            // path in the composition root. What may not appear anywhere else is a use of the
            // library itself, which is what the rule is actually about.
            .replace("inotify_watcher", "");
        if !code.contains("inotify") && !code.contains("Inotify") {
            continue;
        }
        if name == PERMITTED {
            found_the_adapter = true;
        } else {
            offenders.push(name);
        }
    }

    // Both halves. Without the second, deleting the adapter would make this test pass by
    // finding nothing to complain about -- the vacuous form of a structural guard.
    assert!(
        found_the_adapter,
        "the adapter that is supposed to name inotify does not; this guard is asserting nothing"
    );
    assert!(
        offenders.is_empty(),
        "inotify escaped its adapter into {offenders:?}. Everything that decides anything must \
         be testable without a filesystem, which is only true while the name stays in one file"
    );
}

#[test]
fn the_pure_components_name_no_io_at_all() {
    // The coalescer is where the volume requirements live. A filesystem, a thread or a
    // serialiser appearing in it is the first step to those requirements needing a real tree.
    let coalescer = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/coalescer.rs");
    let text = std::fs::read_to_string(&coalescer).expect("the coalescer exists");
    let body: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["std::fs", "std::thread", "serde", "inotify", "Instant"] {
        assert!(
            !body.contains(forbidden),
            "{forbidden} reached the coalescer; it is fed events and told the time, nothing more"
        );
    }
}
