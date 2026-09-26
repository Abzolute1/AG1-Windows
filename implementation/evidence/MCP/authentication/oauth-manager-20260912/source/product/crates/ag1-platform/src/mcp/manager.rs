//! Native discovery/execution owner, deliberately not registered with model/renderer dispatch.
//! One Store is moved into one manager. Runtime leases are never deserialized.
use super::{
    Revision,
    catalog::{CatalogBinding, CatalogScope, Tool},
    definitions::{Availability, Configuration, Draft, MAX_SERVERS, validate_scope},
    discovery::{CatalogKind, DiscoveryLimits},
    http::{Endpoint, HttpDiscovery},
    invalid,
    stdio::StdioDiscovery,
    wire::{Limits, decode_json},
};
use crate::{
    process::{ProcessSpec, ProcessSupervisor},
    provider::http::{CancelSource, cancellation},
};
use ag1_core::DomainError;
use ag1_storage::Store;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

mod capability;
mod credentials;
#[cfg(target_os = "linux")]
mod oauth;
#[cfg(target_os = "linux")]
pub use oauth::{OAuthAttempt, OAuthEntry, OAuthReview};
pub use capability::{CapabilityMatch, CapabilityMatches, SelectedCapability};
pub use credentials::{PrivateEntry, PrivateRequest, PrivateReview};
mod execution;
mod prepared;
pub(crate) use prepared::{PreparedOperation, RuntimeCheck};
mod lease;
mod selection;
pub use lease::SessionLease;
pub use selection::{SelectedTool, Selection, ToolMatch, ToolMatches};

const MAX_CONNECTIONS: usize = 4;
const MAX_REVIEWS: usize = 32;
const CATALOG_BYTES: usize = 16 * 1024 * 1024;
static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);
struct ConnectionBudget;
impl ConnectionBudget {
    fn acquire() -> Result<Self, DomainError> {
        CONNECTIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CONNECTIONS).then_some(n + 1)
            })
            .map_err(|_| DomainError::LimitExceeded("MCP active connections".into()))?;
        Ok(Self)
    }
}
impl Drop for ConnectionBudget {
    fn drop(&mut self) {
        CONNECTIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRecord {
    pub id: String,
    /// Management CAS revision: configuration or availability changed.
    pub revision: u64,
    /// Separate connection identity keeps unrelated allowed scopes alive when
    /// availability changes. Zero imports the legacy record's revision.
    #[serde(default)]
    config_revision: u64,
    pub draft: Draft,
    pub availability: Availability,
}
impl ServerRecord {
    pub fn configuration_revision(&self) -> u64 {
        if self.config_revision == 0 {
            self.revision
        } else {
            self.config_revision
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Records {
    servers: BTreeMap<String, ServerRecord>,
}
struct State {
    revision: u64,
    records: Records,
    live: BTreeMap<String, Worker>,
    poisoned: bool,
    recent: VecDeque<ConnectionStatus>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionHealth {
    Connecting,
    /// Discovery succeeded. For HTTP this is the last successful observation,
    /// not a promise of continued remote liveness or authority for a future call.
    Ready,
    Stopping,
    Stopped,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    PrivateValueEcho,
    Cancelled,
    Timeout,
    Unavailable,
    InvalidPeer,
    StartFailed,
    WorkerFailed,
    PermissionDenied,
    StaleIdentity,
    TransportFailed,
}
#[derive(Clone, Debug)]
pub struct ConnectionStatus {
    pub id: String,
    pub server_id: String,
    pub scope: CatalogScope,
    pub health: ConnectionHealth,
    pub inspection_only: bool,
    pub diagnostic: Option<Diagnostic>,
}
/// Private catalog data: validated metadata, never execution authority. Consumers
/// must obtain it through catalog() at each native request-preparation boundary.
#[derive(Debug)]
pub struct ConnectedCatalog {
    pub binding: CatalogBinding,
    pub server: Value,
    pub tools: Vec<Tool>,
    pub resources: Vec<Value>,
    pub templates: Vec<Value>,
    pub prompts: Vec<Value>,
    pub encoded_bytes: usize,
}
struct Control {
    credential: Option<Arc<super::auth::CredentialLease>>,
    lease: Option<Arc<lease::Token>>,
    cancelled: AtomicBool,
    send_gate: Mutex<()>,
    busy: AtomicBool,
    http_cancel: CancelSource,
    view: Mutex<View>,
}
struct View {
    status: ConnectionStatus,
    catalog: Option<Arc<ConnectedCatalog>>,
}
impl Control {
    fn private_result(&self, result: Result<Value, DomainError>) -> Result<Value, DomainError> {
        let value = result?;
        if let Some(credential) = &self.credential {
            credential.reject_echo(&value)?;
        }
        Ok(value)
    }
    fn credential_check(&self) -> Result<(), DomainError> {
        if let Some(credential) = &self.credential {
            credential.check(credentials::now()?)?;
        }
        Ok(())
    }
    fn cancel(&self) {
        let _gate = self.send_gate.lock().unwrap_or_else(|p| p.into_inner());
        self.cancelled.store(true, Ordering::Release);
        self.http_cancel.cancel();
        if let Ok(mut view) = self.view.lock() {
            view.catalog = None;
            if !matches!(
                view.status.health,
                ConnectionHealth::Stopped | ConnectionHealth::Failed
            ) {
                view.status.health = ConnectionHealth::Stopping;
            }
        }
    }
}
struct Worker {
    sender: std::sync::mpsc::SyncSender<execution::Command>,
    control: Arc<Control>,
    join: Option<JoinHandle<()>>,
}
impl Worker {
    fn join(&mut self) -> Result<(), DomainError> {
        if self.join.take().is_some_and(|j| j.join().is_err()) {
            let mut view = self
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            view.catalog = None;
            view.status.health = ConnectionHealth::Failed;
            view.status.diagnostic = Some(Diagnostic::WorkerFailed);
            return Err(DomainError::Unavailable("MCP discovery worker".into()));
        }
        Ok(())
    }
}

/// A native review witness. No Clone/Deserialize/Serialize; consuming it binds the
/// exact server, config/grant revision, scope, manager owner and short expiry.
/// Its public projection contains only the saved public configuration and launch
/// identity. It neither enables scope availability nor grants any tool call.
pub struct ConnectionReview {
    credential: Option<Arc<super::auth::CredentialLease>>,
    lease: Option<Arc<lease::Token>>,
    owner: Arc<()>,
    pending: Arc<AtomicUsize>,
    record: ServerRecord,
    scope: CatalogScope,
    expires: Instant,
    inspection_only: bool,
    local: Option<PinnedLocal>,
}
impl ConnectionReview {
    pub fn record(&self) -> &ServerRecord {
        &self.record
    }
    pub fn scope(&self) -> &CatalogScope {
        &self.scope
    }
    pub fn executable_sha256(&self) -> Option<&str> {
        self.local.as_ref().map(|l| l.executable_hash.as_str())
    }
}
impl Drop for ConnectionReview {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionConsent {
    RemoteEndpoint,
    LoopbackDevelopmentEndpoint,
    /// Explicit host-account code execution. Workspace availability is NOT an
    /// OS sandbox. Dependencies/argument-referenced files are mutable.
    LocalHostAccountExecution,
}
struct PinnedLocal {
    executable: File,
    cwd: File,
    executable_hash: String,
    executable_identity: (u64, u64),
    cwd_identity: (u64, u64),
}
fn identity(file: &File) -> Result<(u64, u64), DomainError> {
    let m = file
        .metadata()
        .map_err(|_| invalid("local target identity"))?;
    Ok((m.dev(), m.ino()))
}
fn executable_hash(file: &File) -> Result<String, DomainError> {
    use std::os::unix::fs::FileExt;
    let size = file
        .metadata()
        .map_err(|_| invalid("executable metadata"))?
        .len();
    if size > 128 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("MCP executable bytes".into()));
    }
    let mut hash = Sha256::new();
    let mut offset = 0;
    let mut chunk = [0u8; 64 * 1024];
    while offset < size {
        let limit = (size - offset).min(chunk.len() as u64) as usize;
        let n = file
            .read_at(&mut chunk[..limit], offset)
            .map_err(|_| invalid("executable read"))?;
        if n == 0 {
            return Err(DomainError::Conflict);
        }
        hash.update(&chunk[..n]);
        offset += n as u64;
    }
    if file
        .metadata()
        .map_err(|_| invalid("executable metadata"))?
        .len()
        != size
    {
        return Err(DomainError::Conflict);
    }
    Ok(format!("{:x}", hash.finalize()))
}
impl PinnedLocal {
    fn new(executable: &str, cwd: &str) -> Result<Self, DomainError> {
        use std::os::unix::fs::OpenOptionsExt;
        // Refuse non-regular targets without blocking on a FIFO/device open.
        let executable = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(executable)
            .map_err(|_| DomainError::NotFound)?;
        let cwd = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(cwd)
            .map_err(|_| DomainError::NotFound)?;
        if !executable
            .metadata()
            .map_err(|_| DomainError::NotFound)?
            .is_file()
            || !cwd.metadata().map_err(|_| DomainError::NotFound)?.is_dir()
        {
            return Err(invalid("local target type"));
        }
        Ok(Self {
            executable_identity: identity(&executable)?,
            cwd_identity: identity(&cwd)?,
            executable_hash: executable_hash(&executable)?,
            executable,
            cwd,
        })
    }
    fn revalidate(&self, executable: &str, cwd: &str) -> Result<(), DomainError> {
        let current = Self::new(executable, cwd)?;
        if current.executable_identity != self.executable_identity
            || current.cwd_identity != self.cwd_identity
            || current.executable_hash != self.executable_hash
            || executable_hash(&self.executable)? != self.executable_hash
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

pub struct Manager {
    vault: Option<Arc<super::auth::Vault>>,
    // Store move + Store's writer lock prevents two live managers for this state.
    store: Store,
    state: Mutex<State>,
    owner: Arc<()>,
    reviews: Arc<AtomicUsize>,
    helper: PathBuf,
    limits: DiscoveryLimits,
}
pub(super) fn new_id() -> Result<String, DomainError> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| DomainError::Unavailable("MCP native identity entropy".into()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
impl Manager {
    /// No process, DNS, endpoint, credential or auto-reconnect effects on open.
    /// Native composition should move its dedicated user MCP Store here once.
    pub fn open(store: Store, helper: &Path, limits: DiscoveryLimits) -> Result<Self, DomainError> {
        let saved = store.mcp_state().map_err(DomainError::from)?;
        let (revision, records) = if let Some(saved) = saved {
            let value = decode_json(&saved.bytes, Limits::default())?;
            let records: Records =
                serde_json::from_value(value).map_err(|_| invalid("stored definitions"))?;
            if records.servers.len() > MAX_SERVERS {
                return Err(invalid("stored server count"));
            }
            for (key, record) in &records.servers {
                if key != &record.id
                    || record.revision == 0
                    || record.config_revision > record.revision
                {
                    return Err(invalid("stored server identity"));
                }
                record.draft.validate()?;
                record.availability.validate()?;
                super::catalog::CatalogSlot::new(binding(record, CatalogScope::GeneralChat))?;
            }
            (saved.revision, records)
        } else {
            (0, Records::default())
        };
        Ok(Self {
            vault: None,
            store,
            state: Mutex::new(State {
                revision,
                records,
                live: BTreeMap::new(),
                poisoned: false,
                recent: VecDeque::new(),
            }),
            owner: Arc::new(()),
            reviews: Arc::new(AtomicUsize::new(0)),
            helper: helper.into(),
            limits,
        })
    }
    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>, DomainError> {
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        if state.poisoned {
            return Err(DomainError::Unavailable(
                "MCP state requires reopen after storage failure".into(),
            ));
        }
        Ok(state)
    }
    fn commit(&self, state: &mut State, next: Records) -> Result<(), DomainError> {
        let bytes = serde_json::to_vec(&next).map_err(|_| invalid("definition encoding"))?;
        // Apply the same bounded decoder on writes and reopen.
        decode_json(&bytes, Limits::default())?;
        match self.store.commit_mcp_state(state.revision, &bytes) {
            Ok(revision) => {
                state.revision = revision;
                state.records = next;
                Ok(())
            }
            Err(_) => {
                state.poisoned = true;
                for worker in state.live.values() {
                    worker.control.cancel();
                }
                Err(DomainError::Unavailable(
                    "MCP state commit failed; reopen required".into(),
                ))
            }
        }
    }
    pub fn records(&self) -> Result<Vec<ServerRecord>, DomainError> {
        Ok(self.state()?.records.servers.values().cloned().collect())
    }
    /// Atomically save a batch of manual/imported drafts, always disabled.
    pub fn add_drafts(&self, drafts: Vec<Draft>) -> Result<Vec<String>, DomainError> {
        if drafts.is_empty() || drafts.len() > 32 {
            return Err(invalid("draft count"));
        }
        for draft in &drafts {
            draft.validate()?;
        }
        let mut state = self.state()?;
        if state.records.servers.len() + drafts.len() > MAX_SERVERS {
            return Err(DomainError::LimitExceeded("MCP configured servers".into()));
        }
        let mut next = state.records.clone();
        let mut ids = Vec::new();
        for draft in drafts {
            let id = new_id()?;
            if next.servers.contains_key(&id) {
                return Err(DomainError::Conflict);
            }
            next.servers.insert(
                id.clone(),
                ServerRecord {
                    id: id.clone(),
                    revision: 1,
                    config_revision: 1,
                    draft,
                    availability: Availability::default(),
                },
            );
            ids.push(id);
        }
        self.commit(&mut state, next)?;
        Ok(ids)
    }
    /// Configuration edits invalidate all connections and grants. Availability
    /// changes revoke denied scopes while preserving unrelated allowed leases.
    pub fn edit(&self, id: &str, expected: u64, draft: Draft) -> Result<(), DomainError> {
        draft.validate()?;
        self.change(
            id,
            expected,
            Some(draft),
            Some(Availability::default()),
            false,
        )
    }
    pub fn set_availability(
        &self,
        id: &str,
        expected: u64,
        availability: Availability,
    ) -> Result<(), DomainError> {
        availability.validate()?;
        self.change(id, expected, None, Some(availability), false)
    }
    pub fn remove(&self, id: &str, expected: u64) -> Result<(), DomainError> {
        self.change(id, expected, None, None, true)
    }
    fn change(
        &self,
        id: &str,
        expected: u64,
        draft: Option<Draft>,
        availability: Option<Availability>,
        remove: bool,
    ) -> Result<(), DomainError> {
        let mut state = self.state()?;
        let mut next = state.records.clone();
        let record = next.servers.get_mut(id).ok_or(DomainError::NotFound)?;
        if record.revision != expected {
            return Err(DomainError::Conflict);
        }
        record.config_revision = record.configuration_revision();
        let had_credentials = record.draft.auth_profile.is_some();
        record.revision = expected.checked_add(1).ok_or(DomainError::Conflict)?;
        let configuration_changed = draft.is_some();
        if let Some(draft) = draft {
            record.draft = draft;
            record.config_revision = record.revision;
        }
        if let Some(availability) = availability {
            record.availability = availability;
        }
        if remove {
            next.servers.remove(id);
        }
        // Block publication/cancel before persistence. Failure leaves the entire
        // owner retired, so late work cannot revive permissive state.
        for worker in state.live.values() {
            let status = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .status
                .clone();
            if status.server_id == id
                && (remove
                    || configuration_changed
                    || status.inspection_only
                    || !next
                        .servers
                        .get(id)
                        .is_some_and(|r| r.availability.allows(&status.scope)))
            {
                worker.control.cancel();
            }
        }
        if had_credentials && (remove || configuration_changed) {
            self.revoke_credentials(id)?;
        }
        self.commit(&mut state, next)
    }
    pub fn review_connection(
        &self,
        id: &str,
        scope: CatalogScope,
    ) -> Result<ConnectionReview, DomainError> {
        self.review(id, scope, false)
    }
    /// Explicit native connection test, independent of model availability.
    pub fn review_test_connection(
        &self,
        id: &str,
        scope: CatalogScope,
    ) -> Result<ConnectionReview, DomainError> {
        self.review(id, scope, true)
    }
    fn review(
        &self,
        id: &str,
        scope: CatalogScope,
        inspection_only: bool,
    ) -> Result<ConnectionReview, DomainError> {
        validate_scope(&scope)?;
        let record = self
            .state()?
            .records
            .servers
            .get(id)
            .cloned()
            .ok_or(DomainError::NotFound)?;
        if !scope_admitted(&record.availability, &scope, inspection_only) {
            return Err(DomainError::PermissionDenied);
        }
        let local = match &record.draft.configuration {
            Configuration::Local {
                executable, cwd, ..
            } => Some(PinnedLocal::new(executable, cwd)?),
            Configuration::Http { .. } => None,
        };
        let credential = self.credential(&record, local.as_ref())?;
        self.reviews
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_REVIEWS).then_some(n + 1)
            })
            .map_err(|_| DomainError::LimitExceeded("MCP pending reviews".into()))?;
        Ok(ConnectionReview {
            credential,
            lease: None,
            owner: self.owner.clone(),
            pending: self.reviews.clone(),
            record,
            scope,
            expires: Instant::now() + Duration::from_secs(60),
            inspection_only,
            local,
        })
    }
    /// Starts one owned discovery worker. The witness is consumed on failure too.
    /// There is no tool-call, arbitrary method, header or credential argument.
    pub fn connect(
        &self,
        mut review: ConnectionReview,
        consent: ConnectionConsent,
    ) -> Result<String, DomainError> {
        if !Arc::ptr_eq(&self.owner, &review.owner) || Instant::now() >= review.expires {
            return Err(DomainError::PermissionDenied);
        }
        let required = match &review.record.draft.configuration {
            Configuration::Local {
                executable, cwd, ..
            } => {
                review
                    .local
                    .as_ref()
                    .ok_or(DomainError::Conflict)?
                    .revalidate(executable, cwd)?;
                ConnectionConsent::LocalHostAccountExecution
            }
            Configuration::Http {
                loopback_development: true,
                ..
            } => ConnectionConsent::LoopbackDevelopmentEndpoint,
            Configuration::Http { .. } => ConnectionConsent::RemoteEndpoint,
        };
        if required != consent || Instant::now() >= review.expires {
            return Err(DomainError::PermissionDenied);
        }
        self.reap_finished()?;
        let mut state = self.state()?;
        if review
            .lease
            .as_ref()
            .is_some_and(|lease| lease.closed.load(Ordering::Acquire))
        {
            return Err(DomainError::PermissionDenied);
        }
        let record = state
            .records
            .servers
            .get(&review.record.id)
            .ok_or(DomainError::NotFound)?;
        if record.revision != review.record.revision
            || !scope_admitted(&record.availability, &review.scope, review.inspection_only)
        {
            return Err(DomainError::Conflict);
        }
        if let Some(credential) = &review.credential {
            credential.check(credentials::now()?)?;
        }
        if state.live.len() >= MAX_CONNECTIONS {
            return Err(DomainError::LimitExceeded(
                "MCP connection slots; disconnect finished entries".into(),
            ));
        }
        if state.live.values().any(|w| {
            w.control.view.lock().is_ok_and(|v| {
                v.status.server_id == record.id
                    && v.status.scope == review.scope
                    && lease::same(w.control.lease.as_ref(), review.lease.as_ref())
                    && !matches!(
                        v.status.health,
                        ConnectionHealth::Stopped | ConnectionHealth::Failed
                    )
            })
        }) {
            return Err(DomainError::Conflict);
        }
        let budget = ConnectionBudget::acquire()?;
        let id = new_id()?;
        let (http_cancel, token) = cancellation();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let control = Arc::new(Control {
            credential: review.credential.clone(),
            lease: review.lease.clone(),
            cancelled: AtomicBool::new(false),
            send_gate: Mutex::new(()),
            busy: AtomicBool::new(false),
            http_cancel,
            view: Mutex::new(View {
                status: ConnectionStatus {
                    id: id.clone(),
                    server_id: record.id.clone(),
                    scope: review.scope.clone(),
                    health: ConnectionHealth::Connecting,
                    inspection_only: review.inspection_only,
                    diagnostic: None,
                },
                catalog: None,
            }),
        });
        let worker_control = control.clone();
        let record = record.clone();
        let scope = review.scope.clone();
        let local = review.local.take();
        let helper = self.helper.clone();
        let limits = self.limits;
        let join = thread::Builder::new()
            .name("ag1-mcp-discovery".into())
            .spawn(move || {
                let _budget = budget;
                let panic_control = worker_control.clone();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker(
                        record,
                        scope,
                        local,
                        &helper,
                        limits,
                        worker_control,
                        token,
                        receiver,
                    );
                }))
                .is_err()
                {
                    let mut view = panic_control.view.lock().unwrap_or_else(|p| p.into_inner());
                    view.catalog = None;
                    view.status.health = ConnectionHealth::Failed;
                    view.status.diagnostic = Some(Diagnostic::WorkerFailed);
                }
            })
            .map_err(|_| DomainError::Unavailable("MCP worker start".into()))?;
        state.live.insert(
            id.clone(),
            Worker {
                sender,
                control,
                join: Some(join),
            },
        );
        Ok(id)
    }
    pub fn status(&self, id: &str) -> Result<ConnectionStatus, DomainError> {
        let state = self.state()?;
        if let Some(worker) = state.live.get(id) {
            return Ok(worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .status
                .clone());
        }
        state
            .recent
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or(DomainError::NotFound)
    }
    pub fn catalog(
        &self,
        id: &str,
        scope: &CatalogScope,
    ) -> Result<Arc<ConnectedCatalog>, DomainError> {
        self.read_catalog(id, scope, false)
    }
    /// Native manager inspection only. This must not be exposed as a model
    /// discovery shortcut; catalog() enforces model availability separately.
    pub fn inspect_catalog(
        &self,
        id: &str,
        scope: &CatalogScope,
    ) -> Result<Arc<ConnectedCatalog>, DomainError> {
        self.read_catalog(id, scope, true)
    }
    fn read_catalog(
        &self,
        id: &str,
        scope: &CatalogScope,
        inspection: bool,
    ) -> Result<Arc<ConnectedCatalog>, DomainError> {
        let state = self.state()?;
        let worker = state.live.get(id).ok_or(DomainError::NotFound)?;
        worker.control.credential_check()?;
        let view = worker
            .control
            .view
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        let record = state
            .records
            .servers
            .get(&view.status.server_id)
            .ok_or(DomainError::NotFound)?;
        if &view.status.scope != scope
            || !scope_admitted(&record.availability, scope, inspection)
            || (!inspection && view.status.inspection_only)
            || worker.control.cancelled.load(Ordering::Acquire)
        {
            return Err(DomainError::PermissionDenied);
        }
        let catalog = view
            .catalog
            .as_ref()
            .ok_or(DomainError::Unavailable("MCP catalog not ready".into()))?;
        if record.configuration_revision() != catalog.binding.config_revision
            || view.status.health != ConnectionHealth::Ready
        {
            return Err(DomainError::Conflict);
        }
        Ok(catalog.clone())
    }
    /// Nonblocking revocation; wait_disconnected performs actual join on a native
    /// worker. Scope availability/configuration persist across a disconnect.
    pub fn disconnect(&self, id: &str) -> Result<(), DomainError> {
        let state = self.state()?;
        state
            .live
            .get(id)
            .ok_or(DomainError::NotFound)?
            .control
            .cancel();
        Ok(())
    }
    /// Blocking cleanup: call off the native UI/session owner thread.
    pub fn wait_disconnected(&self, id: &str) -> Result<(), DomainError> {
        let mut worker = {
            let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
            let worker = state.live.remove(id).ok_or(DomainError::NotFound)?;
            worker.control.cancel();
            worker
        };
        let result = worker.join();
        let status = worker
            .control
            .view
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .status
            .clone();
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        remember(&mut state, status);
        result
    }
    /// Bounded native metadata diagnostics. No peer error strings/payloads.
    pub fn recent_connections(&self) -> Result<Vec<ConnectionStatus>, DomainError> {
        self.reap_finished()?;
        let state = self.state()?;
        let mut statuses: Vec<_> = state.recent.iter().cloned().collect();
        for worker in state.live.values() {
            statuses.push(
                worker
                    .control
                    .view
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .status
                    .clone(),
            );
        }
        Ok(statuses)
    }
    fn reap_finished(&self) -> Result<(), DomainError> {
        let mut state = self.state()?;
        let finished: Vec<_> = state
            .live
            .iter()
            .filter(|(_, w)| w.join.as_ref().is_none_or(|j| j.is_finished()))
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            if let Some(mut worker) = state.live.remove(&id) {
                let result = worker.join();
                let status = worker
                    .control
                    .view
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .status
                    .clone();
                remember(&mut state, status);
                result?;
            }
        }
        Ok(())
    }
}
fn scope_admitted(availability: &Availability, scope: &CatalogScope, inspection: bool) -> bool {
    if matches!(scope, CatalogScope::Workspace(id) if availability.denied_workspaces.contains(id)) {
        return false;
    }
    inspection || availability.allows(scope)
}
fn remember(state: &mut State, status: ConnectionStatus) {
    if state.recent.len() == 128 {
        state.recent.pop_front();
    }
    state.recent.push_back(status);
}
impl Drop for Manager {
    fn drop(&mut self) {
        // Cancellation fans out first so shutdown latency does not multiply by
        // each operation deadline. Join occurs only in the native owner teardown.
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        for worker in state.live.values() {
            worker.control.cancel();
        }
        for worker in state.live.values_mut() {
            let _ = worker.join();
        }
    }
}
fn binding(record: &ServerRecord, scope: CatalogScope) -> CatalogBinding {
    CatalogBinding {
        server_id: record.id.clone(),
        scope,
        config_revision: record.configuration_revision(),
        auth_profile: "mcp-unauthenticated".into(),
        credential_generation: 0,
        // Per-connection UUID is also carried by native status; this monotonically
        // different native catalog owner is not reused or persisted on restart.
        connection_generation: 1,
        protocol: Revision::parse(&record.draft.protocol).expect("validated definition"),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    record: ServerRecord,
    scope: CatalogScope,
    local: Option<PinnedLocal>,
    helper: &Path,
    limits: DiscoveryLimits,
    control: Arc<Control>,
    mut token: crate::provider::http::CancelToken,
    receiver: std::sync::mpsc::Receiver<execution::Command>,
) {
    let result = (|| -> Result<(), DomainError> {
        if control.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let mut binding = binding(&record, scope);
        control.credential_check()?;
        if let Some(credential) = &control.credential {
            binding.auth_profile = credential.binding().profile_id.clone();
            binding.credential_generation = credential.generation();
        }
        // Random native connection identity, independent of display name and
        // persisted configuration; never resurrected after restart.
        let mut generation = [0u8; 8];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut generation))
            .map_err(|_| DomainError::Unavailable("MCP entropy".into()))?;
        binding.connection_generation = u64::from_le_bytes(generation).max(1);
        match &record.draft.configuration {
            Configuration::Local {
                executable,
                arguments,
                cwd,
                public_environment,
                ..
            } => {
                let pins = local.ok_or(DomainError::Conflict)?;
                pins.revalidate(executable, cwd)?;
                if control.cancelled.load(Ordering::Acquire) {
                    return Err(DomainError::Cancelled);
                }
                let directory = tempfile::tempdir()
                    .map_err(|_| DomainError::Unavailable("MCP private scratch".into()))?;
                let supervisor = ProcessSupervisor::new(helper)?;
                // Held descriptors keep the reviewed executable and cwd identity
                // across path replacement. This does not freeze dependencies or
                // provide containment against deliberate in-place code mutation.
                let mut spec = ProcessSpec {
                    executable: format!(
                        "/proc/{}/fd/{}",
                        std::process::id(),
                        pins.executable.as_raw_fd()
                    )
                    .into(),
                    argv: arguments.clone(),
                    cwd: format!("/proc/{}/fd/{}", std::process::id(), pins.cwd.as_raw_fd()).into(),
                    env: public_environment.clone(),
                    timeout_ms: 600_000,
                    grace_ms: 100,
                    resident: false,
                };
                if let Some(credential) = &control.credential {
                    credential.with_material(credentials::now()?, |material| {
                        let super::auth::Material::Environment(values) = material else {
                            return Err(DomainError::PermissionDenied);
                        };
                        for (name, value) in values {
                            spec.env.insert(
                                name.clone(),
                                std::str::from_utf8(value.expose())
                                    .map_err(|_| invalid("private environment encoding"))?
                                    .into(),
                            );
                        }
                        Ok(())
                    })?;
                }
                control.credential_check()?;
                let process =
                    supervisor.spawn_mcp_protocol(spec, &directory.path().join("stderr"))?;
                let mut client = StdioDiscovery::from_owned(process, binding.clone(), limits)?;
                let discovered = discover_stdio(&mut client, &binding, &control);
                if let Ok(catalog) = discovered.as_ref() {
                    publish(&control, catalog.clone())?;
                }
                let mut operation_failure = None;
                if discovered.is_ok() {
                    while !control.cancelled.load(Ordering::Acquire) {
                        control.credential_check()?;
                        if !client.is_running()? {
                            return Err(DomainError::Unavailable("MCP child exited".into()));
                        }
                        match receiver.recv_timeout(Duration::from_millis(25)) {
                            Ok(command) => {
                                let generation = client.discovery_generation();
                                let result = control.private_result(client.call_selected(
                                    command.identity(),
                                    command.args(),
                                    &control.cancelled,
                                    &|| command.admit_send(&control),
                                ));
                                let retire = result.is_err()
                                    || client.discovery_generation() != generation
                                    || (command.identity().kind
                                        == ag1_core::tools::mcp::Kind::Tool
                                        && client.tools().is_none());
                                if retire {
                                    operation_failure = Some(
                                        result
                                            .as_ref()
                                            .err()
                                            .cloned()
                                            .unwrap_or(DomainError::Conflict),
                                    );
                                    retire_catalog(&control)?;
                                }
                                let _ = command.reply.send(result);
                                if retire {
                                    break;
                                }
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        }
                    }
                }
                let shutdown = client.shutdown();
                discovered?;
                if let Some(error) = operation_failure {
                    return Err(error);
                }
                shutdown?;
            }
            Configuration::Http {
                url,
                loopback_development,
            } => {
                let endpoint = if *loopback_development {
                    Endpoint::loopback_development(url)?
                } else {
                    Endpoint::public_https(url)?
                };
                let mut client = HttpDiscovery::new(endpoint, binding.clone(), limits)?;
                if let Some(credential) = &control.credential {
                    client = client.with_credential(credential.clone())?;
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| DomainError::Unavailable("MCP HTTP worker runtime".into()))?;
                runtime.block_on(async {
                    let discovered =
                        discover_http(&mut client, &binding, &control, &mut token).await;
                    if let Ok(catalog) = discovered.as_ref() {
                        publish(&control, catalog.clone())?;
                    }
                    let mut operation_failure = None;
                    if discovered.is_ok() {
                        while !control.cancelled.load(Ordering::Acquire) {
                            control.credential_check()?;
                            match receiver.try_recv() {
                                Ok(command) => {
                                    let generation = client.discovery_generation();
                                    let result = control.private_result(
                                        client
                                            .call_selected(
                                                command.identity(),
                                                command.args(),
                                                &mut token,
                                                &|| command.admit_send(&control),
                                            )
                                            .await,
                                    );
                                    let retire = result.is_err()
                                        || client.discovery_generation() != generation
                                        || (command.identity().kind
                                            == ag1_core::tools::mcp::Kind::Tool
                                            && client.tools().is_none());
                                    if retire {
                                        operation_failure = Some(
                                            result
                                                .as_ref()
                                                .err()
                                                .cloned()
                                                .unwrap_or(DomainError::Conflict),
                                        );
                                        retire_catalog(&control)?;
                                    }
                                    let _ = command.reply.send(result);
                                    if retire {
                                        break;
                                    }
                                }
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                    tokio::select! {
                                        _ = token.wait() => break,
                                        _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                                    }
                                }
                            }
                        }
                    }
                    let (_source, mut cleanup) = cancellation();
                    // Session deletion is best effort and bounded, not evidence
                    // that an independently running remote effect was cancelled.
                    let _ = tokio::time::timeout(
                        Duration::from_millis(500),
                        client.shutdown(&mut cleanup),
                    )
                    .await;
                    discovered?;
                    if let Some(error) = operation_failure {
                        return Err(error);
                    }
                    Ok(())
                })?;
            }
        }
        Ok(())
    })();
    if let Ok(mut view) = control.view.lock() {
        view.catalog = None;
        view.status.health = if result.is_ok() || control.cancelled.load(Ordering::Acquire) {
            ConnectionHealth::Stopped
        } else {
            ConnectionHealth::Failed
        };
        view.status.diagnostic = result.err().map(|e| match e {
            DomainError::Cancelled => Diagnostic::Cancelled,
            DomainError::TimedOut => Diagnostic::Timeout,
            DomainError::InvalidInput(_) => Diagnostic::InvalidPeer,
            DomainError::NotFound => Diagnostic::StartFailed,
            DomainError::PermissionDenied => Diagnostic::PermissionDenied,
            DomainError::Conflict => Diagnostic::StaleIdentity,
            DomainError::Io(_) => Diagnostic::TransportFailed,
            DomainError::Unavailable(reason) if reason == super::auth::PRIVATE_ECHO => {
                Diagnostic::PrivateValueEcho
            }
            _ => Diagnostic::Unavailable,
        });
    }
}
fn retire_catalog(control: &Control) -> Result<(), DomainError> {
    let mut view = control.view.lock().map_err(|_| DomainError::Conflict)?;
    view.catalog = None;
    view.status.health = ConnectionHealth::Failed;
    Ok(())
}
fn publish(control: &Control, catalog: Arc<ConnectedCatalog>) -> Result<(), DomainError> {
    control.credential_check()?;
    if let Some(credential) = &control.credential {
        credential.reject_echo(&catalog.server)?;
        for tool in &catalog.tools {
            credential.reject_echo(tool.original())?;
        }
        for value in catalog
            .resources
            .iter()
            .chain(&catalog.templates)
            .chain(&catalog.prompts)
        {
            credential.reject_echo(value)?;
        }
    }
    let mut view = control.view.lock().map_err(|_| DomainError::Conflict)?;
    if control.cancelled.load(Ordering::Acquire) {
        return Err(DomainError::Cancelled);
    }
    view.catalog = Some(catalog);
    view.status.health = ConnectionHealth::Ready;
    Ok(())
}
fn empty_catalog(binding: &CatalogBinding, server: Value) -> ConnectedCatalog {
    ConnectedCatalog {
        binding: binding.clone(),
        server,
        tools: vec![],
        resources: vec![],
        templates: vec![],
        prompts: vec![],
        encoded_bytes: 0,
    }
}
fn charge<'a>(
    catalog: &mut ConnectedCatalog,
    values: impl IntoIterator<Item = &'a Value>,
) -> Result<(), DomainError> {
    for value in values {
        catalog.encoded_bytes = catalog
            .encoded_bytes
            .checked_add(
                serde_json::to_vec(value)
                    .map_err(|_| invalid("catalog encoding"))?
                    .len(),
            )
            .ok_or(DomainError::Conflict)?;
        if catalog.encoded_bytes > CATALOG_BYTES {
            return Err(DomainError::LimitExceeded(
                "MCP aggregate connection catalog".into(),
            ));
        }
    }
    Ok(())
}
fn discover_stdio(
    client: &mut StdioDiscovery,
    binding: &CatalogBinding,
    control: &Control,
) -> Result<Arc<ConnectedCatalog>, DomainError> {
    client.negotiate(&control.cancelled)?;
    let generation = client.discovery_generation();
    let server = client
        .server_metadata()
        .cloned()
        .ok_or(DomainError::Conflict)?;
    let capabilities = server["capabilities"].clone();
    let mut catalog = empty_catalog(binding, server.clone());
    charge(&mut catalog, &[server])?;
    for kind in [
        CatalogKind::Resources,
        CatalogKind::ResourceTemplates,
        CatalogKind::Prompts,
    ] {
        if capabilities.get(kind.capability()).is_none() {
            continue;
        }
        let entries = client.list(kind, &control.cancelled)?.entries;
        charge(&mut catalog, &entries)?;
        match kind {
            CatalogKind::Resources => catalog.resources = entries,
            CatalogKind::ResourceTemplates => catalog.templates = entries,
            CatalogKind::Prompts => catalog.prompts = entries,
        }
    }
    if capabilities.get("tools").is_some() {
        client.refresh_tools(&control.cancelled)?;
        let tools: Vec<_> = client
            .tools()
            .ok_or(DomainError::Conflict)?
            .tools()
            .cloned()
            .collect();
        charge(&mut catalog, tools.iter().map(Tool::original))?;
        catalog.tools = tools;
    }
    // Tool-change notifications encountered while listing other kinds retire the
    // tool slot; never publish the earlier copied tool list in that case.
    if capabilities.get("tools").is_some() && client.tools().is_none() {
        return Err(DomainError::Conflict);
    }
    if client.discovery_generation() != generation {
        return Err(DomainError::Conflict);
    }
    Ok(Arc::new(catalog))
}
async fn discover_http(
    client: &mut HttpDiscovery,
    binding: &CatalogBinding,
    control: &Control,
    token: &mut crate::provider::http::CancelToken,
) -> Result<Arc<ConnectedCatalog>, DomainError> {
    client.negotiate(token).await?;
    let generation = client.discovery_generation();
    let server = client
        .server_metadata()
        .cloned()
        .ok_or(DomainError::Conflict)?;
    let capabilities = server["capabilities"].clone();
    let mut catalog = empty_catalog(binding, server.clone());
    charge(&mut catalog, &[server])?;
    for kind in [
        CatalogKind::Resources,
        CatalogKind::ResourceTemplates,
        CatalogKind::Prompts,
    ] {
        if capabilities.get(kind.capability()).is_none() {
            continue;
        }
        let entries = client.list(kind, token).await?.entries;
        charge(&mut catalog, &entries)?;
        match kind {
            CatalogKind::Resources => catalog.resources = entries,
            CatalogKind::ResourceTemplates => catalog.templates = entries,
            CatalogKind::Prompts => catalog.prompts = entries,
        }
    }
    if capabilities.get("tools").is_some() {
        client.refresh_tools(token).await?;
        let tools: Vec<_> = client
            .tools()
            .ok_or(DomainError::Conflict)?
            .tools()
            .cloned()
            .collect();
        charge(&mut catalog, tools.iter().map(Tool::original))?;
        catalog.tools = tools;
    }
    if control.cancelled.load(Ordering::Acquire) {
        return Err(DomainError::Cancelled);
    }
    if capabilities.get("tools").is_some() && client.tools().is_none() {
        return Err(DomainError::Conflict);
    }
    if client.discovery_generation() != generation {
        return Err(DomainError::Conflict);
    }
    Ok(Arc::new(catalog))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::definitions::Provenance;
    #[test]
    fn expired_review_cannot_create_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::open(
            Store::open(dir.path()).unwrap(),
            Path::new("/unused"),
            DiscoveryLimits::default(),
        )
        .unwrap();
        let id = manager
            .add_drafts(vec![Draft {
                name: "owned".into(),
                configuration: Configuration::Http {
                    url: "https://example.invalid/mcp".into(),
                    loopback_development: false,
                },
                protocol: Revision::Current20260728.as_str().into(),
                auth_profile: None,
                provenance: Provenance::Manual,
            }])
            .unwrap()
            .remove(0);
        manager
            .set_availability(
                &id,
                1,
                Availability {
                    enabled: true,
                    general_chat: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let mut review = manager
            .review_connection(&id, CatalogScope::GeneralChat)
            .unwrap();
        review.expires = Instant::now() - Duration::from_secs(1);
        assert!(matches!(
            manager.connect(review, ConnectionConsent::RemoteEndpoint),
            Err(DomainError::PermissionDenied)
        ));
        assert!(manager.recent_connections().unwrap().is_empty());
    }
}
