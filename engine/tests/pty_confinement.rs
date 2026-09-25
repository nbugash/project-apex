//! The pseudo-terminal is named in exactly one source file (Principle VIII).
//!
//! `inotify_confinement.rs` is the precedent and the reasoning is the same: plan.md states this
//! as a rule, and prose does not fail a build. If the mechanism may be named anywhere then
//! anywhere may decide something, and deciding requires a real process to test -- which is
//! exactly what F010's volume, ordering and backpressure requirements cannot afford.
//!
//! What is guarded is the **mechanism**, not the vocabulary. `Shape::Pty` is a domain word: it
//! says a task is attached to a terminal, which is a fact about the task rather than a syscall.
//! `openpty` is a mechanism. The distinction is what lets the use case talk about terminals
//! while remaining testable without one.

use std::path::{Path, PathBuf};

const PERMITTED: &str = "pty_runner.rs";

/// Names of the mechanism. Not `Pty`, which is domain vocabulary and appears in the wire types,
/// the domain and the use cases by design.
const MECHANISM: &[&str] = &[
    "nix::",
    "openpty",
    "forkpty",
    "posix_openpt",
    "grantpt",
    "unlockpt",
    "ptsname",
    "setsid",
    "setrlimit",
    "RLIMIT_",
    "killpg",
    "TIOCSWINSZ",
    "TIOCGWINSZ",
];

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

/// Strip comments and the adapter's own module name.
///
/// Comments are not uses. Several files explain *why* they must not name the mechanism, and a
/// guard firing on its own rationale teaches people to delete the rationale — which F004's
/// guard learned by firing on exactly that.
fn code_of(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("pty_runner", "")
}

/// Does `code` actually name `token`, rather than contain it inside a longer word?
///
/// `nix::` matched inside `std::os::unix::process` on the first run of this guard, and a guard
/// that fires on `unix` while looking for `nix` is one somebody switches off -- which is the
/// failure F004's version was narrowed twice to avoid.
fn names(code: &str, token: &str) -> bool {
    let mut from = 0;
    while let Some(at) = code[from..].find(token) {
        let start = from + at;
        let preceded_by_word = start > 0 && code.as_bytes()[start - 1].is_ascii_alphanumeric()
            || (start > 0 && code.as_bytes()[start - 1] == b'_');
        if !preceded_by_word {
            return true;
        }
        from = start + token.len();
    }
    false
}

#[test]
fn the_pseudo_terminal_is_named_in_exactly_one_file() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 5, "the walk found almost nothing: {files:?}");

    let mut offenders: Vec<(String, &str)> = Vec::new();
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
        let code = code_of(&text);
        for token in MECHANISM {
            if !names(&code, token) {
                continue;
            }
            if name == PERMITTED {
                found_the_adapter = true;
            } else {
                offenders.push((name.clone(), token));
            }
        }
    }

    // Both halves. Without the second, deleting the adapter would make this pass by finding
    // nothing to complain about -- the vacuous form of a structural guard, and the one F004
    // names explicitly.
    assert!(
        found_the_adapter,
        "no file names the pseudo-terminal, so this guard is asserting nothing. Either \
         {PERMITTED} does not exist yet, or it stopped using the mechanism it exists to own"
    );
    assert!(
        offenders.is_empty(),
        "the pseudo-terminal escaped its adapter into {offenders:?}. Everything that decides \
         anything must be testable without a process, which is only true while the mechanism \
         stays in one file"
    );
}

#[test]
fn the_pure_output_core_names_no_io_at_all() {
    // `output.rs` is where chunking, ordering and the retention bound live. A process, a thread
    // or a descriptor appearing in it is the first step to those requirements needing a real
    // pseudo-terminal, which is the whole thing this feature cannot afford to test with.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/output.rs");
    let text = std::fs::read_to_string(&path).expect("the output core exists");
    let body = code_of(&text);
    for forbidden in [
        "std::fs",
        "std::thread",
        "std::process",
        "serde",
        "Instant",
        "nix",
    ] {
        assert!(
            !body.contains(forbidden),
            "{forbidden} reached the output core; it is fed bytes and told the time, nothing more"
        );
    }
}

#[test]
fn the_task_domain_names_no_mechanism() {
    // The domain describes what a task *is*. `Shape::Pty` is allowed and is the point of the
    // distinction: a task attached to a terminal is a fact about the task, and how that
    // terminal is obtained is the adapter's business.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/task.rs");
    let text = std::fs::read_to_string(&path).expect("the task domain exists");
    let body = code_of(&text);
    for token in MECHANISM {
        assert!(
            !body.contains(token),
            "{token} reached the task domain, which must be decidable from values alone"
        );
    }
    assert!(
        body.contains("Pty"),
        "the domain should still describe a task attached to a terminal; if this fails the \
         guard above is passing for the wrong reason"
    );
}
