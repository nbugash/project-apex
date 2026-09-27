//! A real git repository, built on demand, in the six shapes this feature has to read.
//!
//! Real rather than a fixture of captured output, because the parser is only half of what is
//! under test: `git_dir` resolution, the two watches and the coalescer all need a repository that
//! actually changes. Captured output is used where the *format* is the subject
//! (`git_parse.rs` holds its own strings for the cases a real repository will not produce on
//! demand, such as a corrupt record).
//!
//! **The worktree shape is not an optional extra.** FR-004 exists because a linked worktree's
//! `.git` is a file rather than a directory, and a fixture that cannot produce one cannot test
//! the requirement — it would pass for an implementation that assumes `<root>/.git`, which is
//! the single most plausible way to get this wrong.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Repo {
    pub root: PathBuf,
    _dir: tempfile::TempDir,
}

/// Run git in `dir` and fail loudly. A silent git failure here would surface much later as an
/// empty status, which is exactly the answer a broken fixture and a broken implementation share.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} could not run: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

impl Repo {
    /// An initialised repository with one commit, and nothing else yet.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        git(dir.path(), &["init", "-q", "repo"]);
        git(&root, &["config", "user.email", "t@example.invalid"]);
        git(&root, &["config", "user.name", "Fixture"]);
        // A deterministic starting branch: git's default varies by version and by the
        // installation's own configuration, and a test that asserted "master" would fail on a
        // machine configured for "main" for reasons having nothing to do with this feature.
        git(&root, &["checkout", "-q", "-B", "main"]);
        let me = Self { root, _dir: dir };
        me.write("src/a.txt", "one\ntwo\nthree\nfour\n");
        me.write("kept.txt", "kept\n");
        me.write("doomed.txt", "doomed\n");
        git(&me.root, &["add", "-A"]);
        git(&me.root, &["commit", "-qm", "initial"]);
        me
    }

    pub fn write(&self, rel: &str, body: &str) {
        let p = self.root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.root.join(rel)).unwrap();
    }

    pub fn run(&self, args: &[&str]) -> String {
        git(&self.root, args)
    }

    /// All six shapes at once: modified, staged, untracked, deleted, renamed, conflicted.
    ///
    /// One repository rather than six, because the parser reads them from one stream and the
    /// interesting failures are about record boundaries — a rename's extra field desynchronising
    /// everything after it is only observable when something follows it.
    pub fn with_every_shape(&self) {
        self.write("src/a.txt", "one\nCHANGED\nthree\nfour\nfive\n"); // modified in worktree
        self.write("staged.txt", "staged\n");
        self.run(&["add", "staged.txt"]); // staged, no further edit
        self.write("untracked.txt", "new\n"); // untracked
        self.remove("doomed.txt"); // deleted in worktree
        self.run(&["mv", "kept.txt", "renamed.txt"]); // rename, staged
        self.make_conflict();
    }

    /// A real merge conflict, which git reports with a `u` record.
    fn make_conflict(&self) {
        let head = self.run(&["rev-parse", "HEAD"]).trim().to_string();
        self.run(&["checkout", "-q", "-b", "other", &head]);
        self.write("conflict.txt", "theirs\n");
        self.run(&["add", "conflict.txt"]);
        self.run(&["commit", "-qm", "theirs"]);
        self.run(&["checkout", "-q", "main"]);
        self.write("conflict.txt", "ours\n");
        self.run(&["add", "conflict.txt"]);
        self.run(&["commit", "-qm", "ours"]);
        // Expected to fail: that is the point.
        let _ = Command::new("git")
            .args(["merge", "other"])
            .current_dir(&self.root)
            .output();
    }

    /// A linked worktree, where `.git` is a **file** holding a `gitdir:` pointer.
    ///
    /// Returns the worktree's own root. `rev-parse --git-dir` from inside it resolves to
    /// `<main>/.git/worktrees/<name>`, which holds that worktree's own `HEAD` and `index` —
    /// verified in research.md rather than assumed.
    pub fn add_worktree(&self, name: &str) -> PathBuf {
        let path = self.root.parent().unwrap().join(name);
        self.run(&["worktree", "add", "-q", path.to_str().unwrap()]);
        assert!(
            path.join(".git").is_file(),
            "a linked worktree's .git must be a file; if this fails the fixture proves nothing \
             about FR-004"
        );
        path
    }

    /// Leave HEAD detached, which git reports as the literal `(detached)`.
    pub fn detach(&self) {
        let head = self.run(&["rev-parse", "HEAD"]).trim().to_string();
        self.run(&["checkout", "-q", "--detach", &head]);
    }
}

/// A directory that is emphatically not a repository.
pub fn not_a_repo() -> (PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().join("plain");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("file.txt"), "no git here\n").unwrap();
    (root, dir)
}
