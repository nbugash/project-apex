//! Outbound port: the §6.1 trait every consumer of workspace content reaches it through.
//!
//! Guarantees are stated in specs/005-workspace-cache/contracts/provider.md. Three implementations
//! satisfy them after F003: the remote adapter, the application's caching layer, and an in-memory
//! fake. §6.4's local provider belongs to F015.
//!
//! `#[async_trait]` rather than native `async fn`, because FR-001 requires that a consumer cannot
//! tell a local provider from a remote one — so the concrete type is chosen at runtime and the
//! trait must be `dyn`-compatible, which native AFIT is not. F001's `RequestTransport` uses the
//! native form correctly: it is selected once at composition and never varies at a call site.

use crate::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, PageRequest, RelPath, Sha256, WorkspaceId,
};
use async_trait::async_trait;

/// Which feature owns a method this build does not implement.
///
/// Named rather than anonymous so a log reads as a schedule rather than a bug (FR-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// File watching and the invalidation it drives.
    F004FileWatch,
    /// The editor: writes, and `baseSha256` conflict handling.
    F006Editor,
    /// Engine-side content search.
    F013Search,
}

impl std::fmt::Display for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::F004FileWatch => "F004 file-watch-sync",
            Self::F006Editor => "F006 editor-integration",
            Self::F013Search => "F013 global-search",
        };
        write!(f, "{s}")
    }
}

/// Typed, never stringly: a caller distinguishes these without parsing a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Inside the root, and absent (`-32003`).
    NotFound,
    /// Escapes the root (`-32002`). Identical whether or not the target exists (FR-007).
    Refused,
    /// The engine has never been told about this workspace (`-32001`). **Re-register.**
    UnknownWorkspace,
    /// Registered, and the root is gone (`-32009`). **Tell the developer and stop projecting.**
    ///
    /// A separate variant rather than a flag on `UnknownWorkspace`: the two lead to opposite
    /// responses, and a flag is something a caller can forget to read.
    WorkspaceGone,
    /// No connection. Consulted before the cache, so an outage costs nothing and times out never.
    Offline,
    /// Above the inline read limit; the caller should route to `BulkTransfer` (A-BULKSIZE).
    TooLarge { total_size: u64 },
    /// The transport failed, or the engine sent something unintelligible.
    Transport(String),
    /// Declared by §6.1, implemented by a later feature (FR-004).
    Unsupported { owner: Owner },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "no such file or directory"),
            Self::Refused => write!(f, "path refused: outside the workspace root"),
            Self::UnknownWorkspace => write!(f, "workspace is not registered with the engine"),
            Self::WorkspaceGone => write!(f, "the workspace root no longer exists"),
            Self::Offline => write!(f, "not connected"),
            Self::TooLarge { total_size } => {
                write!(f, "{total_size} bytes exceeds the inline read limit")
            }
            Self::Transport(why) => write!(f, "transport: {why}"),
            Self::Unsupported { owner } => write!(f, "not implemented here; {owner} owns it"),
        }
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// The §6.1 trait. The UI never learns which implementation is active.
#[async_trait]
pub trait WorkspaceProvider: Send + Sync {
    /// One page of a directory's immediate children. Never recurses (§10.1).
    async fn read_directory(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage>;

    async fn stat(&self, ws: &WorkspaceId, path: &RelPath) -> ProviderResult<FsMeta>;

    /// Ranged read. `range: None` reads the whole file.
    ///
    /// Returns bytes, never text: build artifacts, images and PDFs are legal content, and an
    /// encoding guess corrupts them silently (FR-003).
    async fn read_file(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk>;

    // ---- Declared by §6.1, refused here. Each names its owner and has no side effect. ----

    async fn write_file(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _content: &[u8],
        _base: &Sha256,
    ) -> ProviderResult<Sha256> {
        Err(ProviderError::Unsupported {
            owner: Owner::F006Editor,
        })
    }

    async fn create_file(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<Sha256> {
        Err(ProviderError::Unsupported {
            owner: Owner::F006Editor,
        })
    }

    async fn create_directory(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<()> {
        Err(ProviderError::Unsupported {
            owner: Owner::F006Editor,
        })
    }

    async fn rename(
        &self,
        _ws: &WorkspaceId,
        _from: &RelPath,
        _to: &RelPath,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported {
            owner: Owner::F006Editor,
        })
    }

    async fn delete(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _recursive: bool,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported {
            owner: Owner::F006Editor,
        })
    }

    async fn search(&self, _ws: &WorkspaceId, _query: &str) -> ProviderResult<Vec<RelPath>> {
        Err(ProviderError::Unsupported {
            owner: Owner::F013Search,
        })
    }

    async fn watch(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<()> {
        Err(ProviderError::Unsupported {
            owner: Owner::F004FileWatch,
        })
    }
}
