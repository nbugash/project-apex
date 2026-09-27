//! Whose git configuration decides what the engine reads.
//!
//! **Its own test binary, because the question needs a process-wide environment variable.**
//! `core.excludesFile` only reaches git through *global* config, and the only way for a test to
//! give the engine's child process one is to set `GIT_CONFIG_GLOBAL` for this process and let
//! it inherit. `std::env::set_var` is global to the binary, so these tests live apart from the
//! ones that must not see it.
//!
//! The first version of this file set the excludes in the repository's **local** config, which
//! the old blanket pin never blocked -- so the test passed either way and proved nothing about
//! the defect it was written for. Restoring the pin now fails it, which is the only evidence
//! that it tests anything.

mod common;

use apex_engine::adapters::outbound::git_cli::GitCli;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::git::Git;
use apex_engine::domain::path::ResolvedPath;
use common::repo::Repo;
use std::sync::Once;

/// One global git configuration for this binary, excluding a name nothing else here uses.
static GLOBAL: Once = Once::new();
const EXCLUDED: &str = "globally-ignored.log";

fn with_global_excludes() {
    GLOBAL.call_once(|| {
        let dir = std::env::temp_dir().join("apex-git-user-config");
        std::fs::create_dir_all(&dir).expect("config dir");
        let excludes = dir.join("excludes");
        std::fs::write(&excludes, format!("{EXCLUDED}\n")).expect("excludes");
        let config = dir.join("gitconfig");
        std::fs::write(
            &config,
            format!("[core]\n\texcludesFile = {}\n", excludes.display()),
        )
        .expect("gitconfig");
        // Inherited by every git the engine spawns from here on, which is the whole point:
        // this is a developer's own configuration arriving the way it really arrives.
        std::env::set_var("GIT_CONFIG_GLOBAL", &config);
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    });
}

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

#[test]
fn a_developer_s_own_excludes_are_honoured() {
    // **The client must agree with the terminal beside it.** `core.excludesFile` is the
    // developer's statement about which files are noise, and an engine that discarded it marked
    // paths their own `git status` does not -- which reads as a bug in the client every time.
    //
    // Found by pointing the application at this repository, where `.claude/` is excluded
    // globally, and findable no other way: every other fixture in this suite builds a pristine
    // repository with no user configuration, so the suite was structurally blind to it.
    with_global_excludes();
    let repo = Repo::new();
    repo.write(EXCLUDED, "chatter\n");
    repo.write("kept.rs", "fn k() {}\n");

    let status = GitCli::default()
        .status(&resolved(&repo.root))
        .expect("status");
    assert!(
        !status
            .changes
            .iter()
            .any(|c| c.path == format!("/{EXCLUDED}")),
        "a globally excluded file was reported: {:?}",
        status.changes
    );
    assert!(
        status.changes.iter().any(|c| c.path == "/kept.rs"),
        "excluding one file must not hide the others: {:?}",
        status.changes
    );
}

#[test]
fn a_configuration_that_hides_untracked_files_is_overridden() {
    // The other direction, and the one setting this feature does override. Reporting untracked
    // files is FR-009b and SC-014; a developer who sets `status.showUntrackedFiles=no` for
    // their terminal has not asked the tree to stop marking new files, and silently honouring
    // it would disable a documented requirement with no way to tell.
    let repo = Repo::new();
    repo.run(&["config", "status.showUntrackedFiles", "no"]);
    repo.write("brand-new.rs", "fn n() {}\n");

    let status = GitCli::default()
        .status(&resolved(&repo.root))
        .expect("status");
    assert!(
        status.changes.iter().any(|c| c.path == "/brand-new.rs"),
        "an untracked file vanished because of a user preference: {:?}",
        status.changes
    );
}

#[test]
fn an_external_diff_driver_does_not_silence_the_gutter() {
    // `diff.external` replaces the diff driver wholesale. With one configured and
    // `--no-ext-diff` absent, `git diff` emits no hunk headers at all — so every file reads as
    // unchanged and the gutter is blank on a repository that is anything but.
    let repo = Repo::new();
    repo.run(&["config", "diff.external", "/bin/true"]);
    repo.write("src/a.txt", "one\nCHANGED\nthree\nfour\n");

    let diff = GitCli::default()
        .file_diff(&resolved(&repo.root), "src/a.txt")
        .expect("diff");
    assert!(
        !diff.modified.is_empty() || !diff.added.is_empty(),
        "an external diff driver silenced the gutter: {diff:?}"
    );
}
