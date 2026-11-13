//! Windows owner for deliberately reviewed local MCP servers.
//!
//! Windows has no equivalent of AG1's Bubblewrap read-only sandbox.  This
//! module therefore supports only the explicit host-account mode.  It keeps
//! configuration, review, process ownership and protocol discovery in one
//! place, and refuses the Linux-only access mode instead of weakening it.

use super::{
    Revision,
    catalog::{CatalogBinding, CatalogScope, CatalogSlot},
    definitions::{Availability, Configuration, Draft, LocalAccess, MAX_SERVERS, validate_scope},
    discovery::{CatalogKind, DiscoveryLimits},
    invalid,
    stdio::StdioDiscovery,
    wire::{Limits, decode_json},
};
use crate::process::{ProcessSpec, ProcessSupervisor};
use ag1_core::DomainError;
use ag1_storage::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_CONNECTIONS: usize = 4;
const MAX_REVIEWS: usize = 32;
const CATALOG_BYTES: usize = 16 * 1024 * 1024;
const MCP_PROCESS_MEMORY: u64 = 1024 * 1024 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The saved portion of a Windows MCP server.  Live process handles and
/// discovery data never enter storage, so a restart always needs a new review.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRecord {
    pub id: String,
    pub revision: u64,
    #[serde(default)]
    config_revision: u64,
    pub draft: Draft,
    pub availability: Availability,
    #[serde(default, skip_serializing_if = "LocalAccess::is_host_account")]
    pub local_access: LocalAccess,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionHealth {
    Connecting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    Cancelled,
    TimedOut,
    Unavailable,
    InvalidPeer,
    StartFailed,
    WorkerFailed,
    PermissionDenied,
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

/// A private live catalog.  It is only a validated description of what this
/// connection advertised; callers still need their own operation authority.
#[derive(Debug)]
pub struct ConnectedCatalog {
    pub binding: CatalogBinding,
    pub server: Value,
    pub tools: Vec<Value>,
    pub resources: Vec<Value>,
    pub templates: Vec<Value>,
    pub prompts: Vec<Value>,
    pub encoded_bytes: usize,
}

struct View {
    status: ConnectionStatus,
    catalog: Option<Arc<ConnectedCatalog>>,
}

struct Control {
    cancelled: AtomicBool,
    view: Mutex<View>,
}

impl Control {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(mut view) = self.view.lock()
            && !matches!(
                view.status.health,
                ConnectionHealth::Stopped | ConnectionHealth::Failed
            )
        {
            view.catalog = None;
            view.status.health = ConnectionHealth::Stopping;
        }
    }
}

struct Worker {
    control: Arc<Control>,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    fn join(&mut self) -> Result<(), DomainError> {
        if self.join.take().is_some_and(|join| join.join().is_err()) {
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

struct State {
    revision: u64,
    records: Records,
    live: BTreeMap<String, Worker>,
    recent: VecDeque<ConnectionStatus>,
    poisoned: bool,
}

/// A short-lived, non-serializable connection approval.  Keeping it linear
/// means a UI review cannot be replayed after an edit or a host restart.
pub struct ConnectionReview {
    owner: Arc<()>,
    pending: Arc<AtomicUsize>,
    record: ServerRecord,
    scope: CatalogScope,
    expires: Instant,
    inspection_only: bool,
    executable_sha256: String,
}

impl ConnectionReview {
    pub fn record(&self) -> &ServerRecord {
        &self.record
    }

    pub fn scope(&self) -> &CatalogScope {
        &self.scope
    }

    pub fn executable_sha256(&self) -> &str {
        &self.executable_sha256
    }
}

impl Drop for ConnectionReview {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionConsent {
    /// This starts the reviewed command with the user's Windows account.  It is
    /// deliberately not presented as a filesystem or network sandbox.
    LocalHostAccountExecution,
}

/// The Windows MCP owner.  It has a dedicated storage directory so the chat
/// journal can keep its own Store writer lease.
pub struct Manager {
    store: Store,
    state: Mutex<State>,
    owner: Arc<()>,
    reviews: Arc<AtomicUsize>,
    root: PathBuf,
    limits: DiscoveryLimits,
}

impl Manager {
    /// Opening reads configuration only.  It never launches, contacts or
    /// reconnects a server on the user's behalf.
    pub fn open(root: &Path, limits: DiscoveryLimits) -> Result<Self, DomainError> {
        fs::create_dir_all(root).map_err(crate::io_error)?;
        let store = Store::open(root).map_err(DomainError::from)?;
        let saved = store.mcp_state().map_err(DomainError::from)?;
        let (revision, records) = match saved {
            Some(saved) => {
                let value = decode_json(&saved.bytes, Limits::default())?;
                let records: Records =
                    serde_json::from_value(value).map_err(|_| invalid("stored definitions"))?;
                validate_records(&records)?;
                (saved.revision, records)
            }
            None => (0, Records::default()),
        };
        Ok(Self {
            store,
            state: Mutex::new(State {
                revision,
                records,
                live: BTreeMap::new(),
                recent: VecDeque::new(),
                poisoned: false,
            }),
            owner: Arc::new(()),
            reviews: Arc::new(AtomicUsize::new(0)),
            root: root.into(),
            limits,
        })
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>, DomainError> {
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        if state.poisoned {
            return Err(DomainError::Unavailable(
                "MCP state requires reopening after a storage failure".into(),
            ));
        }
        Ok(state)
    }

    fn commit(&self, state: &mut State, next: Records) -> Result<(), DomainError> {
        validate_records(&next)?;
        let bytes = serde_json::to_vec(&next).map_err(|_| invalid("definition encoding"))?;
        decode_json(&bytes, Limits::default())?;
        match self.store.commit_mcp_state(state.revision, &bytes) {
            Ok(revision) => {
                state.revision = revision;
                state.records = next;
                Ok(())
            }
            Err(_) => {
                // Once persistence is uncertain, stop live servers rather than
                // allowing a configuration that cannot be reconstructed.
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

    pub fn add_drafts_with_access(
        &self,
        drafts: Vec<Draft>,
        local_access: LocalAccess,
    ) -> Result<Vec<String>, DomainError> {
        if drafts.is_empty() || drafts.len() > 32 {
            return Err(invalid("draft count"));
        }
        for draft in &drafts {
            validate_windows_draft(draft, local_access)?;
        }
        let mut state = self.state()?;
        if state.records.servers.len() + drafts.len() > MAX_SERVERS {
            return Err(DomainError::LimitExceeded("MCP configured servers".into()));
        }
        let mut next = state.records.clone();
        let mut ids = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let id = new_id();
            next.servers.insert(
                id.clone(),
                ServerRecord {
                    id: id.clone(),
                    revision: 1,
                    config_revision: 1,
                    draft,
                    availability: Availability::default(),
                    local_access,
                },
            );
            ids.push(id);
        }
        self.commit(&mut state, next)?;
        Ok(ids)
    }

    pub fn edit_with_access(
        &self,
        id: &str,
        expected: u64,
        draft: Draft,
        access: LocalAccess,
    ) -> Result<(), DomainError> {
        validate_windows_draft(&draft, access)?;
        self.change(
            id,
            expected,
            Some(draft),
            Some(access),
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
        self.change(id, expected, None, None, Some(availability), false)
    }

    pub fn remove(&self, id: &str, expected: u64) -> Result<(), DomainError> {
        self.change(id, expected, None, None, None, true)
    }

    fn change(
        &self,
        id: &str,
        expected: u64,
        draft: Option<Draft>,
        access: Option<LocalAccess>,
        availability: Option<Availability>,
        remove: bool,
    ) -> Result<(), DomainError> {
        let mut state = self.state()?;
        let mut next = state.records.clone();
        let record = next.servers.get_mut(id).ok_or(DomainError::NotFound)?;
        if record.revision != expected {
            return Err(DomainError::Conflict);
        }

        let configuration_changed = draft.is_some() || access.is_some();
        record.config_revision = record.configuration_revision();
        record.revision = expected.checked_add(1).ok_or(DomainError::Conflict)?;
        if let Some(draft) = draft {
            record.draft = draft;
        }
        if let Some(access) = access {
            record.local_access = access;
        }
        if let Some(availability) = availability {
            record.availability = availability;
        }
        validate_windows_draft(&record.draft, record.local_access)?;

        // Cancel before the durable state changes.  A worker that wakes late
        // sees its cancellation flag and cannot publish an old catalog.
        for worker in state.live.values() {
            let status = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .status
                .clone();
            let unavailable_here = !next
                .servers
                .get(id)
                .is_some_and(|saved| saved.availability.allows(&status.scope));
            if status.server_id == id
                && (remove || configuration_changed || status.inspection_only || unavailable_here)
            {
                worker.control.cancel();
            }
        }
        if remove {
            next.servers.remove(id);
        }
        self.commit(&mut state, next)
    }

    pub fn review_connection(
        &self,
        id: &str,
        expected: u64,
        scope: CatalogScope,
    ) -> Result<ConnectionReview, DomainError> {
        self.review(id, expected, scope, false)
    }

    pub fn review_test_connection(
        &self,
        id: &str,
        expected: u64,
        scope: CatalogScope,
    ) -> Result<ConnectionReview, DomainError> {
        self.review(id, expected, scope, true)
    }

    fn review(
        &self,
        id: &str,
        expected: u64,
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
        if record.revision != expected {
            return Err(DomainError::Conflict);
        }
        if !inspection_only && !record.availability.allows(&scope) {
            return Err(DomainError::PermissionDenied);
        }
        validate_windows_draft(&record.draft, record.local_access)?;
        let executable_sha256 = local_digest(&record)?;

        self.reviews
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_REVIEWS).then_some(count + 1)
            })
            .map_err(|_| DomainError::LimitExceeded("MCP pending reviews".into()))?;
        Ok(ConnectionReview {
            owner: self.owner.clone(),
            pending: self.reviews.clone(),
            record,
            scope,
            expires: Instant::now() + Duration::from_secs(60),
            inspection_only,
            executable_sha256,
        })
    }

    /// Starts one bounded discovery worker.  The review is consumed whether
    /// launch succeeds or fails; a stale approval is never retained for retry.
    pub fn connect(
        &self,
        review: ConnectionReview,
        consent: ConnectionConsent,
    ) -> Result<String, DomainError> {
        if !Arc::ptr_eq(&self.owner, &review.owner)
            || Instant::now() >= review.expires
            || consent != ConnectionConsent::LocalHostAccountExecution
        {
            return Err(DomainError::PermissionDenied);
        }
        self.reap_finished()?;
        let mut state = self.state()?;
        let record = state
            .records
            .servers
            .get(&review.record.id)
            .ok_or(DomainError::NotFound)?;
        if record.revision != review.record.revision
            || (!review.inspection_only && !record.availability.allows(&review.scope))
        {
            return Err(DomainError::Conflict);
        }
        if state.live.len() >= MAX_CONNECTIONS {
            return Err(DomainError::LimitExceeded(
                "MCP connection slots; disconnect finished entries".into(),
            ));
        }
        if state.live.values().any(|worker| {
            worker.control.view.lock().is_ok_and(|view| {
                view.status.server_id == record.id
                    && view.status.scope == review.scope
                    && !matches!(
                        view.status.health,
                        ConnectionHealth::Stopped | ConnectionHealth::Failed
                    )
            })
        }) {
            return Err(DomainError::Conflict);
        }
        if local_digest(record)? != review.executable_sha256 {
            return Err(DomainError::Conflict);
        }

        let id = new_id();
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
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
        let root = self.root.clone();
        let limits = self.limits;
        let worker_record = record.clone();
        let worker_scope = review.scope.clone();
        let worker_control = control.clone();
        let digest = review.executable_sha256.clone();
        let worker_id = id.clone();
        let join = thread::Builder::new()
            .name("ag1w-mcp-discovery".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker(
                        &root,
                        &worker_id,
                        worker_record,
                        worker_scope,
                        digest,
                        limits,
                        worker_control.clone(),
                    )
                }));
                if result.is_err() {
                    fail(&worker_control, Diagnostic::WorkerFailed);
                }
            })
            .map_err(|_| DomainError::Unavailable("MCP worker start".into()))?;
        state.live.insert(
            id.clone(),
            Worker {
                control,
                join: Some(join),
            },
        );
        Ok(id)
    }

    pub fn disconnect(&self, id: &str) -> Result<(), DomainError> {
        self.state()?
            .live
            .get(id)
            .ok_or(DomainError::NotFound)?
            .control
            .cancel();
        Ok(())
    }

    /// Join only from an application worker, never from a Tauri event handler.
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

    pub fn recent_connections(&self) -> Result<Vec<ConnectionStatus>, DomainError> {
        self.reap_finished()?;
        let state = self.state()?;
        let mut connections: Vec<_> = state.recent.iter().cloned().collect();
        for worker in state.live.values() {
            connections.push(
                worker
                    .control
                    .view
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .status
                    .clone(),
            );
        }
        Ok(connections)
    }

    pub fn catalog(
        &self,
        id: &str,
        scope: &CatalogScope,
        inspection: bool,
    ) -> Result<Arc<ConnectedCatalog>, DomainError> {
        let state = self.state()?;
        let worker = state.live.get(id).ok_or(DomainError::NotFound)?;
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
            || (!inspection && view.status.inspection_only)
            || (!inspection && !record.availability.allows(scope))
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

    fn reap_finished(&self) -> Result<(), DomainError> {
        let mut state = self.state()?;
        let finished: Vec<_> = state
            .live
            .iter()
            .filter(|(_, worker)| worker.join.as_ref().is_none_or(|join| join.is_finished()))
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

impl Drop for Manager {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner());
        for worker in state.live.values() {
            worker.control.cancel();
        }
        for worker in state.live.values_mut() {
            let _ = worker.join();
        }
    }
}

fn validate_records(records: &Records) -> Result<(), DomainError> {
    if records.servers.len() > MAX_SERVERS {
        return Err(invalid("stored server count"));
    }
    for (id, record) in &records.servers {
        if id != &record.id || record.revision == 0 || record.config_revision > record.revision {
            return Err(invalid("stored server identity"));
        }
        validate_windows_draft(&record.draft, record.local_access)?;
        record.availability.validate()?;
        // Constructing a slot invokes the catalog's private binding checks too.
        // Stored definitions must not bypass the same invariants as live ones.
        let _ = CatalogSlot::new(CatalogBinding {
            server_id: record.id.clone(),
            scope: CatalogScope::GeneralChat,
            config_revision: record.configuration_revision(),
            auth_profile: "mcp-unauthenticated".into(),
            credential_generation: 0,
            connection_generation: 1,
            protocol: Revision::parse(&record.draft.protocol)?,
        })?;
    }
    Ok(())
}

fn validate_windows_draft(draft: &Draft, access: LocalAccess) -> Result<(), DomainError> {
    draft.validate()?;
    let Configuration::Local {
        secret_environment, ..
    } = &draft.configuration
    else {
        return Err(DomainError::Unavailable(
            "Windows-native MCP currently supports reviewed local stdio servers only".into(),
        ));
    };
    if access != LocalAccess::HostAccount {
        return Err(DomainError::Unavailable(
            "Windows-native MCP does not emulate Linux read-only containment; choose explicit host-account access or keep this server inactive".into(),
        ));
    }
    if draft.auth_profile.is_some() || !secret_environment.is_empty() {
        return Err(DomainError::Unavailable(
            "Windows-native MCP private credentials and OAuth are not implemented yet".into(),
        ));
    }
    Ok(())
}

fn local_digest(record: &ServerRecord) -> Result<String, DomainError> {
    let Configuration::Local {
        executable, cwd, ..
    } = &record.draft.configuration
    else {
        return Err(DomainError::Unavailable(
            "Windows-native MCP currently supports local stdio servers only".into(),
        ));
    };
    let executable = Path::new(executable);
    let cwd = Path::new(cwd);
    let metadata = executable.metadata().map_err(|_| DomainError::NotFound)?;
    if !metadata.is_file() || !cwd.is_dir() {
        return Err(invalid("local target type"));
    }
    if metadata.len() > 128 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("MCP executable bytes".into()));
    }
    let mut file = fs::File::open(executable).map_err(crate::io_error)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(crate::io_error)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if executable.metadata().map_err(crate::io_error)?.len() != metadata.len() {
        return Err(DomainError::Conflict);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn new_id() -> String {
    // The IDs are local opaque handles, not authentication material.  Combining
    // time with an atomic sequence gives canonical UUID-shaped keys without a
    // network source or a dependency on the legacy Linux entropy path.
    let high = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let low = NEXT_ID.fetch_add(1, Ordering::AcqRel);
    let hex = format!("{high:016x}{low:016x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn binding(
    record: &ServerRecord,
    scope: CatalogScope,
    connection_generation: u64,
) -> CatalogBinding {
    CatalogBinding {
        server_id: record.id.clone(),
        scope,
        config_revision: record.configuration_revision(),
        auth_profile: "mcp-unauthenticated".into(),
        credential_generation: 0,
        connection_generation: connection_generation.max(1),
        protocol: Revision::parse(&record.draft.protocol).expect("validated draft"),
    }
}

fn run_worker(
    root: &Path,
    connection_id: &str,
    record: ServerRecord,
    scope: CatalogScope,
    reviewed_digest: String,
    limits: DiscoveryLimits,
    control: Arc<Control>,
) {
    let result = (|| -> Result<(), DomainError> {
        if control.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        if local_digest(&record)? != reviewed_digest {
            return Err(DomainError::Conflict);
        }
        let Configuration::Local {
            executable,
            arguments,
            cwd,
            public_environment,
            ..
        } = &record.draft.configuration
        else {
            return Err(DomainError::Unavailable("MCP local configuration".into()));
        };
        let logs = root.join("logs");
        fs::create_dir_all(&logs).map_err(crate::io_error)?;
        let stderr = logs.join(format!("mcp-{connection_id}.stderr"));
        let supervisor = ProcessSupervisor::new(root)?;
        let process = supervisor.spawn_protocol_limited(
            ProcessSpec {
                executable: executable.into(),
                argv: arguments.clone(),
                cwd: cwd.into(),
                env: public_environment.clone(),
                timeout_ms: 86_400_000,
                grace_ms: 1_000,
                resident: true,
            },
            &stderr,
            MCP_PROCESS_MEMORY,
        )?;
        let generation = NEXT_ID.fetch_add(1, Ordering::AcqRel);
        let mut client = StdioDiscovery::from_owned(
            process,
            binding(&record, scope.clone(), generation),
            limits,
        )?;
        client.negotiate(&control.cancelled)?;
        if control.cancelled.load(Ordering::Acquire) {
            let _ = client.shutdown();
            return Err(DomainError::Cancelled);
        }

        let server = client
            .server_metadata()
            .cloned()
            .unwrap_or_else(|| json!({}));
        let tools = if server["capabilities"]["tools"].is_object() {
            client.refresh_tools(&control.cancelled)?;
            client
                .tools()
                .map(|catalog| {
                    catalog
                        .tools()
                        .map(|tool| tool.original().clone())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let resources = if server["capabilities"]["resources"].is_object() {
            client
                .list(CatalogKind::Resources, &control.cancelled)
                .map(|catalog| catalog.entries)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let templates = if server["capabilities"]["resources"].is_object() {
            client
                .list(CatalogKind::ResourceTemplates, &control.cancelled)
                .map(|catalog| catalog.entries)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let prompts = if server["capabilities"]["prompts"].is_object() {
            client
                .list(CatalogKind::Prompts, &control.cancelled)
                .map(|catalog| catalog.entries)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let binding = client
            .tools()
            .map(|catalog| catalog.binding().clone())
            .unwrap_or_else(|| binding(&record, scope.clone(), generation));
        let encoded_bytes = serde_json::to_vec(&json!({
            "server":server,"tools":tools,"resources":resources,
            "templates":templates,"prompts":prompts
        }))
        .map_err(|_| invalid("MCP catalog encoding"))?
        .len();
        if encoded_bytes > CATALOG_BYTES {
            return Err(DomainError::LimitExceeded("MCP catalog bytes".into()));
        }
        let catalog = Arc::new(ConnectedCatalog {
            binding,
            server,
            tools,
            resources,
            templates,
            prompts,
            encoded_bytes,
        });
        {
            let mut view = control.view.lock().map_err(|_| DomainError::Conflict)?;
            if control.cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            view.catalog = Some(catalog);
            view.status.health = ConnectionHealth::Ready;
            view.status.diagnostic = None;
        }

        // The stdio client retains the owned process and its bounded pipes for
        // this connection.  We poll process liveness; there is no auto-retry.
        while !control.cancelled.load(Ordering::Acquire) {
            if !client.is_running()? {
                return Err(DomainError::Unavailable("MCP child exited".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
        client.shutdown()?;
        Ok(())
    })();
    match result {
        Ok(()) | Err(DomainError::Cancelled) => stop(&control),
        Err(error) => fail(&control, diagnostic(&error)),
    }
}

fn stop(control: &Control) {
    if let Ok(mut view) = control.view.lock() {
        view.catalog = None;
        view.status.health = ConnectionHealth::Stopped;
        view.status.diagnostic = None;
    }
}

fn fail(control: &Control, diagnostic: Diagnostic) {
    if let Ok(mut view) = control.view.lock() {
        view.catalog = None;
        view.status.health = ConnectionHealth::Failed;
        view.status.diagnostic = Some(diagnostic);
    }
}

fn diagnostic(error: &DomainError) -> Diagnostic {
    match error {
        DomainError::Cancelled => Diagnostic::Cancelled,
        DomainError::TimedOut => Diagnostic::TimedOut,
        DomainError::PermissionDenied => Diagnostic::PermissionDenied,
        DomainError::InvalidInput(_) => Diagnostic::InvalidPeer,
        DomainError::NotFound => Diagnostic::StartFailed,
        DomainError::Conflict => Diagnostic::TransportFailed,
        _ => Diagnostic::Unavailable,
    }
}

fn remember(state: &mut State, status: ConnectionStatus) {
    if state.recent.len() == 128 {
        state.recent.pop_front();
    }
    state.recent.push_back(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn draft(executable: &Path, cwd: &Path) -> Draft {
        Draft {
            name: "fixture".into(),
            configuration: Configuration::Local {
                executable: executable.to_string_lossy().into_owned(),
                arguments: vec![],
                cwd: cwd.to_string_lossy().into_owned(),
                public_environment: BTreeMap::new(),
                secret_environment: Default::default(),
            },
            protocol: Revision::Current20260728.as_str().into(),
            auth_profile: None,
            provenance: super::super::definitions::Provenance::Manual,
        }
    }

    #[test]
    fn local_server_requires_explicit_host_account_access() {
        let directory = tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let manager = Manager::open(directory.path(), DiscoveryLimits::default()).unwrap();
        let error = manager
            .add_drafts_with_access(
                vec![draft(&executable, directory.path())],
                LocalAccess::ReadOnlyDirectory,
            )
            .unwrap_err();
        assert!(matches!(error, DomainError::Unavailable(_)));
        let ids = manager
            .add_drafts_with_access(
                vec![draft(&executable, directory.path())],
                LocalAccess::HostAccount,
            )
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(manager.records().unwrap().len(), 1);
    }

    #[test]
    fn review_requires_availability_before_a_normal_connection() {
        let directory = tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let manager = Manager::open(directory.path(), DiscoveryLimits::default()).unwrap();
        let id = manager
            .add_drafts_with_access(
                vec![draft(&executable, directory.path())],
                LocalAccess::HostAccount,
            )
            .unwrap()
            .pop()
            .unwrap();
        let record = manager.records().unwrap().pop().unwrap();
        assert!(matches!(
            manager.review_connection(&id, record.revision, CatalogScope::GeneralChat),
            Err(DomainError::PermissionDenied)
        ));
        let mut availability = record.availability.clone();
        availability.enabled = true;
        availability.general_chat = true;
        manager
            .set_availability(&id, record.revision, availability)
            .unwrap();
        let updated = manager.records().unwrap().pop().unwrap();
        let review = manager
            .review_connection(&id, updated.revision, CatalogScope::GeneralChat)
            .unwrap();
        assert_eq!(review.record().id, id);
        assert_eq!(review.scope(), &CatalogScope::GeneralChat);
    }
}
