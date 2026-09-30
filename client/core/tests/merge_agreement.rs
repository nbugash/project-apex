//! SC-006b: the client's decision to merge or prompt matches `git merge-file`, on every pair.
//!
//! **This is the evidence for the feature's central decision.** research.md chose `diffy` over
//! writing the merge because agreement with git's conflict boundary could be *measured* rather than
//! intended, and this suite is that measurement. Eight cases were run by hand during planning; this
//! runs twenty-two on every build.
//!
//! **Where `git` is absent this test fails and does not skip.** The house pattern for a missing
//! external tool is `skip_reason()`, which `bootstrap_real_sshd.rs` uses four times, and following
//! it here would retire the evidence for the central decision while the suite stayed green. Needing
//! `git` does not contradict research.md's reason for refusing to *shell out* to it in production:
//! that is about the shipped client on a developer's machine, and this runs where the repository is
//! checked out. A subprocess is also fine under `make no-network`'s `unshare -rn`, where a socket
//! would not be.

use apex_shell::adapters::outbound::text_merge::DiffyMerge;
use apex_shell::application::ports::text_merge::{MergeOutcome, TextMerge};
use std::io::Write;
use std::process::Command;

/// One pair: what the host confirmed, what the developer saved, what the host holds now.
struct Case {
    name: &'static str,
    base: &'static str,
    local: &'static str,
    remote: &'static str,
}

/// Twenty-two pairs. The first eight are research.md's measured corpus, kept so the recorded
/// decision and the shipped check are the same cases.
const CASES: &[Case] = &[
    Case {
        name: "changes far apart",
        base: "a\nb\nc\nd\ne\nf\ng\nh\n",
        local: "A\nb\nc\nd\ne\nf\ng\nh\n",
        remote: "a\nb\nc\nd\ne\nf\ng\nH\n",
    },
    Case {
        // The row that decides the design. A zero-context merge combines this and is wrong.
        name: "adjacent lines",
        base: "a\nb\nc\nd\n",
        local: "a\nB\nc\nd\n",
        remote: "a\nb\nC\nd\n",
    },
    Case {
        name: "same line, different content",
        base: "a\nb\nc\n",
        local: "a\nMINE\nc\n",
        remote: "a\nTHEIRS\nc\n",
    },
    Case {
        name: "one side unchanged",
        base: "a\nb\nc\n",
        local: "a\nB\nc\n",
        remote: "a\nb\nc\n",
    },
    Case {
        name: "both sides made the identical change",
        base: "a\nb\nc\n",
        local: "a\nSAME\nc\n",
        remote: "a\nSAME\nc\n",
    },
    Case {
        // The other row that decides the design. This must merge, or the feature is useless.
        name: "two lines apart",
        base: "a\nb\nc\nd\ne\n",
        local: "a\nB\nc\nd\ne\n",
        remote: "a\nb\nc\nD\ne\n",
    },
    Case {
        name: "insert vs insert at one point",
        base: "a\nb\n",
        local: "a\nMINE\nb\n",
        remote: "a\nTHEIRS\nb\n",
    },
    Case {
        name: "delete vs edit of the same line",
        base: "a\nb\nc\n",
        local: "a\nc\n",
        remote: "a\nB\nc\n",
    },
    // Beyond research.md's eight.
    Case {
        name: "both sides append, different text",
        base: "a\n",
        local: "a\nmine\n",
        remote: "a\ntheirs\n",
    },
    Case {
        name: "local appends, remote untouched",
        base: "a\n",
        local: "a\nmine\n",
        remote: "a\n",
    },
    Case {
        name: "remote appends, local untouched",
        base: "a\n",
        local: "a\n",
        remote: "a\ntheirs\n",
    },
    Case {
        name: "local prepends, remote appends",
        base: "b\nc\nd\ne\nf\n",
        local: "a\nb\nc\nd\ne\nf\n",
        remote: "b\nc\nd\ne\nf\ng\n",
    },
    Case {
        name: "both delete the same line",
        base: "a\nb\nc\n",
        local: "a\nc\n",
        remote: "a\nc\n",
    },
    Case {
        name: "local deletes, remote deletes elsewhere",
        base: "a\nb\nc\nd\ne\nf\n",
        local: "b\nc\nd\ne\nf\n",
        remote: "a\nb\nc\nd\ne\n",
    },
    Case {
        name: "local empties the file",
        base: "a\nb\nc\n",
        local: "",
        remote: "a\nB\nc\n",
    },
    Case {
        name: "remote empties the file",
        base: "a\nb\nc\n",
        local: "a\nB\nc\n",
        remote: "",
    },
    Case {
        name: "whitespace-only change on one side",
        base: "a\nb\nc\n",
        local: "a\n  b\nc\n",
        remote: "a\nb\nc\n",
    },
    Case {
        name: "whitespace-only change on both sides, same line",
        base: "a\nb\nc\n",
        local: "a\n  b\nc\n",
        remote: "a\n\tb\nc\n",
    },
    Case {
        name: "no trailing newline on one side",
        base: "a\nb\nc\n",
        local: "a\nB\nc",
        remote: "a\nb\nC\n",
    },
    Case {
        name: "long file, one change each, far apart",
        base: "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n",
        local: "ONE\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n",
        remote: "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\nTWENTY\n",
    },
    Case {
        name: "identical rewrite of the whole file",
        base: "a\nb\nc\n",
        local: "x\ny\nz\n",
        remote: "x\ny\nz\n",
    },
    Case {
        name: "different rewrites of the whole file",
        base: "a\nb\nc\n",
        local: "x\ny\nz\n",
        remote: "1\n2\n3\n",
    },
];

/// Whether `git merge-file` considers this a clean merge.
///
/// `-p` writes to stdout rather than overwriting, and the exit status is the number of conflicts:
/// zero is clean, anything else is a conflict. A failure to *run* git is a panic, not a skip.
fn git_merges_cleanly(case: &Case) -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let write = |name: &str, body: &str| {
        let p = dir.path().join(name);
        let mut f = std::fs::File::create(&p).expect("create");
        f.write_all(body.as_bytes()).expect("write");
        p
    };
    // `git merge-file <ours> <base> <theirs>`.
    let ours = write("ours", case.local);
    let base = write("base", case.base);
    let theirs = write("theirs", case.remote);

    let out = Command::new("git")
        .arg("merge-file")
        .arg("-p")
        .arg(&ours)
        .arg(&base)
        .arg(&theirs)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "SC-006b needs `git` on this machine and it could not be run: {e}. This test fails \
                 rather than skipping on purpose: it is the evidence for choosing `diffy` over \
                 writing the merge, and a silent skip would retire that evidence while the suite \
                 stayed green."
            )
        });
    match out.status.code() {
        Some(0) => true,
        Some(n) if n > 0 && n < 128 => false,
        other => panic!(
            "git merge-file exited unexpectedly ({other:?}) for {}",
            case.name
        ),
    }
}

#[test]
fn every_pair_agrees_with_git_merge_file() {
    assert!(
        CASES.len() >= 20,
        "SC-006b asks for at least twenty pairs; this corpus has {}",
        CASES.len()
    );
    let merge = DiffyMerge::new();

    let mut disagreements = Vec::new();
    for case in CASES {
        let git_clean = git_merges_cleanly(case);
        let ours_clean = matches!(
            merge.merge(case.base, case.local, case.remote),
            MergeOutcome::Clean(_)
        );
        if git_clean != ours_clean {
            disagreements.push(format!(
                "{}: git says {}, we say {}",
                case.name,
                if git_clean { "clean" } else { "conflict" },
                if ours_clean { "clean" } else { "conflict" }
            ));
        }
    }

    println!(
        "SC-006b merge decisions matching `git merge-file`: {}/{} pairs",
        CASES.len() - disagreements.len(),
        CASES.len()
    );
    assert!(
        disagreements.is_empty(),
        "SC-006b requires 100% agreement:\n  {}",
        disagreements.join("\n  ")
    );
}

/// FR-020a as behaviour, named separately so a failure says which property broke.
///
/// The corpus above would still pass at 100% if both git and `diffy` were replaced by something
/// that agreed with itself. These two assertions are about the boundary itself.
#[test]
fn adjacent_lines_conflict_and_two_lines_apart_merge() {
    let merge = DiffyMerge::new();

    let adjacent = merge.merge("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n");
    assert!(
        matches!(adjacent, MergeOutcome::Conflict(_)),
        "changes on neighbouring lines must conflict; a zero-context merge combines them and is \
         the reason FR-020a names context-aware semantics"
    );

    let apart = merge.merge("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n");
    assert!(
        matches!(apart, MergeOutcome::Clean(_)),
        "changes two lines apart must merge, or the feature prompts for everything and is useless"
    );
    if let MergeOutcome::Clean(text) = apart {
        assert!(
            text.contains('B') && text.contains('D'),
            "both changes survive: {text:?}"
        );
    }
}

/// `git merge-file -p --diff3`'s output, whatever its exit status. `--diff3` because that is
/// `diffy`'s default style, so the two drafts carry the same three sections per conflict.
fn git_draft(base: &str, local: &str, remote: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let write = |name: &str, body: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, body).expect("write");
        p
    };
    let (ours, base, theirs) = (
        write("ours", local),
        write("base", base),
        write("theirs", remote),
    );
    let out = Command::new("git")
        .args(["merge-file", "-p", "--diff3"])
        .arg(&ours)
        .arg(&base)
        .arg(&theirs)
        .output()
        .expect("SC-006b needs `git` on this machine; see `git_merges_cleanly`");
    String::from_utf8(out.stdout).expect("utf-8")
}

/// A draft with every marker block removed: what the merge settled on its own.
///
/// Compared rather than the whole draft because the marker *labels* differ -- git names the files
/// it was given, `diffy` says "ours" and "theirs" -- and a label is not a merge decision.
fn settled(draft: &str) -> (Vec<&str>, usize) {
    let (mut kept, mut blocks, mut inside) = (Vec::new(), 0, false);
    for line in draft.lines() {
        if line.starts_with("<<<<<<<") {
            inside = true;
            blocks += 1;
        } else if line.starts_with(">>>>>>>") {
            inside = false;
        } else if !inside {
            kept.push(line);
        }
    }
    (kept, blocks)
}

/// US4 scenario 7: two offline edits in different regions, and a host change close to one of them.
/// The developer is prompted for **that region**, on the terms a version-control merge uses, not for
/// the file as a whole.
///
/// The fixture puts the host's change on the line next to the second edit and far from the first,
/// or the test could not tell per-region from per-file: a per-file prompt would leave the first
/// edit inside the markers too.
#[test]
fn a_conflict_draft_marks_only_the_colliding_region_as_git_does() {
    let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let local = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nNINE\nten\n";
    let remote = "one\ntwo\nthree\nfour\nfive\nsix\nseven\nEIGHT\nnine\nten\n";

    let MergeOutcome::Conflict(draft) = DiffyMerge::new().merge(base, local, remote) else {
        panic!("the host changed the line next to an offline edit, which must conflict");
    };
    let (ours_settled, ours_blocks) = settled(&draft);
    assert_eq!(ours_blocks, 1, "one colliding region, one prompt:\n{draft}");
    assert!(
        ours_settled.contains(&"ONE"),
        "the far edit is merged outside the markers, not asked about again:\n{draft}"
    );
    assert!(
        !ours_settled.contains(&"NINE") && !ours_settled.contains(&"EIGHT"),
        "the colliding lines are inside the markers:\n{draft}"
    );

    let git = git_draft(base, local, remote);
    let (git_settled, git_blocks) = settled(&git);
    assert_eq!(
        (ours_settled, ours_blocks),
        (git_settled, git_blocks),
        "the region prompted for is the region git would prompt for.\n  ours:\n{draft}\n  git:\n{git}"
    );
}
