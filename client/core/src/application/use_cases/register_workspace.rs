//! Mint an identity, attach to an existing projection, tell the engine, and delete.

use crate::application::ports::clock::Clock;
use crate::application::ports::workspace_cache::{Attachment, CacheError, WorkspaceCache};
use crate::domain::workspace::{Location, Workspace, WorkspaceId};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    Cache(CacheError),
}

pub struct RegisterWorkspace {
    cache: Arc<dyn WorkspaceCache>,
    clock: Arc<dyn Clock>,
}

impl RegisterWorkspace {
    pub fn new(cache: Arc<dyn WorkspaceCache>, clock: Arc<dyn Clock>) -> Self {
        Self { cache, clock }
    }

    /// A fresh identity, minted client-side.
    ///
    /// A-WORKSPACE: minting here means a workspace is addressable before the engine has ever seen
    /// it, which is what the first connect needs.
    pub fn mint() -> WorkspaceId {
        WorkspaceId(uuid::Uuid::new_v4().to_string())
    }

    /// Register, or attach to what is already there.
    ///
    /// The decision is keyed on `WorkspaceId` and **nothing else**. Keying on a path or a display
    /// name would make two checkouts of one repository indistinguishable, which is the ordinary
    /// case rather than an edge one (FR-010, A-WORKSPACE).
    pub fn open(
        &self,
        id: WorkspaceId,
        name: String,
        location: Location,
    ) -> Result<(Workspace, Attachment), RegisterError> {
        let ws = Workspace {
            id,
            name,
            location,
            last_opened_at: self.clock.now(),
        };
        let attachment = self
            .cache
            .register(&ws, self.clock.now())
            .map_err(RegisterError::Cache)?;
        Ok((ws, attachment))
    }

    /// Remove a workspace's cached content **and** its tree (FR-012).
    ///
    /// Both go together by cascade. The cascade depends on the per-connection `foreign_keys`
    /// pragma, which a schema test asserts actually fires — without it this silently leaves
    /// orphaned rows and nothing says so.
    pub fn delete(&self, id: &WorkspaceId) -> Result<(), RegisterError> {
        self.cache.forget(id).map_err(RegisterError::Cache)
    }
}
