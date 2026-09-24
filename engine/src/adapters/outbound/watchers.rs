//! One watch service per registered workspace, made on demand.
//!
//! The composition root owns this. It exists because a workspace's watcher, coalescer and clock
//! are owned by one thread, and the thread cannot be made before the workspace it observes is
//! registered -- the root has to be canonical first.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{WatchResult, WorkspaceId};

use crate::adapters::outbound::frame_writer::FrameWriter;
use crate::adapters::outbound::watch_thread::WatchService;
use crate::application::exclusions::ExclusionSet;
use crate::application::ports::clock::Clock;
use crate::application::ports::file_system::FileSystem;
use crate::application::ports::file_watcher::FileWatcher;
use crate::domain::path::CanonicalRoot;

/// How a watcher and a clock are made for one workspace. A closure rather than a type so the
/// composition root decides, and so tests substitute in-memory doubles without a filesystem.
pub type WatcherFactory =
    Box<dyn Fn(&CanonicalRoot) -> Option<(Box<dyn FileWatcher>, Arc<dyn Clock>)> + Send + Sync>;

pub struct Watchers {
    services: Mutex<HashMap<String, WatchService>>,
    factory: WatcherFactory,
    fs: Arc<dyn FileSystem>,
    writer: Arc<FrameWriter>,
    codec: FrameCodec,
}

impl Watchers {
    pub fn new(
        factory: WatcherFactory,
        fs: Arc<dyn FileSystem>,
        writer: Arc<FrameWriter>,
        codec: FrameCodec,
    ) -> Self {
        Self {
            services: Mutex::new(HashMap::new()),
            factory,
            fs,
            writer,
            codec,
        }
    }

    fn with_service<T>(
        &self,
        workspace: &WorkspaceId,
        root: &CanonicalRoot,
        exclusions: Arc<ExclusionSet>,
        f: impl FnOnce(&WatchService) -> Option<T>,
    ) -> Option<T> {
        let mut services = self.services.lock().expect("watchers poisoned");
        if !services.contains_key(&workspace.0) {
            let (watcher, clock) = (self.factory)(root)?;
            services.insert(
                workspace.0.clone(),
                WatchService::spawn(
                    workspace.clone(),
                    root.clone(),
                    watcher,
                    clock,
                    Arc::clone(&self.fs),
                    exclusions,
                    Arc::clone(&self.writer),
                    self.codec.clone(),
                ),
            );
        }
        f(services.get(&workspace.0).expect("just inserted"))
    }

    pub fn watch(
        &self,
        workspace: &WorkspaceId,
        root: &CanonicalRoot,
        exclusions: Arc<ExclusionSet>,
        paths: Vec<String>,
    ) -> Option<WatchResult> {
        self.with_service(workspace, root, exclusions, |s| s.watch(paths))
    }

    pub fn unwatch(
        &self,
        workspace: &WorkspaceId,
        root: &CanonicalRoot,
        exclusions: Arc<ExclusionSet>,
        paths: Vec<String>,
    ) -> Option<u32> {
        self.with_service(workspace, root, exclusions, |s| s.unwatch(paths))
    }

    /// FR-004: everything goes when the workspace closes or the connection drops.
    pub fn forget(&self, workspace: &WorkspaceId) {
        let mut services = self.services.lock().expect("watchers poisoned");
        if let Some(service) = services.remove(&workspace.0) {
            service.release_all();
            drop(service); // joins the thread, which releases anything left
        }
    }
}
