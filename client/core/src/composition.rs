//! Composition root. The only place adapters are bound to use cases.
//!
//! Constitution Principle VIII: wiring lives here, not scattered behind hidden globals or a
//! service locator. Swapping the stub connection source for the real transport in F001 is a
//! one-line change in this file.

use crate::adapters::inbound::tauri_commands::{Shell, WorkspaceAccess};
use crate::adapters::outbound::deploy::SshStreamDeployer;
use crate::adapters::outbound::json_session_store::JsonFileSessionStore;
use crate::adapters::outbound::openssh::{OpenSshSpawner, SshTransport};
use crate::adapters::outbound::stub_connection::StubConnectionStatusSource;
use crate::adapters::outbound::system_clock::SystemClock;
use crate::application::ports::connection::ConnectionStatusSource;
use crate::application::ports::session_store::SessionStore;
use crate::application::ports::spawner::SpawnSpec;
use crate::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use crate::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
use crate::application::use_cases::restore_session::RestoreSession;
use crate::composition_workspace::prepare_cache;
use crate::domain::rail::RailCatalogue;
use crate::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, PageRequest, RelPath, WorkspaceId,
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

pub fn build(data_dir: PathBuf, window: Arc<WindowController>) -> Wiring {
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
    let source: Arc<dyn ConnectionStatusSource> = match remote_target() {
        Some(spec) => {
            crate::logging::info(&format!("connecting to {}@{}", spec.user, spec.host));
            let transport = Arc::new(SshTransport::new(Arc::new(OpenSshSpawner::default()), spec));
            // FR-005: refused at startup rather than discovered at the first failure, so
            // "ssh is too old" and "the host refused you" are never confused.
            match transport.preflight() {
                Ok(banner) => {
                    crate::logging::info(&format!("local ssh client: {banner}"));
                    deployer = Some(Arc::new(SshStreamDeployer::default()));
                    transport
                }
                Err(e) => {
                    crate::logging::warn(&format!("cannot use ssh: {e}; running unconnected"));
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

    // No engine-backed provider until a transport exists. `FakeWorkspace` is a test double and
    // does not belong here, so the inner provider is one that refuses: the caching layer then
    // serves what is already cached and reports the rest as offline, which is exactly what it
    // does during a real outage.
    let inner: Arc<dyn WorkspaceProvider> = Arc::new(DisconnectedWorkspace);
    let workspace = WorkspaceAccess {
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
}
