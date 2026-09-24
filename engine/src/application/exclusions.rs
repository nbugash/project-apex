//! The one resolved set of paths that are neither watched nor indexed.
//!
//! A-IGNORE requires a single set shared by the watcher and the indexer, because an indexer
//! that indexes what the watcher ignores returns search results for files whose changes are
//! never noticed. Neither existed when this was written, so this is where the set is born --
//! and it is stored on the registered workspace rather than inside the watcher precisely so
//! the indexer cannot later compute a second one.
//!
//! **The pattern subset is deliberate and bounded**, not an attempt at `.gitignore` in full.
//! Supported: a trailing `/` for directory-only, a leading `/` for anchoring to the file's own
//! directory, `*` within a segment, `**` across segments, `!` for negation, and `#` comments.
//! Not supported: character classes, escapes. The `ignore` crate handles all of it and pulls
//! `regex` with it, which A-BOOT makes a real cost on a binary transferred on every first
//! connect. A stated subset is a boundary; an unstated one is a bug waiting to be found.

use std::path::Path;

use crate::application::ports::file_system::FileSystem;
use crate::domain::path::CanonicalRoot;

/// §10.3's fixed list. Applies at any depth, whatever the repository says.
pub const BUILT_IN: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "__pycache__",
];

#[derive(Debug, Clone)]
struct Pattern {
    /// Workspace-relative directory the pattern was declared in. Empty for the root.
    base: String,
    body: String,
    negated: bool,
    directory_only: bool,
    anchored: bool,
}

#[derive(Debug, Default)]
pub struct ExclusionSet {
    patterns: Vec<Pattern>,
}

impl ExclusionSet {
    /// Walk the workspace once, collecting every `.gitignore`.
    ///
    /// Bounded by the built-in set: the walk never descends into a directory it has already
    /// decided to exclude, so a repository with a hundred thousand files in `node_modules`
    /// costs one `read_dir` there rather than a traversal of it. This is what git does, for
    /// the same reason.
    pub fn resolve(root: &CanonicalRoot, fs: &dyn FileSystem) -> Self {
        let mut set = Self::default();
        for name in BUILT_IN {
            set.patterns.push(Pattern {
                base: String::new(),
                body: (*name).to_string(),
                negated: false,
                directory_only: true,
                anchored: false,
            });
        }
        set.collect(root.as_path(), "", fs, 0);
        set
    }

    fn collect(&mut self, absolute: &Path, relative: &str, fs: &dyn FileSystem, depth: usize) {
        // A symlink loop inside a workspace would otherwise walk forever. Real trees are
        // nowhere near this deep, so the bound costs nothing and removes the failure mode.
        if depth > 64 {
            return;
        }
        let ignore_file = absolute.join(".gitignore");
        if let Ok(bytes) = fs.read_range(&ignore_file, 0, 1 << 20) {
            if let Ok(text) = String::from_utf8(bytes) {
                self.parse_into(&text, relative);
            }
        }
        let Ok(entries) = fs.read_dir(absolute) else {
            return;
        };
        for entry in entries.into_iter().filter(|e| e.is_directory) {
            let child_relative = if relative.is_empty() {
                entry.name.clone()
            } else {
                format!("{relative}/{}", entry.name)
            };
            if self.is_excluded(&child_relative, true) {
                continue; // never descend into what is already excluded
            }
            self.collect(&absolute.join(&entry.name), &child_relative, fs, depth + 1);
        }
    }

    fn parse_into(&mut self, text: &str, base: &str) {
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, rest) = match line.strip_prefix('!') {
                Some(r) => (true, r),
                None => (false, line),
            };
            let (directory_only, rest) = match rest.strip_suffix('/') {
                Some(r) => (true, r),
                None => (false, rest),
            };
            let (anchored, body) = match rest.strip_prefix('/') {
                Some(r) => (true, r),
                None => (rest.contains('/'), rest),
            };
            if body.is_empty() {
                continue;
            }
            self.patterns.push(Pattern {
                base: base.to_string(),
                body: body.to_string(),
                negated,
                directory_only,
                anchored,
            });
        }
    }

    /// Is this workspace-relative path excluded?
    ///
    /// Later patterns win, which is what makes `!` mean anything: a negation is only ever
    /// useful when something before it already matched.
    pub fn is_excluded(&self, relative_path: &str, is_directory: bool) -> bool {
        let mut excluded = false;
        for pattern in &self.patterns {
            // A trailing `/` means the pattern names a directory, so it must not exclude a
            // *file* of the same name -- `scratch/` and a file called `scratch` are different
            // things and git treats them so.
            let names_this =
                matches(pattern, relative_path) && (!pattern.directory_only || is_directory);
            // An excluded directory excludes everything beneath it. This is the half the
            // first version missed: it checked the ancestor and then still demanded the path
            // itself match, so `node_modules` excluded the directory and none of its contents
            // -- which is every file whose churn the exclusion exists to avoid.
            let under_excluded_ancestor = self.covers_ancestor(pattern, relative_path);
            if names_this || under_excluded_ancestor {
                excluded = !pattern.negated;
            }
        }
        excluded
    }

    /// Does this pattern name a directory that contains `relative_path`?
    fn covers_ancestor(&self, pattern: &Pattern, relative_path: &str) -> bool {
        let mut prefix = relative_path;
        while let Some((head, _)) = prefix.rsplit_once('/') {
            if matches(pattern, head) {
                return true;
            }
            prefix = head;
        }
        false
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

fn matches(pattern: &Pattern, relative_path: &str) -> bool {
    let Some(within) = strip_base(&pattern.base, relative_path) else {
        return false;
    };
    if pattern.anchored {
        return glob(&pattern.body, within);
    }
    // Unanchored: the pattern may match the path, or any suffix of it beginning at a segment
    // boundary. `node_modules` excludes `a/b/node_modules` as well as `node_modules`.
    if glob(&pattern.body, within) {
        return true;
    }
    let mut rest = within;
    while let Some((_, tail)) = rest.split_once('/') {
        if glob(&pattern.body, tail) {
            return true;
        }
        rest = tail;
    }
    false
}

fn strip_base<'a>(base: &str, path: &'a str) -> Option<&'a str> {
    if base.is_empty() {
        return Some(path);
    }
    path.strip_prefix(base)?.strip_prefix('/')
}

/// `*` within a segment, `**` across segments, everything else literal.
fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once("**") {
        Some((head, tail)) => {
            let head = head.trim_end_matches('/');
            let tail = tail.trim_start_matches('/');
            if !head.is_empty() && !glob_segment_prefix(head, text) {
                return false;
            }
            if tail.is_empty() {
                return true;
            }
            let mut rest = text;
            loop {
                if glob(tail, rest) {
                    return true;
                }
                match rest.split_once('/') {
                    Some((_, next)) => rest = next,
                    None => return false,
                }
            }
        }
        None => glob_flat(pattern, text),
    }
}

fn glob_segment_prefix(head: &str, text: &str) -> bool {
    text.split_once('/')
        .map(|(first, _)| glob_flat(head, first))
        .unwrap_or_else(|| glob_flat(head, text))
}

/// A single `*` never crosses a `/`.
fn glob_flat(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            if t[mark] == '/' {
                return false; // a single star stops at a segment boundary
            }
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}
