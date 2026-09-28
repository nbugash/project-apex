//! Composition root. The only place adapters are bound to use cases.
//!
//! Constitution Principle VIII: wiring lives here, not scattered behind hidden globals or a
//! service locator. Swapping the stub connection source for the real transport in F001 is a
//! one-line change in this file.

use crate::adapters::inbound::tauri_commands::{Shell, WorkspaceAccess};
use crate::adapters::outbound::deploy::SshStreamDeployer;
use crate::adapters::outbound::json_session_store::JsonFileSessionStore;
use crate::adapters::outbound::local_engine::LocalEngineSpawner;
use crate::adapters::outbound::openssh::{OpenSshSpawner, SshTransport};
use crate::adapters::outbound::remote_tasks::RemoteTasks;
use crate::adapters::outbound::stub_connection::StubConnectionStatusSource;
use crate::adapters::outbound::system_clock::SystemClock;
use crate::adapters::outbound::transport_sender::TransportSender;
use crate::application::ports::connection::ConnectionStatusSource;
use crate::application::ports::notification_sink::NotificationSink;
use crate::application::ports::session_store::SessionStore;
use crate::application::ports::spawner::ProcessSpawner;
use crate::application::ports::spawner::SpawnSpec;
use crate::application::ports::task_provider::TaskProvider;
use crate::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use crate::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
use crate::application::use_cases::register_workspace::RegisterWorkspace;
use crate::application::use_cases::restore_session::RestoreSession;
use crate::composition_workspace::prepare_cache;
use crate::domain::connection::ConnectionState;
use crate::domain::rail::RailCatalogue;
use crate::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, PageRequest, RelPath, Sha256, WorkspaceId,
};
use crate::window::controller::WindowController;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Wiring {
    pub shell: Shell,
    /// Which workspace the interface is looking at, shared with `Tasks`.
    ///
    /// Owned here because two things need it and neither owns the other: the commands, which set
    /// it when a workspace opens, and F012's reconciliation trigger, which reads it when the
    /// connection returns and has no `State` to reach it through.
    pub current_workspace: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// The workspace provider the interface reaches, with maintenance already run.
    ///
    /// Always present: a workspace command must be answerable before a host is configured, and
    /// answering "offline" is information while a missing command is a crash.
    pub workspace: WorkspaceAccess,
    pub stub: Arc<StubConnectionStatusSource>,
    /// Present when a host is configured. F002 gives F001's `HandToBootstrap` a recipient: the
    /// transport can already classify a missing engine, and this is what deploys one.
    pub deployer: Option<Arc<SshStreamDeployer>>,
    /// The task provider the terminal panel reaches, present only when an engine is.
    ///
    /// `Option` rather than a refusing implementation, unlike `DisconnectedWorkspace` below,
    /// and the asymmetry is deliberate. A workspace command must answer before an engine
    /// exists, because "offline" is information and a cached tree is still worth showing. A
    /// task command has nothing to say without an engine: there is no cached output, and a
    /// terminal with no engine is not a degraded terminal but an absent one.
    pub tasks: Option<Arc<dyn TaskProvider>>,
    /// The same connection the provider uses, for `workspace/register`.
    pub sender: Option<Arc<dyn crate::application::ports::request_sender::RequestSender>>,
}

/// Which engine to talk to, decided once, here.
enum Engine {
    /// A host was named: the engine runs there and `ssh` carries the protocol.
    Remote(SpawnSpec),
    /// No host named: run the engine binary as a child and speak its stdio.
    Local(std::path::PathBuf),
}

/// **Both forms are opt-in, and the local one deliberately so.**
///
/// The first version selected a local engine whenever the binary happened to exist, on the
/// reasoning that a packaged application would never have one. That was wrong in the way silent
/// behaviour changes usually are: every developer with a built workspace silently acquired a
/// live transport, and the connection status bar began reporting a real connection instead of
/// the stub -- which broke F001's `connection-status` spec, whose whole method is to drive that
/// stub. It failed on a five-second budget, naming neither the engine nor the cause.
///
/// An engine is a large thing to acquire from the presence of a file. `APEX_LOCAL_ENGINE` now
/// names one explicitly, exactly as `APEX_REMOTE_HOST` names a host.
fn engine_target() -> Option<Engine> {
    if let Some(spec) = remote_target() {
        return Some(Engine::Remote(spec));
    }
    let binary = LocalEngineSpawner::named()?;
    if !binary.exists() {
        crate::logging::warn(&format!(
            "{} names {}, which does not exist; running unconnected",
            crate::adapters::outbound::local_engine::ENGINE_BINARY_VAR,
            binary.display()
        ));
        return None;
    }
    Some(Engine::Local(binary))
}

/// The host to connect to, when one has been named.
///
/// Environment variables until the feature that owns remote configuration exists. They are
/// read here, in the composition root, rather than inside the transport: an adapter that
/// reads its own configuration from the environment is one that cannot be constructed two
/// different ways, which is the whole property this file protects.
fn remote_target() -> Option<SpawnSpec> {
    let host = std::env::var("APEX_REMOTE_HOST")
        .ok()
        .filter(|h| !h.is_empty())?;
    let user = std::env::var("APEX_REMOTE_USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| std::env::var("USER").ok())?;
    Some(SpawnSpec {
        host,
        user,
        assisted: false,
    })
}

pub fn build(
    data_dir: PathBuf,
    window: Arc<WindowController>,
    notifications: Arc<dyn NotificationSink>,
) -> Wiring {
    let store: Arc<dyn SessionStore> =
        Arc::new(JsonFileSessionStore::new(data_dir.join("session.json")));

    let restored = RestoreSession::new(store.clone()).execute(&window.attached_displays());
    if let Err(e) = window.apply(&restored.window) {
        crate::logging::warn(&format!("could not apply restored geometry: {e}"));
    }

    let persist = Arc::new(PersistSession::new(store, restored));

    let rail = Arc::new(RailCatalogue::default());
    let stub = Arc::new(StubConnectionStatusSource::new());

    // The swap this file exists for. Which adapter supplies connection state is decided
    // here and nowhere else; `ObserveConnection` and the status bar cannot tell them apart.
    //
    // It is conditional only because nothing configures a host yet — `WorkspaceReference`
    // records a name and a location type, not a host and a user, and the screen that
    // collects them belongs to a later feature. Connecting unconditionally would mean every
    // launch failing against a host nobody named, which reads as a broken application
    // rather than an unconfigured one.
    // Built alongside the transport, because a deployer without a connection has nothing to
    // deploy over — both are absent together when no host is configured.
    // The projection, migrated and evicted before any provider exists. `ReadyCache` is the only
    // thing that hands out a cache and the only way to obtain one runs maintenance first, so the
    // ordering FR-018c and FR-026a require is checked by the compiler rather than by review.
    let ready = prepare_cache(
        data_dir.clone(),
        Arc::new(|phase| crate::logging::info(&format!("cache maintenance: {phase:?}"))),
    );
    if ready.report.rebuilt {
        crate::logging::warn("the workspace cache was rebuilt; cached content will be refetched");
    }

    let mut deployer: Option<Arc<SshStreamDeployer>> = None;
    // Shared between the notification path, which applies what the engine pushes, and
    // `workspace_open`, which has to **ask** once -- because asking is what makes the engine
    // start watching. Two constructions would be two pagers accumulating separately.
    let mut apply_git: Option<
        Arc<crate::application::use_cases::apply_git_status::ApplyGitStatus>,
    > = None;
    let mut tasks: Option<Arc<dyn TaskProvider>> = None;
    let mut sender: Option<Arc<dyn crate::application::ports::request_sender::RequestSender>> =
        None;
    let source: Arc<dyn ConnectionStatusSource> = match engine_target() {
        Some(target) => {
            let (spawner, spec): (Arc<dyn ProcessSpawner>, SpawnSpec) = match target {
                Engine::Remote(spec) => {
                    crate::logging::info(&format!("connecting to {}@{}", spec.user, spec.host));
                    (Arc::new(OpenSshSpawner::default()), spec)
                }
                Engine::Local(binary) => {
                    crate::logging::info(&format!("running a local engine: {}", binary.display()));
                    (
                        Arc::new(LocalEngineSpawner::new(binary)),
                        // Required by the spawn contract and meaningless to a child process.
                        SpawnSpec {
                            host: "localhost".into(),
                            user: "local".into(),
                            assisted: false,
                        },
                    )
                }
            };
            let transport = Arc::new(SshTransport::new(spawner, spec));

            // The sender is built here rather than after `connect`, because git status has to
            // be applied from the moment the first notification can arrive -- and the first one
            // can arrive before this function returns. `TransportSender` needs the transport,
            // not a live connection; a request made before there is one fails, which is the
            // same answer it would give during an outage.
            let to_engine: Arc<dyn crate::application::ports::request_sender::RequestSender> =
                Arc::new(TransportSender::new(transport.clone()));

            // `git/onStatusUpdate` is applied to the projection **and then** forwarded. The
            // interface reads git state back out of the projection, so forwarding first would
            // have it read the state the update was about to replace -- right often enough to
            // look like a rare glitch rather than a race (see `git_notification.rs`).
            let git = Arc::new(
                crate::application::use_cases::apply_git_status::ApplyGitStatus::new(
                    Arc::new(
                        crate::adapters::outbound::remote_git::RemoteGitProvider::new(
                            to_engine.clone(),
                        ),
                    ),
                    ready.get(),
                ),
            );
            apply_git = Some(git.clone());
            let notifications: Arc<dyn NotificationSink> = Arc::new(
                crate::adapters::inbound::git_notification::GitNotifications::new(
                    git,
                    notifications,
                ),
            );

            // Before connecting, so the reader thread starts with somewhere to put the first
            // frame. Set afterwards, a task started immediately would produce output the
            // transport dropped -- the original defect, reintroduced as a race.
            transport.set_notification_sink(notifications);
            // FR-005: refused at startup rather than discovered at the first failure, so
            // "ssh is too old" and "the host refused you" are never confused.
            match transport.preflight() {
                Ok(banner) => {
                    crate::logging::info(&format!("engine transport ready: {banner}"));
                    deployer = Some(Arc::new(SshStreamDeployer::default()));
                    // The connection the whole client has been missing. Until this line
                    // `RemoteTasks` was written, tested and impossible to construct.
                    if let Err(e) = transport.connect() {
                        crate::logging::warn(&format!("the engine did not start: {e}"));
                    } else {
                        tasks = Some(Arc::new(RemoteTasks::new(to_engine.clone())));
                        sender = Some(to_engine);
                    }
                    transport
                }
                Err(e) => {
                    crate::logging::warn(&format!(
                        "cannot reach an engine: {e}; running unconnected"
                    ));
                    stub.clone()
                }
            }
        }
        None => stub.clone(),
    };
    let connection = Arc::new(ObserveConnection::new(source.clone()));
    let source_for_reconcile = source.clone();
    let current_workspace = std::sync::Arc::new(std::sync::Mutex::new(None));

    // The engine-backed provider, when there is an engine to back it.
    //
    // This was `DisconnectedWorkspace` unconditionally until F006, under a comment reading "no
    // engine-backed provider until a transport exists" -- written when that was true and left
    // standing after F010 built one. The consequence was not a crash: every workspace read and
    // every write answered `Offline` from the caching layer, which is indistinguishable from a
    // real outage and is what an outage is supposed to look like. `RemoteWorkspaceProvider` was
    // constructed nowhere in the application. The same shape as the defect that left four
    // features marked complete without a request ever crossing the seam, and found the same
    // way: by following what the feature under construction actually needs to reach.
    //
    // `None` for the bulk transport, unchanged: nothing implements `BulkTransfer` yet, and the
    // remote base it would need is a property of a workspace rather than of the process, so it
    // arrives with the workspace rather than here (A-BULK).
    let inner: Arc<dyn WorkspaceProvider> = match sender.as_ref() {
        Some(transport) => Arc::new(
            crate::adapters::outbound::remote_workspace::RemoteWorkspaceProvider::new(
                transport.clone(),
                None,
                String::new(),
            ),
        ),
        // Still the truth when no host is configured: nothing is reachable, the projection
        // serves what it holds, and the rest reports offline.
        None => Arc::new(DisconnectedWorkspace),
    };
    let provider: Arc<dyn crate::application::ports::workspace_provider::WorkspaceProvider> =
        Arc::new(CachedWorkspace::new(
            inner,
            ready.get(),
            Arc::new(SystemClock),
            source,
            Arc::new(|_presentation| {}),
            Limits::default(),
        ));

    // ---- F012: reconcile the work the host has not seen ----
    //
    // Built before `WorkspaceAccess` because the workspace commands need it too: `workspace_open`
    // and `workspace_resume` are the second trigger (see below). `DiffyMerge` is constructed
    // **here** and injected as `Reconcile`'s `TextMerge` -- the only place that hands the adapter to
    // anything, so without it the reconciler would have a port and no implementation. Naming the
    // *type* here does not breach `tests/merge_confinement.rs`: that rule is about the `diffy`
    // crate, and `engine/src/main.rs` names `inotify_watcher::git_watch()` in its composition root.
    let reconcile = Arc::new(crate::application::use_cases::reconcile::Reconcile::new(
        ready.get(),
        provider.clone(),
        Arc::new(crate::adapters::outbound::text_merge::DiffyMerge::new()),
    ));

    let workspace = WorkspaceAccess {
        cache: ready.get(),
        git: apply_git,
        register: Arc::new(RegisterWorkspace::new(ready.get(), Arc::new(SystemClock))),
        provider,
        reconcile: reconcile.clone(),
    };

    #[cfg(debug_assertions)]
    let shell = Shell {
        persist,
        connection,
        window,
        rail: rail.clone(),
        stub: stub.clone(),
    };
    #[cfg(not(debug_assertions))]
    let shell = Shell {
        persist,
        connection,
        window,
        rail,
    };

    // ---- F012: the first reconciliation trigger, a transition into Connected ----
    //
    // **Driven by the published connection state, not by a timer and not by the first successful
    // request** (A-RECONNECT). A timer would be a second detector of a state that is already
    // published, which is the defect F011 paid for when the git watch and the workspace watch
    // answered different questions about one repository. The first-successful-request trigger was
    // rejected in research.md for a different reason: it makes reconciliation a side effect of
    // whatever the developer happened to do next.
    {
        let reconcile = reconcile.clone();
        let current = current_workspace.clone();
        // `StateSink` is a `Fn`, so the memory of the last state needs interior mutability. A
        // mutex rather than a `Cell`, because the sink must be `Send + Sync`.
        let previous = std::sync::Mutex::new(ConnectionState::Unknown);
        source_for_reconcile.subscribe(Box::new(move |state| {
            // Once per *transition into* Connected. The sink is invoked with the current state on
            // subscribe and on every change, so without this comparison a reconnection that
            // reported `Connected` twice would reconcile twice -- reading every file again and
            // writing nothing, which is harmless and still wrong.
            let entered = {
                let Ok(mut last) = previous.lock() else {
                    return;
                };
                let entered =
                    state == ConnectionState::Connected && *last != ConnectionState::Connected;
                *last = state;
                entered
            };
            if !entered {
                return;
            }
            let Some(ws) = current
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .map(crate::domain::workspace::WorkspaceId)
            else {
                // No workspace open: nothing to reconcile, and nothing to say about it.
                return;
            };
            let reconcile = reconcile.clone();
            // A thread with its own runtime rather than `tokio::spawn`, for the reason
            // `SshTransport::subscribe` gives: this is reached from the composition root during
            // startup, which is not guaranteed to be inside a runtime, and a `spawn` that panicked
            // there would take the connection reporting down with it.
            std::thread::spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    crate::logging::warn("could not reconcile offline work: no runtime");
                    return;
                };
                rt.block_on(async move {
                    let report = reconcile.run(&ws).await;
                    // Logged per file, never with content: a file being edited may hold a
                    // credential, which is the reason F006 gives for the same rule.
                    for (path, outcome) in &report.files {
                        crate::logging::info(&format!("reconcile {}: {outcome:?}", path.as_str()));
                    }
                });
            });
        }));
    }

    Wiring {
        shell,
        current_workspace,
        workspace,
        stub,
        deployer,
        tasks,
        sender,
    }
}

/// The inner provider before a transport exists.
///
/// Every method reports `Offline`, which is the truth: nothing is reachable. The caching layer
/// above it still serves whatever the projection holds and reports the rest as unavailable —
/// the same behaviour as a real outage, which is why this is a provider rather than an
/// `Option` that every call site would have to unwrap.
struct DisconnectedWorkspace;

#[async_trait]
impl WorkspaceProvider for DisconnectedWorkspace {
    async fn read_directory(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _page: PageRequest,
    ) -> ProviderResult<DirPage> {
        Err(ProviderError::Offline)
    }

    async fn stat(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<FsMeta> {
        Err(ProviderError::Offline)
    }

    async fn read_file(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        Err(ProviderError::Offline)
    }

    /// Overridden so a save with no engine reads as an outage and not as unfinished work.
    ///
    /// The trait's default would answer `Unsupported { F006Editor }`, which was true until F006
    /// shipped and is a lie afterwards: it would tell a developer whose link had dropped that
    /// saving is not built yet, and they would stop trying instead of reconnecting.
    async fn write_file(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _content: &[u8],
        _base: &Sha256,
    ) -> ProviderResult<Sha256> {
        Err(ProviderError::Offline)
    }
}
