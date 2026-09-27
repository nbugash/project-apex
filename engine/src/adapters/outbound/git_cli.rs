//! Git, as a subprocess.
//!
//! Everything git-specific lives here: which arguments, which output format, and how to read it.
//! The use case above sees `StatusSnapshot` and `GitDiffResult` and never a `Command`.
//!
//! **What git prints is untrusted input.** It is a program the engine did not write, reading a
//! repository the engine does not control, and a path it reports goes on to key a row in the
//! client's projection. So paths are contained here before they are emitted, exactly as a path
//! arriving off the wire is (Principle VI).

use crate::application::ports::git::{Git, GitFailure, StatusSnapshot};
use crate::domain::path::ResolvedPath;
use apex_protocol::wire::{BranchPosition, GitChange, GitDiffResult, GitStatusKind};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct GitCli {
    program: String,
}

impl Default for GitCli {
    fn default() -> Self {
        Self::new("git")
    }
}

impl GitCli {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_string(),
        }
    }

    /// Is this directory **itself** a repository's working tree?
    ///
    /// `rev-parse` walks upward, so a plain directory inside a checkout answers every git
    /// question with the enclosing repository's -- and that is not a workspace's git state, it
    /// is somebody else's. Verified against git directly: from a subdirectory,
    /// `--porcelain=v2` prints paths relative to the **repository** root and lists files
    /// outside the workspace entirely, so a workspace opened inside a larger repository would
    /// mark paths that do not exist in it and miss the ones that do.
    ///
    /// So the rule is equality, not ancestry: the workspace root must be the working tree's top
    /// level. Anything else degrades as a non-repository, which FR-027 already describes
    /// exactly -- no branch, no marks, workspace fully usable.
    ///
    /// The case this refuses is real and wanted -- a subdirectory of a monorepo as a workspace
    /// -- and refusing it cleanly is better than serving it wrongly. Supporting it means
    /// scoping the status and re-rooting every path, which is a feature rather than a check.
    fn is_the_top_level(&self, root: &Path) -> Result<(), GitFailure> {
        let out = self.run(root, &["rev-parse", "--show-toplevel"])?;
        let top = PathBuf::from(out.trim());
        // Compared canonically: the root has already been canonicalised, and git answers with a
        // resolved path, but a symlinked temporary directory makes the two differ textually
        // while naming one directory.
        let same = std::fs::canonicalize(&top)
            .ok()
            .zip(std::fs::canonicalize(root).ok())
            .map(|(a, b)| a == b)
            .unwrap_or(false);
        if same {
            Ok(())
        } else {
            Err(GitFailure::NotARepository)
        }
    }

    /// Run git and classify what happened.
    ///
    /// The environment is pinned so that a developer's own git configuration cannot change what
    /// the engine reads — `status.relativePaths`, a custom `core.quotePath`, an alias shadowing
    /// `status` — any of which would alter the output format under a parser that expects one.
    fn run(&self, dir: &Path, args: &[&str]) -> Result<String, GitFailure> {
        let out = Command::new(&self.program)
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => GitFailure::GitUnavailable,
                _ => GitFailure::Failed(e.to_string()),
            })?;

        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        // Distinguished here and nowhere else: both reach the client as an empty status, but the
        // engine's own diagnostics should not report an ordinary non-repository as a fault.
        if err.contains("not a git repository") || err.contains("Not a git repository") {
            Err(GitFailure::NotARepository)
        } else {
            Err(GitFailure::Failed(err.trim().to_string()))
        }
    }
}

impl Git for GitCli {
    fn git_dir(&self, root: &ResolvedPath) -> Result<PathBuf, GitFailure> {
        // `rev-parse --git-dir`, never `<root>/.git`. In a linked worktree or a submodule that
        // path is a *file* holding a `gitdir:` pointer, and the directory with `HEAD` and `index`
        // in it is elsewhere entirely — verified against git 2.43 in research.md.
        self.is_the_top_level(root.as_path())?;
        let out = self.run(root.as_path(), &["rev-parse", "--git-dir"])?;
        let raw = out.trim();
        if raw.is_empty() {
            return Err(GitFailure::Failed("git-dir was empty".into()));
        }
        let p = PathBuf::from(raw);
        Ok(if p.is_absolute() {
            p
        } else {
            root.as_path().join(p)
        })
    }

    fn status(&self, root: &ResolvedPath) -> Result<StatusSnapshot, GitFailure> {
        self.is_the_top_level(root.as_path())?;
        let raw = self.run(
            root.as_path(),
            // `--no-optional-locks` first, and it is not a micro-optimisation: an ordinary
            // `git status` refreshes the index stat cache and **writes `.git/index`**, which
            // is one of the two files A-GITWATCH watches. Without this, every status run wakes
            // the watch that schedules the next one -- a loop the coalescer bounds to a low
            // rate but never ends, on every open repository, forever. It also means a status
            // computation is no longer distinguishable from a developer staging a file.
            &[
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
            ],
        )?;
        parse_status(&raw)
    }

    fn file_diff(&self, root: &ResolvedPath, relative: &str) -> Result<GitDiffResult, GitFailure> {
        self.is_the_top_level(root.as_path())?;
        let rel = relative.trim_start_matches('/');
        // `--unified=0` removes context lines, so the hunk headers are exact and the changed
        // lines that follow them are discarded here rather than transmitted (§12.3).
        let raw = self.run(
            root.as_path(),
            &["diff", "--unified=0", "--no-color", "--", rel],
        )?;
        if raw.trim().is_empty() {
            // Either unchanged, or untracked — git diff says nothing about a path it does not
            // track. An untracked file is wholly new, which the caller resolves from its status.
            return Ok(GitDiffResult::default());
        }
        Ok(parse_diff(&raw))
    }
}

/// Turn a workspace-relative path from git into the rooted form the client keys on.
///
/// Returns `None` for anything that escapes, which is dropped rather than forwarded: git should
/// never print such a path, and if it does, the entry is not one this workspace owns.
/// A path git reported, as a workspace-relative path -- or nothing.
///
/// **A check, not a normalisation.** The previous version split on `/` and reassembled, which
/// rejected `..` and quietly reinterpreted an absolute path: `/etc/passwd` and `etc/passwd`
/// both came out as `/etc/passwd`, the first silently re-read as though it were relative to the
/// workspace. git reports repository-relative paths, so an absolute one means something is
/// wrong upstream, and reinterpreting it is exactly the repair Principle VI forbids --
/// contracts/git-status.md guarantee 7 says the entry is *dropped*.
///
/// Containment is a real property here because `is_the_top_level` establishes that the
/// repository root and the workspace root are the same directory. Without that, "relative to
/// the repository" and "inside the workspace" are different questions and this function could
/// not answer either.
fn contained(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    // Absolute on the host. Dropped rather than re-read as workspace-relative.
    if raw.starts_with('/') {
        return None;
    }
    let mut out = String::from("/");
    for part in raw.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p => {
                if out.len() > 1 {
                    out.push('/');
                }
                out.push_str(p);
            }
        }
    }
    (out.len() > 1).then_some(out)
}

/// One state from the two characters git reports.
///
/// The worktree character wins unless it is `.`, in which case the index character gives
/// `STAGED`. A staged file with further edits still holds work recorded nowhere, and reporting
/// it as staged would say the opposite (spec.md, *Clarifications*).
fn collapse(xy: &str) -> Option<GitStatusKind> {
    let mut chars = xy.chars();
    let index = chars.next()?;
    let worktree = chars.next()?;
    Some(match (index, worktree) {
        (_, 'M') | (_, 'T') => GitStatusKind::Modified,
        (_, 'D') => GitStatusKind::Deleted,
        (_, 'A') => GitStatusKind::Untracked,
        ('M', '.') | ('A', '.') | ('R', '.') | ('C', '.') | ('T', '.') => GitStatusKind::Staged,
        ('D', '.') => GitStatusKind::Deleted,
        _ => return None,
    })
}

/// Parse the whole of a `--porcelain=v2 -z --branch` stream.
///
/// **By record type, not by splitting into equal pieces.** Each type declares how many
/// NUL-terminated fields it consumes, and a `2` record consumes one extra for the path it came
/// from. Treating every field as a record start produces a phantom entry for that old path and
/// then misreads everything after it.
///
/// A record this build cannot read rejects the whole snapshot. A partial status is
/// indistinguishable from a repository where the missing files are clean, so the alternative to
/// an error is telling the developer their changes do not exist.
pub fn parse_status(raw: &str) -> Result<StatusSnapshot, GitFailure> {
    let mut fields = raw.split('\0').filter(|f| !f.is_empty()).peekable();
    let mut branch = BranchPosition::None;
    let mut oid: Option<String> = None;
    let mut changes: Vec<GitChange> = Vec::new();

    while let Some(field) = fields.next() {
        let (kind, rest) = field.split_at(field.find(' ').unwrap_or(field.len()));
        let rest = rest.trim_start();
        match kind {
            "#" => {
                // `# branch.oid <sha>` and `# branch.head <name>`.
                let mut it = rest.splitn(2, ' ');
                match (it.next(), it.next()) {
                    (Some("branch.oid"), Some(v)) => oid = Some(v.trim().to_string()),
                    (Some("branch.head"), Some(v)) => {
                        let v = v.trim();
                        // The literal git writes where a name goes when there is no branch.
                        branch = if v == "(detached)" {
                            BranchPosition::Detached(String::new())
                        } else {
                            BranchPosition::Branch(v.to_string())
                        };
                    }
                    _ => {}
                }
            }
            "1" | "2" => {
                // `<XY> <sub> <mH> <mI> <mW> <hH> <hI> [<score>] <path>`
                let parts: Vec<&str> = rest.splitn(if kind == "2" { 9 } else { 8 }, ' ').collect();
                let expected = if kind == "2" { 9 } else { 8 };
                if parts.len() != expected {
                    return Err(GitFailure::Failed(format!(
                        "unreadable `{kind}` record: {field}"
                    )));
                }
                let xy = parts[0];
                let path = parts[expected - 1];
                if kind == "2" {
                    // The extra field: where this file came from. Consumed so it cannot be read
                    // as the next record, and otherwise unused — a rename is reported under the
                    // path the file has now.
                    if fields.next().is_none() {
                        return Err(GitFailure::Failed(
                            "a rename record without its original path".into(),
                        ));
                    }
                }
                if let (Some(status), Some(p)) = (collapse(xy), contained(path)) {
                    changes.push(GitChange { path: p, status });
                }
            }
            "u" => {
                // `<xy> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
                let parts: Vec<&str> = rest.splitn(10, ' ').collect();
                if parts.len() != 10 {
                    return Err(GitFailure::Failed(format!(
                        "unreadable `u` record: {field}"
                    )));
                }
                if let Some(p) = contained(parts[9]) {
                    changes.push(GitChange {
                        path: p,
                        status: GitStatusKind::Conflict,
                    });
                }
            }
            "?" => {
                if let Some(p) = contained(rest) {
                    changes.push(GitChange {
                        path: p,
                        status: GitStatusKind::Untracked,
                    });
                }
            }
            // Ignored files are not a state §4.8 carries, and asking for them costs work.
            "!" => {}
            other => {
                return Err(GitFailure::Failed(format!(
                    "unknown status record type `{other}`"
                )))
            }
        }
    }

    if let (BranchPosition::Detached(_), Some(o)) = (&branch, &oid) {
        // The commit stands in for the name a detached head does not have. Carried **whole**:
        // shortening is presentation, and a wire that shortened would leave no way to identify
        // the commit from what arrived -- the interface shows seven characters and puts the
        // full id where it can be read (FR-018, `branch.ts`). This shortened here first, which
        // made the status bar's tooltip repeat its own label.
        branch = BranchPosition::Detached(o.trim().to_string());
    }

    Ok(StatusSnapshot { branch, changes })
}

/// Read `@@` hunk headers into coordinates, discarding everything else.
///
/// The counts are **elided when they are 1**, so `@@ -2 +2 @@` and `@@ -4,0 +5 @@` are both
/// well-formed and a parser assuming two numbers per side mishandles the first (research.md).
///
/// Content lines are skipped rather than collected: this function has no return path through
/// which text could reach a caller, which is how FR-021 is kept by construction.
pub fn parse_diff(raw: &str) -> GitDiffResult {
    let mut out = GitDiffResult::default();
    for line in raw.lines().filter(|l| l.starts_with("@@")) {
        let Some(body) = line.strip_prefix("@@ ").and_then(|r| r.split(" @@").next()) else {
            continue;
        };
        let mut sides = body.split_whitespace();
        let (Some(old), Some(new)) = (sides.next(), sides.next()) else {
            continue;
        };
        let (_, old_count) = parse_side(old);
        let (new_start, new_count) = parse_side(new);

        match (old_count, new_count) {
            (0, n) if n > 0 => out.added.push([new_start, new_start + n - 1]),
            (o, 0) if o > 0 => out.deleted.push(new_start),
            (_, n) if n > 0 => out.modified.push([new_start, new_start + n - 1]),
            _ => {}
        }
    }
    out
}

/// `-4,0` or `+5`. A missing count means one line, which is why it may be absent at all.
fn parse_side(s: &str) -> (u32, u32) {
    let s = s.trim_start_matches(['-', '+']);
    let mut it = s.splitn(2, ',');
    let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count = it.next().map_or(1, |v| v.parse().unwrap_or(1));
    (start, count)
}
