//! Offline path search, served from the projection.
//!
//! Never consults a provider (C6). That is not an optimisation: FR-031 requires that a
//! disconnected search return results **without a request being attempted**, and a use case that
//! held a provider could always grow a fallback that tried one.

use crate::application::ports::workspace_cache::{CacheError, WorkspaceCache};
use crate::domain::workspace::{RelPath, WorkspaceId};
use std::sync::Arc;

/// The largest result list a filter box can usefully show.
///
/// A bound rather than a preference: an unbounded search over a hundred-thousand-file projection
/// would hand the interface more rows than it can render inside §1.4's budget.
pub const DEFAULT_LIMIT: u32 = 100;

pub struct SearchPaths {
    cache: Arc<dyn WorkspaceCache>,
}

impl SearchPaths {
    /// Deliberately takes only the cache. There is no provider here to fall back to, which is
    /// what makes "zero requests attempted" structural rather than a promise.
    pub fn new(cache: Arc<dyn WorkspaceCache>) -> Self {
        Self { cache }
    }

    pub fn find(
        &self,
        ws: &WorkspaceId,
        fragment: &str,
        limit: Option<u32>,
    ) -> Result<Vec<RelPath>, CacheError> {
        self.cache
            .search_paths(ws, fragment, limit.unwrap_or(DEFAULT_LIMIT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_limit_is_bounded() {
        assert!(DEFAULT_LIMIT > 0 && DEFAULT_LIMIT <= 1000);
    }
}
