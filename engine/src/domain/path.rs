//! §4.7 path containment, enforced by the type system.
//!
//! `ResolvedPath` has no public constructor other than `resolve`, so a use case cannot name a
//! path it has not checked. That is Principle VI made structural rather than a rule reviewers
//! have to remember.

use crate::application::ports::file_system::FileSystem;
use std::path::{Path, PathBuf};

/// Why a path was not resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRefusal {
    /// Outside the workspace root — lexically, or after a symlink resolved.
    ///
    /// **Returned identically whether or not the escaped target exists** (FR-007). A caller that
    /// could tell the two apart would be able to probe the host's filesystem for the existence of
    /// arbitrary files, using nothing but refusals.
    Refused,
    /// Inside the root, and not there. Information the caller is entitled to.
    NotFound,
}

/// A canonical workspace root, resolved once at registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRoot(PathBuf);

impl CanonicalRoot {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// An absolute path proven to be a descendant of its workspace root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath(PathBuf);

impl ResolvedPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Canonicalise a workspace root. Done once, at registration, so the per-request check is a
    /// resolve and a prefix comparison rather than a second canonicalisation of the root.
    pub fn canonical_root(root: &Path, fs: &dyn FileSystem) -> Result<CanonicalRoot, PathRefusal> {
        let c = fs.canonicalize(root).map_err(|_| PathRefusal::NotFound)?;
        if fs.metadata(&c).map(|m| m.is_directory).unwrap_or(false) {
            Ok(CanonicalRoot(c))
        } else {
            Err(PathRefusal::NotFound)
        }
    }

    /// Resolve untrusted input against a canonical root.
    ///
    /// Two stages, and the order is the point.
    ///
    /// **Lexical first**, before the filesystem is consulted at all. `canonicalize` fails on a
    /// path that does not exist, so a single-stage check would answer "no such file" for an escape
    /// to a missing target and "refused" for an escape to a real one — telling a caller whether
    /// `/etc/shadow` is there. Rejecting the escape before touching the disk makes every escape
    /// produce the same answer (FR-007).
    ///
    /// **Canonical second**, because a path with no `..` in it can still leave the root through a
    /// symlink, and only resolving it catches that (FR-006).
    pub fn resolve(
        root: &CanonicalRoot,
        relative: &str,
        fs: &dyn FileSystem,
    ) -> Result<Self, PathRefusal> {
        let parts = lexical_parts(relative)?;
        let mut joined = root.0.clone();
        for p in &parts {
            joined.push(p);
        }

        match fs.canonicalize(&joined) {
            Ok(c) => {
                if c.starts_with(&root.0) {
                    Ok(Self(c))
                } else {
                    // A symlink took it out of the root.
                    Err(PathRefusal::Refused)
                }
            }
            Err(_) => {
                // The leaf is not there. Whether that is a miss or an escape depends on where its
                // parent lands, so resolve the parent: a contained parent means an honest miss, and
                // a parent outside the root means the caller was reaching out through a symlink and
                // must not learn that the target is absent.
                let parent = joined.parent().unwrap_or(&joined);
                match fs.canonicalize(parent) {
                    Ok(pc) if pc.starts_with(&root.0) => Err(PathRefusal::NotFound),
                    _ => Err(PathRefusal::Refused),
                }
            }
        }
    }
}

/// Split a workspace-relative path into components, refusing anything that could escape.
///
/// Rejects `..`, absolute forms, backslashes and NUL. A NUL never reaches the filesystem: it
/// terminates a C string, so `"/a\0/../../etc"` could be truncated to something the check above
/// already approved.
fn lexical_parts(relative: &str) -> Result<Vec<String>, PathRefusal> {
    if relative.contains('\0') || relative.contains('\\') {
        return Err(PathRefusal::Refused);
    }
    let mut parts = Vec::new();
    for part in relative.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(PathRefusal::Refused),
            p => parts.push(p.to_string()),
        }
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_refuses_every_spelling_of_escape() {
        for raw in ["..", "/..", "a/../..", "/a/../../b", "..\\b", "a\0b"] {
            assert_eq!(lexical_parts(raw), Err(PathRefusal::Refused), "{raw:?}");
        }
    }

    #[test]
    fn lexical_drops_empty_and_dot_components() {
        assert_eq!(
            lexical_parts("/src//./main.rs").unwrap(),
            vec!["src", "main.rs"]
        );
        assert_eq!(lexical_parts("/").unwrap(), Vec::<String>::new());
    }
}
