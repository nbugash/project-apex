//! The workspace read path, and what a `workspaceId` means.

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
}

impl InMemoryRoots {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self {
            fs,
            roots: Mutex::new(HashMap::new()),
        }
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
