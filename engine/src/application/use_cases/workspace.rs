//! The workspace read path, and what a `workspaceId` means.

use crate::application::exclusions::ExclusionSet;
use crate::application::ports::file_system::FileSystem;
use crate::application::ports::roots::{RootError, WorkspaceRoots};
use crate::domain::path::{CanonicalRoot, PathRefusal, ResolvedPath};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// `workspaceId` -> canonical root, for the engine's lifetime.
pub struct InMemoryRoots {
    fs: Arc<dyn FileSystem>,
    /// The registered path as given, beside its canonical form. The former is what a conflict is
    /// reported against, because it is what the client asked for.
    roots: Mutex<HashMap<String, (String, CanonicalRoot)>>,
    /// The resolved exclusion set, per workspace, computed once at registration.
    ///
    /// **Here rather than inside the watcher, deliberately.** A-IGNORE requires one set shared by
    /// the watcher and the indexer, because an indexer that indexes what the watcher ignores
    /// returns search results for files whose changes are never noticed. Neither existed when
    /// this was written, so storing it on the registered workspace is what makes the sharing
    /// structural: a later indexer reads it from here rather than computing a second one, and
    /// "somebody will remember" is exactly the assumption A-IGNORE was written to remove.
    exclusions: Mutex<HashMap<String, Arc<ExclusionSet>>>,
}

impl InMemoryRoots {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self {
            fs,
            roots: Mutex::new(HashMap::new()),
            exclusions: Mutex::new(HashMap::new()),
        }
    }

    /// The one exclusion set for this workspace. `None` when it is not registered.
    pub fn exclusions(&self, id: &str) -> Option<Arc<ExclusionSet>> {
        self.exclusions
            .lock()
            .expect("exclusions poisoned")
            .get(id)
            .cloned()
    }
}

impl WorkspaceRoots for InMemoryRoots {
    fn register(&self, id: &str, path: &str) -> Result<CanonicalRoot, RootError> {
        let mut roots = self.roots.lock().expect("roots registry poisoned");
        if let Some((existing, root)) = roots.get(id) {
            // Idempotent for the same path (FR-011, A-WORKSPACE); an error for a different one.
            return if existing == path {
                Ok(root.clone())
            } else {
                Err(RootError::Conflict {
                    existing: existing.clone(),
                })
            };
        }
        // Canonicalised once, here. Every later request is then a resolve and a prefix
        // comparison rather than a second canonicalisation of the root.
        let root = ResolvedPath::canonical_root(Path::new(path), self.fs.as_ref())
            .map_err(|_| RootError::Unusable)?;
        // Walked once, here. Re-walking on every watch request would put a filesystem
        // traversal inside the interaction budget; a `.gitignore` edit taking effect on
        // re-registration is an acceptable staleness, and it is recorded as a decision rather
        // than discovered as a bug (research.md, *Where the exclusion set lives*).
        let set = ExclusionSet::resolve(&root, self.fs.as_ref());
        self.exclusions
            .lock()
            .expect("exclusions poisoned")
            .insert(id.to_string(), Arc::new(set));
        roots.insert(id.to_string(), (path.to_string(), root.clone()));
        Ok(root)
    }

    fn resolve(&self, id: &str) -> Result<CanonicalRoot, RootError> {
        let roots = self.roots.lock().expect("roots registry poisoned");
        let (_, root) = roots.get(id).ok_or(RootError::NotRegistered)?;
        // Re-check that the root is still there. A registered workspace whose directory has been
        // deleted underneath the engine is `Gone`, not a missing path inside it — the two demand
        // opposite responses from the client, and conflating them lets a developer keep browsing
        // a projection of something that no longer exists (FR-038).
        match self.fs.metadata(root.as_path()) {
            Ok(m) if m.is_directory => Ok(root.clone()),
            _ => Err(RootError::Gone),
        }
    }
}

/// Resolve a request's workspace and path together.
///
/// Every workspace method starts here, which is what makes the containment check impossible to
/// skip: there is no other way to obtain a `ResolvedPath`.
pub fn resolve_request(
    roots: &dyn WorkspaceRoots,
    fs: &dyn FileSystem,
    workspace_id: &str,
    relative: &str,
) -> Result<ResolvedPath, RequestRefusal> {
    let root = roots.resolve(workspace_id).map_err(RequestRefusal::Root)?;
    ResolvedPath::resolve(&root, relative, fs).map_err(RequestRefusal::Path)
}

/// Why a workspace request could not be served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestRefusal {
    Root(RootError),
    Path(PathRefusal),
}

impl RequestRefusal {
    /// The §4.4 code, and a message for the developer.
    pub fn wire(&self) -> (i32, String) {
        use apex_protocol::wire::codes;
        match self {
            Self::Root(RootError::NotRegistered) => (
                codes::WORKSPACE_NOT_REGISTERED,
                "workspace is not registered with this engine".into(),
            ),
            Self::Root(RootError::Gone) => (
                codes::WORKSPACE_GONE,
                "the workspace root no longer exists on this host".into(),
            ),
            Self::Root(RootError::Conflict { existing }) => (
                codes::WORKSPACE_NOT_REGISTERED,
                format!("this workspace id is already registered against {existing}"),
            ),
            Self::Root(RootError::Unusable) => (
                codes::WORKSPACE_NOT_REGISTERED,
                "that path is not a readable directory".into(),
            ),
            Self::Path(PathRefusal::Refused) => (
                codes::PATH_REFUSED,
                "path refused: outside the workspace root".into(),
            ),
            Self::Path(PathRefusal::NotFound) => {
                (codes::NOT_FOUND, "no such file or directory".into())
            }
        }
    }
}

// ---- The read path (§4.8). Every one starts at `resolve_request`, which is the only way to
// obtain a `ResolvedPath` — so the containment check cannot be skipped. ----

use apex_protocol::wire::{EntryKind, FsEntryWire, MAX_DIRECTORY_PAGE, MAX_INLINE_READ};

/// One page of a directory's immediate children.
///
/// Shallow, ordered `(type DESC, name ASC)` byte-wise on UTF-8, and paged. The ordering is
/// contractual because the cursor is the last name returned, so a page request needs no
/// server-side iterator and can never duplicate or skip a stable entry.
pub fn read_directory(
    fs: &dyn FileSystem,
    path: &ResolvedPath,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<(Vec<FsEntryWire>, Option<String>), std::io::Error> {
    let mut items: Vec<FsEntryWire> = fs
        .read_dir(path.as_path())?
        .into_iter()
        .map(|e| FsEntryWire {
            name: e.name,
            kind: if e.is_directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: e.size,
            modified: e.modified,
        })
        .collect();

    items.sort_by(|a, b| {
        let dir = |e: &FsEntryWire| matches!(e.kind, EntryKind::Directory);
        dir(b)
            .cmp(&dir(a))
            .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
    });

    if let Some(after) = cursor {
        // "Strictly after this token, in this order." Resumable with no server state, and a
        // cursor whose entry has since been deleted resumes where that entry would have sorted
        // rather than failing.
        //
        // The token encodes the **whole** ordering key, not just the name. Comparing on the name
        // alone is subtly wrong the moment a directory and a file interleave: with directories
        // `a` and `z` and a file `b`, the listing is `a, z, b`, and a name-only cursor after `z`
        // finds no later name and drops `b` entirely. That bug is invisible in any fixture whose
        // entries are all the same type, which is exactly what the first test fixture was.
        let at = items
            .iter()
            .position(|e| page_cursor(e).as_str() > after)
            .unwrap_or(items.len());
        items.drain(..at);
    }

    let limit = limit.unwrap_or(MAX_DIRECTORY_PAGE).min(MAX_DIRECTORY_PAGE) as usize;
    let more = items.len() > limit;
    items.truncate(limit);
    let next = more.then(|| items.last().map(page_cursor)).flatten();
    Ok((items, next))
}

/// The pagination cursor for an entry: an opaque token that sorts exactly as the listing does.
///
/// `0` for directories and `1` for files, so a plain string comparison reproduces
/// `(type DESC, name ASC)`. The separator is a unit separator, which cannot occur in a filename
/// this engine will list — `read_dir` drops names that are not valid UTF-8, and a `\x1f` in a
/// name would sort within its own type group rather than across it.
pub fn page_cursor(e: &FsEntryWire) -> String {
    let group = if matches!(e.kind, EntryKind::Directory) {
        '0'
    } else {
        '1'
    };
    format!("{group}\u{1f}{}", e.name)
}

/// Metadata, with the whole file's digest.
///
/// The digest is the sole input to cache validity (§5.3), and it is absent for a directory:
/// there is nothing to hash and no caller that needs it.
pub fn stat(
    fs: &dyn FileSystem,
    path: &ResolvedPath,
) -> Result<apex_protocol::wire::StatResult, std::io::Error> {
    let m = fs.metadata(path.as_path())?;
    let sha256 = if m.is_directory {
        None
    } else {
        // Hashing costs a read of the file. The client only stats what it may cache, and
        // A-CACHECAP bounds that, so this is bounded too.
        let bytes = fs.read_range(path.as_path(), 0, u64::MAX)?;
        Some(hex_digest(&bytes))
    };
    Ok(apex_protocol::wire::StatResult {
        kind: if m.is_directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: m.size,
        modified: m.modified,
        sha256,
    })
}

/// Why a read could not be served inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadRefusal {
    /// Above A-BULKSIZE's threshold. The caller routes to the bulk path instead of the engine
    /// truncating silently, because a silent truncation is a corrupt file the caller cannot see.
    TooLarge { total_size: u64 },
}

/// A ranged read, refused above the inline threshold.
pub fn read_file(
    fs: &dyn FileSystem,
    path: &ResolvedPath,
    offset: Option<u64>,
    length: Option<u64>,
) -> Result<Result<apex_protocol::wire::ReadFileResult, ReadRefusal>, std::io::Error> {
    let m = fs.metadata(path.as_path())?;
    let offset = offset.unwrap_or(0);
    let requested = length.unwrap_or_else(|| m.size.saturating_sub(offset));
    if requested > MAX_INLINE_READ {
        return Ok(Err(ReadRefusal::TooLarge { total_size: m.size }));
    }

    let bytes = fs.read_range(path.as_path(), offset, requested)?;
    // The digest describes the WHOLE file, never the returned range: a caller assembling several
    // ranges compares it across them, and a change means the file moved underneath the read.
    let whole = fs.read_range(path.as_path(), 0, u64::MAX)?;
    Ok(Ok(apex_protocol::wire::ReadFileResult {
        content: apex_protocol::base64::encode(&bytes),
        // Always base64. There is no utf8 path: assuming text corrupts binary content silently,
        // and a method that sometimes returns text makes every caller branch on it.
        encoding: "base64".into(),
        sha256: hex_digest(&whole),
        total_size: m.size,
    }))
}

fn hex_digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> FsEntryWire {
        FsEntryWire {
            name: name.into(),
            kind: if dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: 0,
            modified: 0,
        }
    }

    #[test]
    fn the_cursor_sorts_exactly_as_the_listing_does() {
        // Directories before files, then byte-wise by name. A plain string comparison on the
        // token must reproduce that, because the token IS how a page boundary is found.
        let mut tokens: Vec<String> = [
            entry("b.rs", false),
            entry("Z", true),
            entry("a.rs", false),
            entry("a", true),
        ]
        .iter()
        .map(page_cursor)
        .collect();
        tokens.sort();
        let names: Vec<&str> = tokens.iter().map(|t| &t[2..]).collect();
        assert_eq!(
            names,
            vec!["Z", "a", "a.rs", "b.rs"],
            "sorting the tokens must give the listing order; if it did not, a page would resume \
             in a different place than it left off"
        );
    }

    #[test]
    fn a_directory_token_always_precedes_a_file_token() {
        // The case a name-only cursor got wrong: a file whose name sorts before a directory's.
        let dir = page_cursor(&entry("zzz", true));
        let file = page_cursor(&entry("aaa", false));
        assert!(
            dir < file,
            "a file named `aaa` must still come after a directory named `zzz`, or paging across \
             the type boundary loses entries"
        );
    }

    #[test]
    fn tokens_are_distinct_for_a_directory_and_a_file_of_the_same_name() {
        // A directory and a file can share a name on some filesystems. If their tokens
        // collided, one would be skipped when resuming.
        assert_ne!(
            page_cursor(&entry("same", true)),
            page_cursor(&entry("same", false))
        );
    }

    #[test]
    fn the_digest_is_the_known_sha256() {
        assert_eq!(
            hex_digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
