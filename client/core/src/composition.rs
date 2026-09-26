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
    let mut deployer: Option<Arc<SshStreamDeployer>> = None;
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
                        let to_engine: Arc<
                            dyn crate::application::ports::request_sender::RequestSender,
                        > = Arc::new(TransportSender::new(transport.clone()));
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
    let workspace = WorkspaceAccess {
        cache: ready.get(),
        register: Arc::new(RegisterWorkspace::new(ready.get(), Arc::new(SystemClock))),
        provider: Arc::new(CachedWorkspace::new(
            inner,
            ready.get(),
            Arc::new(SystemClock),
            source,
            Arc::new(|_presentation| {}),
            Limits::default(),
        )),
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

    Wiring {
        shell,
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
