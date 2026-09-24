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
    use crate::application::ports::workspace_cache::*;
    use crate::domain::cache::CacheEntry;
    use crate::domain::workspace::{FileId, FsEntry, Sha256, Workspace};
    use std::sync::Mutex;

    /// Records what limit it was asked for. The point of the use case is that it applies a bound
    /// even when the caller states none, so the bound has to be observable.
    #[derive(Default)]
    struct RecordingCache(Mutex<Vec<u32>>);

    impl WorkspaceCache for RecordingCache {
        fn search_paths(
            &self,
            _ws: &WorkspaceId,
            _fragment: &str,
            limit: u32,
        ) -> CacheResult<Vec<RelPath>> {
            self.0.lock().unwrap().push(limit);
            Ok(vec![RelPath::parse("/hit.rs").unwrap()])
        }

        fn register(&self, _: &Workspace, _: i64) -> CacheResult<Attachment> {
            unreachable!("search must not register")
        }
        fn forget(&self, _: &WorkspaceId) -> CacheResult<()> {
            unreachable!("search must not forget")
        }
        fn list_children(&self, _: &WorkspaceId, _: &RelPath) -> CacheResult<Vec<FsEntry>> {
            unreachable!("search must not list")
        }
        fn put_listing(&self, _: &WorkspaceId, _: &RelPath, _: &[FsEntry]) -> CacheResult<()> {
            unreachable!("search must not write")
        }
        fn lookup(&self, _: &WorkspaceId, _: &RelPath) -> CacheResult<Option<CacheEntry>> {
            unreachable!("search must not read content")
        }
        fn file_id(&self, _: &WorkspaceId, _: &RelPath) -> CacheResult<Option<FileId>> {
            unreachable!()
        }
        fn put_content(&self, _: &FileId, _: &[u8], _: &Sha256, _: i64) -> StoreOutcome {
            unreachable!("search must not write")
        }
        fn touch(&self, _: &FileId, _: i64) -> StoreOutcome {
            unreachable!(
                "a search is not an access: retention measures files opened, not files listed"
            )
        }
        fn rename(&self, _: &FileId, _: &RelPath) -> CacheResult<()> {
            unreachable!()
        }

        fn mark_stale(&self, _: &WorkspaceId, _: &RelPath) -> CacheResult<()> {
            unreachable!("this double exists for search; nothing here marks staleness")
        }
        fn mark_unproven(&self, _: &WorkspaceId, _: &RelPath) -> CacheResult<()> {
            unreachable!("this double exists for search; nothing here marks content")
        }
        fn clear_unproven(&self, _: &FileId) -> CacheResult<()> {
            unreachable!("this double exists for search; nothing here settles content")
        }
        fn rename_subtree(&self, _: &WorkspaceId, _: &RelPath, _: &RelPath) -> CacheResult<usize> {
            unreachable!("this double exists for search; nothing here renames a subtree")
        }
        fn evict(&self, _: i64) -> CacheResult<EvictionReport> {
            unreachable!("search must not evict")
        }
        fn schema_version(&self) -> CacheResult<u32> {
            unreachable!()
        }
        fn migrate_to(&self, _: u32, _: PhaseSink<'_>) -> Result<(), MigrationFailure> {
            unreachable!()
        }
    }

    #[test]
    fn a_caller_that_states_no_limit_still_gets_a_bounded_search() {
        let cache = Arc::new(RecordingCache::default());
        let s = SearchPaths::new(cache.clone());
        let hits = s.find(&WorkspaceId("w".into()), "hit", None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            *cache.0.lock().unwrap(),
            vec![DEFAULT_LIMIT],
            "an unbounded search over a hundred-thousand-file projection would hand the interface \
             more rows than it can render inside §1.4's budget"
        );
    }

    #[test]
    fn a_stated_limit_is_honoured() {
        let cache = Arc::new(RecordingCache::default());
        let s = SearchPaths::new(cache.clone());
        let _ = s.find(&WorkspaceId("w".into()), "hit", Some(7)).unwrap();
        assert_eq!(*cache.0.lock().unwrap(), vec![7]);
    }

    /// Every other cache operation panics in this double. If searching ever grew a side effect —
    /// recording an access, warming content — this test would fail rather than the behaviour
    /// changing quietly.
    #[test]
    fn searching_touches_nothing_else() {
        let s = SearchPaths::new(Arc::new(RecordingCache::default()));
        let _ = s.find(&WorkspaceId("w".into()), "hit", None).unwrap();
    }
}
