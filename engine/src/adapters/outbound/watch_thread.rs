//! One thread owns the watcher, the coalescer and the clock.
//!
//! Not a mutex around the watcher. `poll` blocks for as long as the coalescer's next window,
//! and a lock held across it would make every `workspace/watch` request wait behind it -- up to
//! the whole window, inside an interaction the developer initiated. Commands arrive by channel
//! instead, so the only thing either side ever waits for is the other side's reply.
//!
//! No async runtime. `engine/Cargo.toml` records why: the engine is transferred on every first
//! connect, and one `std::thread` plus a poll with a timeout does the entire job.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{
    FileEvent as WireEvent, FileEventKind, FileEventParams, InvalidateAllParams, WatchResult,
    WorkspaceId,
};

use crate::adapters::inbound::rpc::encode_notification;
use crate::adapters::outbound::frame_writer::FrameWriter;
use crate::application::coalescer::{Coalescer, Emission};
use crate::application::exclusions::ExclusionSet;
use crate::application::ports::clock::{Clock, Millis};
use crate::application::ports::file_system::FileSystem;
use crate::application::ports::file_watcher::FileWatcher;
use crate::application::use_cases::watch::{release_all, unwatch_paths, watch_paths};
use crate::domain::path::CanonicalRoot;
use crate::domain::watch::{EventKind, FileEvent, WatchSet};

/// How long to wait when nothing is pending. Long enough not to spin; short enough that a stop
/// request is honoured promptly.
const IDLE_POLL_MS: Millis = 50;

pub enum Command {
    Watch {
        paths: Vec<String>,
        reply: Sender<WatchResult>,
    },
    Unwatch {
        paths: Vec<String>,
        reply: Sender<u32>,
    },
    ReleaseAll,
}

pub struct WatchService {
    commands: Sender<Command>,
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl WatchService {
    /// Take ownership of a watcher and run it until `stop`.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        workspace: WorkspaceId,
        root: CanonicalRoot,
        mut watcher: Box<dyn FileWatcher>,
        clock: Arc<dyn Clock>,
        fs: Arc<dyn FileSystem>,
        exclusions: Arc<ExclusionSet>,
        writer: Arc<FrameWriter>,
        codec: FrameCodec,
    ) -> Self {
        let (tx, rx): (Sender<Command>, Receiver<Command>) = channel();
        let running = Arc::new(AtomicBool::new(true));
        let alive = Arc::clone(&running);

        let handle = std::thread::spawn(move || {
            let mut set = WatchSet::new();
            let mut coalescer = Coalescer::default();

            while alive.load(Ordering::Relaxed) {
                while let Ok(command) = rx.try_recv() {
                    match command {
                        Command::Watch { paths, reply } => {
                            let result = watch_paths(
                                &root,
                                &mut set,
                                watcher.as_mut(),
                                &exclusions,
                                fs.as_ref(),
                                &paths,
                            );
                            let _ = reply.send(result);
                        }
                        Command::Unwatch { paths, reply } => {
                            let n = unwatch_paths(
                                &root,
                                &mut set,
                                watcher.as_mut(),
                                fs.as_ref(),
                                &paths,
                            );
                            let _ = reply.send(n);
                        }
                        Command::ReleaseAll => release_all(&mut set, watcher.as_mut()),
                    }
                }

                let now = clock.now();
                // The coalescer decides how long to wait. Timing lives in the pure component
                // and this thread owns no policy of its own.
                let timeout = coalescer
                    .next_deadline()
                    .map(|due| due.saturating_sub(now))
                    .unwrap_or(IDLE_POLL_MS)
                    .min(IDLE_POLL_MS);

                for raw in watcher.poll(timeout) {
                    // Excluded paths are dropped here as well as never watched. The second
                    // check is not redundant: a watch on a directory reports its children, and
                    // an excluded child of a watched directory would otherwise be delivered.
                    if exclusions.is_excluded(&raw.relative_path, raw.is_directory) {
                        continue;
                    }
                    let at = clock.now();
                    coalescer.accept(raw, at);
                }

                match coalescer.drain_due(clock.now()) {
                    Emission::Batch(events) if events.is_empty() => {}
                    Emission::Batch(events) => {
                        let params = FileEventParams {
                            workspace_id: workspace.clone(),
                            events: events.iter().map(to_wire).collect(),
                        };
                        if let Some(frame) =
                            encode_notification(&codec, "workspace/onFileEvent", &params)
                        {
                            let _ = writer.write_interactive(&frame);
                        }
                    }
                    Emission::InvalidateAll => {
                        let params = InvalidateAllParams {
                            workspace_id: workspace.clone(),
                        };
                        if let Some(frame) =
                            encode_notification(&codec, "workspace/invalidateAll", &params)
                        {
                            let _ = writer.write_interactive(&frame);
                        }
                    }
                }
            }
            release_all(&mut set, watcher.as_mut());
        });

        Self {
            commands: tx,
            running,
            handle: Some(handle),
        }
    }

    pub fn watch(&self, paths: Vec<String>) -> Option<WatchResult> {
        let (tx, rx) = channel();
        self.commands
            .send(Command::Watch { paths, reply: tx })
            .ok()?;
        rx.recv().ok()
    }

    pub fn unwatch(&self, paths: Vec<String>) -> Option<u32> {
        let (tx, rx) = channel();
        self.commands
            .send(Command::Unwatch { paths, reply: tx })
            .ok()?;
        rx.recv().ok()
    }

    pub fn release_all(&self) {
        let _ = self.commands.send(Command::ReleaseAll);
    }
}

impl Drop for WatchService {
    fn drop(&mut self) {
        // FR-004: watches are released when the engine exits, not left to the kernel to reap.
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Domain event to wire event. Metadata on created and modified only: a deleted path has
/// nothing to describe, and a rename describes a move rather than a file now there (FR-013a).
fn to_wire(e: &FileEvent) -> WireEvent {
    use apex_protocol::wire::EntryKind;
    let describes = matches!(e.kind, EventKind::Created | EventKind::Modified);
    WireEvent {
        event: match &e.kind {
            EventKind::Created => FileEventKind::Created,
            EventKind::Modified => FileEventKind::Modified,
            EventKind::Deleted => FileEventKind::Deleted,
            EventKind::Renamed { .. } => FileEventKind::Renamed,
        },
        relative_path: e.relative_path.clone(),
        to_path: match &e.kind {
            EventKind::Renamed { to } => Some(to.clone()),
            _ => None,
        },
        kind: describes.then_some(if e.is_directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        }),
        // A directory has nothing to measure.
        size: (describes && !e.is_directory).then_some(e.size),
        modified: describes.then_some(e.modified),
    }
}
