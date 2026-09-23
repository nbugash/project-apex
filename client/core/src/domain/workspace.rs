//! Workspace identity, paths and the shapes §6.1's trait signature fixes.
//!
//! Entities and their rules are defined in specs/005-workspace-cache/data-model.md. What lives
//! here is the domain: no SQLite, no Tauri, no serde envelopes (Principle VIII).

use std::fmt;

pub use apex_protocol::wire::{EntryKind, WorkspaceId};

/// An opaque file identity.
///
/// **Not derived from the path.** A-B5's first correction: path-derived identity meant a rename
/// produced a new identity and orphaned the cached blob. With an opaque id a rename updates the
/// path columns and leaves the content alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileId(pub String);

impl FileId {
    /// A fresh identity. Minted locally; nothing on the wire carries it.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for FileId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A SHA-256 digest, constructed from bytes and never from a hand-written string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256(String);

impl Sha256 {
    /// Hash the content. §5.6 requires this over **decompressed** bytes, so it is directly
    /// comparable with the engine's.
    pub fn of(bytes: &[u8]) -> Self {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(bytes);
        Self(format!("{:x}", h.finalize()))
    }

    /// Adopt a digest computed elsewhere — the engine's `stat`, or a stored row.
    ///
    /// Rejects anything that is not 64 lowercase hex characters. A malformed digest that reached
    /// a comparison would simply never match, turning a corrupt row into a permanent cache miss
    /// that looks like a slow network.
    pub fn parse(s: &str) -> Option<Self> {
        let ok = s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then(|| Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why a `RelPath` was refused.
///
/// The client's own check (FR-008). It protects against bugs; it is **not** what makes the system
/// safe, because the engine never receives this type — it receives a string off the wire and
/// validates it itself, which is the only arrangement in which "independently" is true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Contains a `..` component.
    Traversal,
    /// Not workspace-relative.
    Absolute,
    /// Contains a NUL, which no filesystem accepts and which truncates a C string.
    Nul,
}

/// A workspace-relative path, validated on construction (§6.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

impl RelPath {
    /// The workspace root.
    pub fn root() -> Self {
        Self("/".into())
    }

    /// Validate and normalise: a leading `/`, `/` separators, no trailing slash except at the
    /// root, and no empty or `.` components.
    pub fn parse(raw: &str) -> Result<Self, PathError> {
        if raw.contains('\0') {
            return Err(PathError::Nul);
        }
        if raw.starts_with("//") || raw.contains('\\') {
            return Err(PathError::Absolute);
        }
        let mut parts: Vec<&str> = Vec::new();
        for part in raw.split('/') {
            match part {
                "" | "." => continue,
                ".." => return Err(PathError::Traversal),
                p => parts.push(p),
            }
        }
        Ok(Self(format!("/{}", parts.join("/"))))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }

    /// The parent path, or `None` at the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        let cut = self.0.rfind('/').unwrap_or(0);
        Some(Self(if cut == 0 { "/".into() } else { self.0[..cut].to_string() }))
    }

    /// The final component, or `""` at the root.
    pub fn name(&self) -> &str {
        if self.is_root() {
            return "";
        }
        self.0.rsplit('/').next().unwrap_or("")
    }

    /// Append one child component.
    pub fn join(&self, child: &str) -> Result<Self, PathError> {
        Self::parse(&format!("{}/{}", self.0.trim_end_matches('/'), child))
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a workspace's content actually lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Remote { host: String, base: String },
    Local { base: String },
}

/// A registered root the developer browses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    /// Display only. The repository directory name, which **may collide freely** because nothing
    /// keys on it (A-WORKSPACE). Two checkouts of one repository is the ordinary case.
    pub name: String,
    pub location: Location,
    pub last_opened_at: i64,
}

/// One child of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: i64,
}

impl FsEntry {
    /// The contractual listing order: directories first, then by name, byte-wise on UTF-8.
    ///
    /// §4.8 makes this part of the protocol because the pagination cursor is the last name
    /// returned, and §5.4's sidebar query uses the same order so the client renders what it
    /// receives without re-sorting.
    pub fn listing_order(a: &Self, b: &Self) -> std::cmp::Ordering {
        let dir = |e: &Self| matches!(e.kind, EntryKind::Directory);
        dir(b).cmp(&dir(a)).then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
    }
}

/// Metadata for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsMeta {
    pub kind: EntryKind,
    pub size: u64,
    pub modified: i64,
    /// `None` for a directory.
    pub sha256: Option<Sha256>,
}

/// An offset and length within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

/// Part of a file, with the hash of the **whole** file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChunk {
    pub bytes: Vec<u8>,
    pub range: ByteRange,
    pub total_size: u64,
    /// Of the whole file, never of `bytes`. A caller assembling several ranges compares this
    /// across them: a change means the file moved underneath the read, and the assembled result
    /// must be discarded (FR-021).
    pub sha256: Sha256,
}

/// A requested page of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    /// The last name of the previous page.
    pub cursor: Option<String>,
    pub limit: u32,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self { cursor: None, limit: apex_protocol::wire::MAX_DIRECTORY_PAGE }
    }
}

/// A page of entries, with the cursor to continue from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirPage {
    pub items: Vec<FsEntry>,
    /// Present exactly when more entries follow.
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_is_refused_however_it_is_spelled() {
        for raw in ["../etc/passwd", "/a/../../b", "..", "/..", "a/b/../../.."] {
            assert_eq!(
                RelPath::parse(raw),
                Err(PathError::Traversal),
                "must refuse {raw:?}"
            );
        }
    }

    #[test]
    fn a_nul_is_refused_because_it_truncates_a_c_string() {
        assert_eq!(RelPath::parse("/a\0b"), Err(PathError::Nul));
    }

    #[test]
    fn normalisation_is_idempotent_and_collapses_noise() {
        for (raw, want) in [
            ("/src//main.rs", "/src/main.rs"),
            ("src/./main.rs", "/src/main.rs"),
            ("/src/", "/src"),
            ("", "/"),
            ("/", "/"),
        ] {
            let p = RelPath::parse(raw).expect("valid");
            assert_eq!(p.as_str(), want, "{raw:?}");
            assert_eq!(RelPath::parse(p.as_str()).unwrap(), p, "not idempotent: {raw:?}");
        }
    }

    #[test]
    fn parent_and_name_agree_with_the_path() {
        let p = RelPath::parse("/src/controllers/user.go").unwrap();
        assert_eq!(p.name(), "user.go");
        assert_eq!(p.parent().unwrap().as_str(), "/src/controllers");
        assert_eq!(RelPath::root().parent(), None);
        assert_eq!(RelPath::parse("/top").unwrap().parent().unwrap(), RelPath::root());
    }

    #[test]
    fn join_cannot_be_used_to_escape() {
        let base = RelPath::parse("/src").unwrap();
        assert_eq!(base.join("..").unwrap_err(), PathError::Traversal);
        assert_eq!(base.join("main.rs").unwrap().as_str(), "/src/main.rs");
    }

    #[test]
    fn a_digest_is_hex_of_the_bytes_and_a_malformed_one_is_refused() {
        let d = Sha256::of(b"");
        assert_eq!(
            d.as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "empty-string SHA-256 is a known constant"
        );
        assert_eq!(Sha256::parse(d.as_str()), Some(d));
        assert_eq!(Sha256::parse("ABC"), None);
        assert_eq!(Sha256::parse(&"z".repeat(64)), None, "not hex");
        assert_eq!(Sha256::parse(&"a".repeat(63)), None, "wrong length");
    }

    #[test]
    fn listing_order_puts_directories_first_then_bytewise_by_name() {
        let e = |n: &str, k| FsEntry { name: n.into(), kind: k, size: 0, modified: 0 };
        let mut v = vec![
            e("b.rs", EntryKind::File),
            e("Z", EntryKind::Directory),
            e("a.rs", EntryKind::File),
            e("a", EntryKind::Directory),
        ];
        v.sort_by(FsEntry::listing_order);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Z", "a", "a.rs", "b.rs"],
            "directories first; uppercase sorts before lowercase because the order is byte-wise, \
             which is what makes the pagination cursor reproducible on both ends"
        );
    }
}
