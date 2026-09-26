//! Application-owned manager and session-scoped UI operations. This API is not
//! registered as a model tool; activation consumes a native pending review.
use super::{
    catalog::CatalogScope,
    definitions::{Availability, Configuration, Draft, Provenance},
    discovery::DiscoveryLimits,
    manager::{ConnectionConsent, ConnectionHealth, ConnectionReview, Manager, SessionLease},
    wire::{Limits, decode_json},
};
use ag1_core::{
    DomainError,
    tools::{mcp::Kind, registry::Scope},
};
use ag1_storage::Store;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

pub struct Service {
    directory: Arc<super::registry::Directory>,
    manager: Arc<Manager>,
    helper: PathBuf,
    private_entry: Arc<Mutex<Option<Arc<dyn super::manager::PrivateEntry>>>>,
    #[cfg(target_os = "linux")]
    oauth_entry: Arc<Mutex<Option<Arc<dyn super::manager::OAuthEntry>>>>,
}
#[derive(Clone)]
pub struct Context {
    pub service: Arc<Service>,
    pub general_chat: bool,
}
impl Context {
    /// Folder grants carry a conversation ID for journal custody. Availability
    /// instead follows the selected, verified directory identity. An unavailable
    /// primary receives no MCP runtime; folder recovery remains usable.
    pub(crate) fn catalog_scope(
        &self,
        storage: &crate::workspace::Workspace,
        folders: &crate::folder_access::Snapshot,
    ) -> Result<Option<CatalogScope>, DomainError> {
        if !folders.primary_available() {
            return Ok(None);
        }
        let primary = folders.primary();
        let id = primary.canonical_id()?;
        let scope = if self.general_chat
            && primary.root() == storage.root()
            && primary.directory_identity() == storage.directory_identity()
        {
            CatalogScope::GeneralChat
        } else {
            CatalogScope::Workspace(id.as_str().into())
        };
        Ok(Some(scope))
    }
}
fn shared() -> &'static Mutex<BTreeMap<PathBuf, Weak<Service>>> {
    static SERVICES: std::sync::OnceLock<Mutex<BTreeMap<PathBuf, Weak<Service>>>> =
        std::sync::OnceLock::new();
    SERVICES.get_or_init(Default::default)
}
impl Service {
    pub fn shared(state: &Path, helper: &Path) -> Result<Arc<Self>, DomainError> {
        let state = std::fs::canonicalize(state).map_err(|_| DomainError::NotFound)?;
        let helper = std::fs::canonicalize(helper).map_err(|_| DomainError::NotFound)?;
        let mut services = shared().lock().map_err(|_| DomainError::Conflict)?;
        services.retain(|_, s| s.strong_count() > 0);
        if let Some(service) = services.get(&state).and_then(Weak::upgrade) {
            if service.helper != helper {
                return Err(DomainError::Conflict);
            }
            return Ok(service);
        }
        let service = Arc::new(Self {
            directory: Arc::new(Store::open(&state.join("mcp-discovery-cache")).map_err(DomainError::from).and_then(super::registry::Directory::open).unwrap_or_else(|_|super::registry::Directory::memory())),
            manager: Arc::new(
                Manager::open(
                    Store::open(&state.join("mcp-store"))?,
                    &helper,
                    DiscoveryLimits::new(Duration::from_secs(15))?,
                )?
                .with_vault(super::auth::Vault::open(
                    ag1_storage::mcp_private::PrivateStore::open(&state.join("mcp-private-store"))?,
                )?)?,
            ),
            helper,
            private_entry: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            oauth_entry: Arc::new(Mutex::new(None)),
        });
        services.insert(state, Arc::downgrade(&service));
        Ok(service)
    }
    /// Native fixture composition still uses the real manager, transport and store.
    pub fn from_manager(manager: Arc<Manager>, helper: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            directory: Arc::new(super::registry::Directory::memory()),
            manager,
            helper,
            private_entry: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            oauth_entry: Arc::new(Mutex::new(None)),
        })
    }
    pub fn from_manager_with_directory(manager: Arc<Manager>, helper: PathBuf, directory: Arc<super::registry::Directory>) -> Arc<Self> {
        Arc::new(Self { manager, helper, directory, private_entry: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")] oauth_entry: Arc::new(Mutex::new(None)), })
    }
    #[cfg(feature = "mcp-fixtures")]
    pub fn with_fixture_directory(&self, directory: Arc<super::registry::Directory>) -> Arc<Self> {
        Arc::new(Self { directory, manager:self.manager.clone(), helper:self.helper.clone(), private_entry:self.private_entry.clone(),
            #[cfg(target_os="linux")] oauth_entry:self.oauth_entry.clone() })
    }
    pub fn set_input_entry(&self, entry: Arc<dyn super::input::Entry>) -> Result<(),DomainError> {
        self.manager.set_input_entry(entry)
    }
    pub fn set_private_entry(
        &self,
        entry: Arc<dyn super::manager::PrivateEntry>,
    ) -> Result<(), DomainError> {
        *self
            .private_entry
            .lock()
            .map_err(|_| DomainError::Conflict)? = Some(entry);
        Ok(())
    }
    #[cfg(target_os = "linux")]
    pub fn set_oauth_entry(
        &self,
        entry: Arc<dyn super::manager::OAuthEntry>,
    ) -> Result<(), DomainError> {
        *self.oauth_entry.lock().map_err(|_| DomainError::Conflict)? = Some(entry);
        Ok(())
    }
    pub fn session(
        &self,
        catalog_scope: CatalogScope,
        scope: Scope,
        policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
    ) -> Result<Arc<Session>, DomainError> {
        self.session_with_project(catalog_scope, scope, policy, None)
    }
    /// Native folder capability only. Renderer paths cannot select an import root.
    pub fn session_with_project(
        &self, catalog_scope: CatalogScope, scope: Scope,
        policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
        project: Option<Arc<crate::workspace::Workspace>>,
    ) -> Result<Arc<Session>, DomainError> {
        if let Some(project) = &project {
            if catalog_scope != CatalogScope::Workspace(project.canonical_id()?.as_str().into()) {
                return Err(DomainError::PermissionDenied);
            }
        }
        let lease = Arc::new(SessionLease::for_session(
            self.manager.clone(),
            catalog_scope,
            &scope,
        )?);
        let discovery = Arc::new(crate::session::mcp::discovery::Discovery::owned(
            lease.clone(),
            scope.clone(),
            policy,
        )?);
        Ok(Arc::new(Session {
            directory: self.directory.clone(),
            setup_pending: Mutex::new(BTreeMap::new()),
            private_entry: self.private_entry.clone(),
            #[cfg(target_os = "linux")]
            oauth_entry: self.oauth_entry.clone(),
            scope,
            lease,
            discovery,
            pending: Mutex::new(BTreeMap::new()),
            trust_pending: Mutex::new(BTreeMap::new()),
            project,
            import_pending: Mutex::new(BTreeMap::new()),
        }))
    }
}
struct PendingSetup {
    draft: Draft,
    expires: Instant,
}
struct PendingImport {
    drafts: Vec<Draft>,
    digest: String,
    expires: Instant,
}
struct PendingTrust {
    review: super::manager::TrustReview,
    expires: Instant,
}
struct Pending {
    review: ConnectionReview,
    consent: ConnectionConsent,
    expires: Instant,
}
pub struct Session {
    directory: Arc<super::registry::Directory>,
    setup_pending: Mutex<BTreeMap<String, PendingSetup>>,
    private_entry: Arc<Mutex<Option<Arc<dyn super::manager::PrivateEntry>>>>,
    #[cfg(target_os = "linux")]
    oauth_entry: Arc<Mutex<Option<Arc<dyn super::manager::OAuthEntry>>>>,
    scope: Scope,
    lease: Arc<SessionLease>,
    discovery: Arc<crate::session::mcp::discovery::Discovery>,
    pending: Mutex<BTreeMap<String, Pending>>,
    trust_pending: Mutex<BTreeMap<String, PendingTrust>>,
    project: Option<Arc<crate::workspace::Workspace>>,
    import_pending: Mutex<BTreeMap<String, PendingImport>>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    List,
    Discover { #[serde(default)] query: String, #[serde(default)] cursor: String, #[serde(default)] fetch: bool, #[serde(default)] refresh: bool },
    DiscoverDetail { key: String },
    DiscoverIcon { key: String },
    DiscoverReview { key: String, sha256: String, index: usize },
    DiscoverLocalReview { key: String, sha256: String, index: usize, configuration: Configuration },
    DiscoverConfirm { review: String },
    DiscoverDismiss { review: String },
    ImportReview {},
    ImportConfirm { review: String },
    ImportDismiss { review: String },
    TrustMode {
        id: String,
        revision: u64,
        mode: ag1_core::tools::mcp::trust::TrustMode,
    },
    TrustRevoke {
        id: String,
        revision: u64,
        wire_name: String,
    },
    TrustReview {
        connection: String,
        revision: u64,
        kind: Kind,
        key: String,
        rules: Vec<ag1_core::tools::mcp::trust::Rule>,
        #[serde(default)]
        append: bool,
    },
    TrustConfirm {
        review: String,
    },
    TrustDismiss {
        review: String,
    },
    PrivateEntry {
        id: String,
        revision: u64,
    },
    #[cfg(target_os = "linux")]
    #[serde(rename = "oauth")]
    OAuth {
        id: String,
        revision: u64,
        #[serde(default)]
        refresh: bool,
    },
    Logout {
        id: String,
        revision: u64,
    },
    Add {
        draft: Draft,
        #[serde(default)]
        local_access: super::definitions::LocalAccess,
    },
    Edit {
        id: String,
        revision: u64,
        draft: Draft,
        #[serde(default)]
        local_access: super::definitions::LocalAccess,
    },
    Availability {
        id: String,
        revision: u64,
        enabled: bool,
        target: AvailabilityTarget,
    },
    Remove {
        id: String,
        revision: u64,
    },
    Review {
        id: String,
        revision: u64,
        test: bool,
    },
    Confirm {
        review: String,
    },
    Dismiss {
        review: String,
    },
    Disconnect {
        connection: String,
    },
    Catalog {
        connection: String,
        kind: Kind,
        offset: usize,
    },
    Detail {
        connection: String,
        kind: Kind,
        key: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AvailabilityTarget {
    Current,
    AllWorkspaces,
    Everywhere,
}
impl Session {
    pub fn discovery(&self) -> &Arc<crate::session::mcp::discovery::Discovery> {
        &self.discovery
    }
    pub fn close(&self) -> Result<(), DomainError> {
        self.setup_pending.lock().map_err(|_| DomainError::Conflict)?.clear();
        self.import_pending.lock().map_err(|_| DomainError::Conflict)?.clear();
        self.trust_pending
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clear();
        self.pending
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clear();
        self.discovery.close()
    }
    fn owner(&self, scope: &Scope) -> Result<(), DomainError> {
        self.lease.check()?;
        if scope.workspace != self.scope.workspace
            || scope.session != self.scope.session
            || scope.epoch != self.scope.epoch
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    fn current(
        &self,
        id: &str,
        revision: u64,
    ) -> Result<super::manager::ServerRecord, DomainError> {
        let record = self
            .lease
            .manager()
            .records()?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or(DomainError::NotFound)?;
        if record.revision != revision {
            return Err(DomainError::Conflict);
        }
        Ok(record)
    }
    /// Called by the native command worker, with its actual selected session.
    pub fn dispatch(&self, scope: &Scope, args: &Value) -> Result<Value, DomainError> {
        self.owner(scope)?;
        let raw = serde_json::to_vec(args)
            .map_err(|_| DomainError::InvalidInput("MCP command".into()))?;
        if raw.len() > 128 * 1024 {
            return Err(DomainError::LimitExceeded("MCP command".into()));
        }
        let value = decode_json(&raw, Limits::default())?;
        let action: Action = serde_json::from_value(value)
            .map_err(|_| DomainError::InvalidInput("MCP management fields".into()))?;
        let manager = self.lease.manager();
        match action {
            Action::Discover { query, cursor, fetch, refresh } => self.directory.search(&query,&cursor,fetch,refresh),
            Action::DiscoverIcon { key } => self.directory.icon(&key),
            Action::DiscoverDetail { key } => {
                let mut value=self.directory.detail(&key)?;
                value["suggestedDirectory"]=self.project.as_ref().map(|p|json!(p.root())).unwrap_or(Value::Null);
                Ok(value)
            },
            Action::DiscoverReview { key, sha256, index } => {
                let draft=self.directory.remote_draft(&key,&sha256,index)?;
                self.directory.revalidate(&draft.provenance)?;
                let id=format!("mcp-setup-{}",super::manager::new_id()?);
                let display=json!({"review":id,"draft":draft,"scope":scope_value(self.lease.scope()),"expiresInSeconds":60,"message":"This exact registry endpoint will be saved inactive. Authentication, connection and scope activation remain separate reviewed steps. No server endpoint is contacted by saving."});
                let mut pending=self.setup_pending.lock().map_err(|_|DomainError::Conflict)?;
                pending.retain(|_,p|Instant::now()<p.expires);
                if pending.len()>=16{return Err(DomainError::LimitExceeded("MCP setup reviews".into()));}
                pending.insert(id,PendingSetup{draft,expires:Instant::now()+Duration::from_secs(60)});Ok(display)
            }
            Action::DiscoverLocalReview { key, sha256, index, configuration } => {
                let draft=self.directory.local_draft(&key,&sha256,index,configuration)?;
                self.directory.revalidate(&draft.provenance)?;
                let id=format!("mcp-setup-{}",super::manager::new_id()?);
                let display=json!({"review":id,"draft":draft,"localAccess":"readOnlyDirectory","scope":scope_value(self.lease.scope()),"expiresInSeconds":60,"message":"Save this existing local installation inactive, with read-only directory access and no networking. AG1 has not installed or verified the advertised package/version on disk. Review the executable and arguments; dependencies are mutable. Supply private environment values after saving."});
                let mut pending=self.setup_pending.lock().map_err(|_|DomainError::Conflict)?;
                pending.retain(|_,p|Instant::now()<p.expires);
                if pending.len()>=16{return Err(DomainError::LimitExceeded("MCP setup reviews".into()));}
                pending.insert(id,PendingSetup{draft,expires:Instant::now()+Duration::from_secs(60)});Ok(display)
            }
            Action::DiscoverConfirm { review } => {
                let pending=self.setup_pending.lock().map_err(|_|DomainError::Conflict)?.remove(&review).ok_or(DomainError::PermissionDenied)?;
                if Instant::now()>=pending.expires{return Err(DomainError::TimedOut);}
                self.lease.check()?;
                let access=if matches!(pending.draft.configuration,Configuration::Local{..}) {super::definitions::LocalAccess::ReadOnlyDirectory}else{super::definitions::LocalAccess::HostAccount};
                Ok(json!({"ids":manager.add_drafts_with_access(vec![pending.draft],access)?,"active":false,"message":"Server saved inactive. Configure authentication, test the connection and enable it in the desired scope."}))
            }
            Action::DiscoverDismiss { review } => { self.setup_pending.lock().map_err(|_|DomainError::Conflict)?.remove(&review);Ok(json!({"dismissed":true})) }

            Action::ImportReview {} => {
                let project = self.project.as_ref().ok_or_else(|| DomainError::Unavailable("Open a project folder to import its .mcp.json configuration.".into()))?;
                let snapshot = project.snapshot(".mcp.json", 256 * 1024)?;
                let source = project.root().join(".mcp.json");
                let drafts = super::definitions::import_project(&snapshot.bytes, &source.to_string_lossy(), &project.root().to_string_lossy())?;
                for draft in &drafts { super::definitions::LocalAccess::ReadOnlyDirectory.validate(&draft.configuration)?; }
                let digest = ag1_storage::digest(&snapshot.bytes);
                let id = format!("mcp-import-{}", super::manager::new_id()?);
                let display = json!({"review":id,"source":source,"sha256":digest,"drafts":drafts,"localAccess":"readOnlyDirectory","expiresInSeconds":300,"message":"These untrusted project definitions will be saved inactive with read-only directory access and no network. Review each command and working directory. Nothing is installed, launched, connected, authenticated or enabled by import."});
                let mut pending = self.import_pending.lock().map_err(|_| DomainError::Conflict)?;
                pending.retain(|_, p| Instant::now() < p.expires);
                if pending.len() >= 4 { return Err(DomainError::LimitExceeded("MCP import reviews".into())); }
                pending.insert(id, PendingImport { drafts, digest, expires: Instant::now() + Duration::from_secs(300) });
                Ok(display)
            }
            Action::ImportConfirm { review } => {
                let pending = self.import_pending.lock().map_err(|_| DomainError::Conflict)?.remove(&review).ok_or(DomainError::PermissionDenied)?;
                if Instant::now() >= pending.expires { return Err(DomainError::TimedOut); }
                let project = self.project.as_ref().ok_or(DomainError::PermissionDenied)?;
                let snapshot = project.snapshot(".mcp.json", 256 * 1024)?;
                if ag1_storage::digest(&snapshot.bytes) != pending.digest { return Err(DomainError::Conflict); }
                self.lease.check()?;
                Ok(json!({"ids":manager.add_drafts_with_access(pending.drafts, super::definitions::LocalAccess::ReadOnlyDirectory)?,"active":false,"message":"Project servers saved inactive. Configure authentication and review a connection separately."}))
            }
            Action::ImportDismiss { review } => {
                self.import_pending.lock().map_err(|_| DomainError::Conflict)?.remove(&review);
                Ok(json!({"dismissed":true}))
            }
            Action::TrustMode { id, revision, mode } => {
                self.current(&id, revision)?;
                self.lease.set_trust_mode(&id, revision, mode)?;
                Ok(
                    json!({"saved":true,"message":"Scope policy saved. Reconnect to use this scope with the new policy."}),
                )
            }
            Action::TrustRevoke {
                id,
                revision,
                wire_name,
            } => {
                self.current(&id, revision)?;
                self.lease.revoke_trust(&id, revision, &wire_name)?;
                Ok(
                    json!({"saved":true,"message":"Rule revoked. Connections in this scope were retired; other scopes are unchanged."}),
                )
            }
            Action::TrustReview {
                connection,
                revision,
                kind,
                key,
                rules,
                append,
            } => {
                let review = if append {
                    self.lease
                        .review_trust_append(&connection, revision, kind, &key, rules)?
                } else {
                    self.lease
                        .review_trust(&connection, revision, kind, &key, rules)?
                };
                let id = format!("mcp-trust-{}", super::manager::new_id()?);
                let display = json!({"review":id,"expiresInSeconds":60,"details":review.display()});
                let mut pending = self
                    .trust_pending
                    .lock()
                    .map_err(|_| DomainError::Conflict)?;
                pending.retain(|_, p| Instant::now() < p.expires);
                if pending.len() >= 16 {
                    return Err(DomainError::LimitExceeded("MCP trust reviews".into()));
                }
                pending.insert(
                    id,
                    PendingTrust {
                        review,
                        expires: Instant::now() + Duration::from_secs(60),
                    },
                );
                Ok(display)
            }
            Action::TrustConfirm { review } => {
                let pending = self
                    .trust_pending
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .remove(&review)
                    .ok_or(DomainError::NotFound)?;
                if Instant::now() >= pending.expires {
                    return Err(DomainError::Conflict);
                }
                self.lease.save_trust(pending.review)?;
                Ok(
                    json!({"saved":true,"message":"Reviewed rules saved for this scope. Reconnect to apply them; consequential operations still require confirmation."}),
                )
            }
            Action::TrustDismiss { review } => {
                self.trust_pending
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .remove(&review);
                Ok(json!({"dismissed":true}))
            }
            Action::List => self.list(),
            Action::PrivateEntry { id, revision } => {
                let entry = self
                    .private_entry
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .clone()
                    .ok_or_else(|| {
                        DomainError::Unavailable("Native MCP private entry is unavailable".into())
                    })?;
                let review = self.lease.review_private(&id, revision)?;
                self.lease.enter_private(review, entry.as_ref())?;
                self.owner(scope)?;
                Ok(
                    json!({"saved":true,"active":false,"message":"Private values saved. Review and connect separately."}),
                )
            }
            #[cfg(target_os = "linux")]
            Action::OAuth {
                id,
                revision,
                refresh,
            } => {
                let entry = self
                    .oauth_entry
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .clone()
                    .ok_or_else(|| {
                        DomainError::Unavailable("Native MCP OAuth sign-in is unavailable".into())
                    })?;
                let review = self.lease.review_oauth(&id, revision)?;
                self.lease.enter_oauth(review, refresh, entry.as_ref())?;
                self.owner(scope)?;
                Ok(
                    json!({"saved":true,"active":false,"message":"Authentication updated. Review and connect separately."}),
                )
            }
            Action::Logout { id, revision } => {
                manager.logout(&id, revision)?;
                Ok(
                    json!({"loggedOutLocally":true,"upstreamRevocation":"not requested or confirmed","message":"Local credentials removed and affected connections retired. Revoke remotely in the server account settings if needed."}),
                )
            }
            Action::Add { draft, local_access } => {
                if draft.provenance != Provenance::Manual {
                    return Err(DomainError::PermissionDenied);
                }
                Ok(json!({"ids":manager.add_drafts_with_access(vec![draft], local_access)?,"active":false}))
            }
            Action::Edit {
                id,
                revision,
                draft,
                local_access,
            } => {
                self.current(&id, revision)?;
                if draft.provenance != Provenance::Manual {
                    return Err(DomainError::PermissionDenied);
                }
                manager.edit_with_access(&id, revision, draft, local_access)?;
                Ok(json!({"saved":true,"active":false}))
            }
            Action::Availability {
                id,
                revision,
                enabled,
                target,
            } => {
                let mut availability = self.current(&id, revision)?.availability;
                match target {
                    AvailabilityTarget::Everywhere => {
                        if enabled {
                            return Err(DomainError::InvalidInput(
                                "Choose a specific availability scope".into(),
                            ));
                        }
                        availability = Availability::default();
                    }
                    AvailabilityTarget::AllWorkspaces => {
                        availability.all_workspaces = enabled;
                        if enabled {
                            availability.enabled = true;
                        }
                    }
                    AvailabilityTarget::Current => {
                        if enabled {
                            availability.enabled = true;
                        }
                        match self.lease.scope() {
                            CatalogScope::GeneralChat => availability.general_chat = enabled,
                            CatalogScope::Workspace(id) => {
                                if enabled {
                                    availability.workspaces.insert(id.clone());
                                    availability.denied_workspaces.remove(id);
                                } else {
                                    availability.workspaces.remove(id);
                                    availability.denied_workspaces.insert(id.clone());
                                }
                            }
                        }
                    }
                }
                manager.set_availability(&id, revision, availability)?;
                Ok(json!({"saved":true}))
            }
            Action::Remove { id, revision } => {
                self.current(&id, revision)?;
                manager.remove(&id, revision)?;
                Ok(json!({"removed":true,"ownedWork":"revoked and draining"}))
            }
            Action::Review { id, revision, test } => {
                let record = self.current(&id, revision)?;
                self.directory.revalidate(&record.draft.provenance)?;
                let review = self.lease.review(&id, test)?;
                let consent = match &review.record().draft.configuration {
                    Configuration::Local { .. } => if review.record().local_access == super::definitions::LocalAccess::ReadOnlyDirectory { ConnectionConsent::LocalReadOnlyDirectory } else { ConnectionConsent::LocalHostAccountExecution },
                    Configuration::Http {
                        loopback_development: true,
                        ..
                    } => ConnectionConsent::LoopbackDevelopmentEndpoint,
                    _ => ConnectionConsent::RemoteEndpoint,
                };
                let id = format!("mcp-review-{}", super::manager::new_id()?);
                let display = json!({"review":id,"server":public_record(review.record()),"scope":scope_value(review.scope()),"executableSha256":review.executable_sha256(),"inspectionOnly":test,"expiresInSeconds":60,"warning":match consent {ConnectionConsent::LocalReadOnlyDirectory=>super::definitions::LocalAccess::ReadOnlyDirectory.description(),ConnectionConsent::LocalHostAccountExecution=>"This starts the exact command below with your operating-system account permissions. Package runners may download and execute code. Scope availability is not an OS sandbox.",ConnectionConsent::LoopbackDevelopmentEndpoint=>"This connects to the reviewed local development endpoint. The server can observe the connection.",ConnectionConsent::RemoteEndpoint=>"This connects to the reviewed remote endpoint. The server can observe the connection."},"purpose":"Handshake and capability discovery only. No sample tool call, resource read or prompt retrieval."});
                let mut pending = self.pending.lock().map_err(|_| DomainError::Conflict)?;
                pending.retain(|_, p| Instant::now() < p.expires);
                if pending.len() >= 16 {
                    return Err(DomainError::LimitExceeded("MCP connection reviews".into()));
                }
                pending.insert(
                    id,
                    Pending {
                        review,
                        consent,
                        expires: Instant::now() + Duration::from_secs(60),
                    },
                );
                Ok(display)
            }
            Action::Confirm { review } => {
                let pending = self
                    .pending
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .remove(&review)
                    .ok_or(DomainError::NotFound)?;
                if Instant::now() >= pending.expires {
                    return Err(DomainError::Conflict);
                }
                Ok(
                    json!({"connection":self.lease.connect(pending.review,pending.consent)?,"health":"connecting"}),
                )
            }
            Action::Dismiss { review } => {
                self.pending
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .remove(&review);
                Ok(json!({"dismissed":true}))
            }
            Action::Disconnect { connection } => {
                self.lease.disconnect(&connection)?;
                Ok(json!({"disconnected":true}))
            }
            Action::Catalog {
                connection,
                kind,
                offset,
            } => {
                let catalog = self.lease.inspect(&connection)?;
                let rows = rows(&catalog, kind);
                if offset > rows.len() {
                    return Err(DomainError::InvalidInput("MCP catalog page".into()));
                }
                let items:Vec<_>=rows.iter().skip(offset).take(20).map(|r|json!({"key":r[kind.key_field()],"name":r["name"],"description":r["description"].as_str().unwrap_or("").chars().take(160).collect::<String>()})).collect();
                Ok(
                    json!({"items":items,"total":rows.len(),"next":(offset+20<rows.len()).then_some(offset+20),"binding":binding_value(&catalog.binding),"server":catalog.server}),
                )
            }
            Action::Detail {
                connection,
                kind,
                key,
            } => {
                if key.len() > 4096 {
                    return Err(DomainError::LimitExceeded("MCP capability key".into()));
                }
                let catalog = self.lease.inspect(&connection)?;
                let original = rows(&catalog, kind)
                    .into_iter()
                    .find(|r| r[kind.key_field()] == key)
                    .ok_or(DomainError::NotFound)?;
                let selected = self.lease.definition(&connection, kind, &key);
                let active = self.lease.connections()?.iter().any(|c| {
                    c.id == connection && !c.inspection_only && c.health == ConnectionHealth::Ready
                });
                let unavailable = if selected.is_err() {
                    Some("This capability schema is not supported for execution.")
                } else if !active {
                    Some(
                        "This test connection is for inspection only. Enable this scope and reconnect to use the capability.",
                    )
                } else {
                    None
                };
                let identity = selected.as_ref().ok().map(|d| {
                    let mut value = json!(d.identity());
                    value["binding"] = binding_value(&catalog.binding);
                    if let Some(owner) = &d.identity().execution_scope {
                        value["executionScope"]["epoch"] = json!(owner.epoch.to_string());
                    }
                    value
                });
                let host_classification = selected
                    .as_ref()
                    .ok()
                    .map(|d| {
                        ag1_core::tools::mcp::trust::HostRisk::for_call(d.identity(), &json!({}))
                    })
                    .transpose()?;
                let reviewed = selected
                    .as_ref()
                    .ok()
                    .and_then(|d| d.reviewed_policy())
                    .is_some();
                Ok(
                    json!({"original":original,"originalJson":original.to_string(),"parameters":selected.as_ref().ok().map(|d|d.metadata()["parameters"].clone()),"binding":binding_value(&catalog.binding),"identity":identity,"wireName":selected.as_ref().ok().map(|d|d.identity().wire_name()),"schemaSupported":selected.is_ok(),"executable":selected.is_ok() && active,"classification":host_classification,"classificationBasis":"AG1 checks native target and scope bindings. Server annotations do not prove effects; argument signals can require consequential confirmation.","reviewedPolicy":reviewed,"approval":if reviewed {"The reviewed scope policy applies to matching argument cases. Consequential actions still require exact confirmation."}else{"No matching reviewed policy. Exact confirmation is required."},"unavailableReason":unavailable}),
                )
            }
        }
    }
    fn list(&self) -> Result<Value, DomainError> {
        #[cfg(target_os = "linux")]
        let oauth_available = self
            .oauth_entry
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_some();
        #[cfg(not(target_os = "linux"))]
        let oauth_available = false;
        let rows: Vec<_> = self
            .lease
            .manager()
            .records()?
            .into_iter()
            .map(|r| {
                let authentication=self.lease.manager().authentication(&r.id)?;
                let public_auth=authentication.map(|a|json!({"status":a.status,"profileId":a.binding.profile_id,"credentialGeneration":a.generation.to_string(),"target":a.binding.target,"account":a.grant.account,"scopes":a.grant.scopes,"expiresAtMs":a.grant.expires_at_ms}));
                let trust_policy=self.lease.manager().scope_policy(&r.id,self.lease.scope())?.map(|p|json!({
                    "scope":p.scope,"revision":p.revision.to_string(),"mode":p.mode,
                    "grants":p.grants.iter().map(|g|json!({"identity":g.identity,"wireName":g.identity.wire_name(),"targetDigest":g.target_digest,"rules":g.rules,"rulesJson":serde_json::to_string_pretty(&g.rules).unwrap_or_default(),"identityJson":serde_json::to_string_pretty(&g.identity).unwrap_or_default()})).collect::<Vec<_>>()
                }));
                Ok(json!({"availableHere":r.availability.allows(self.lease.scope()),"record":public_record(&r),"authentication":public_auth,"trustPolicy":trust_policy}))
            })
            .collect::<Result<_,DomainError>>()?;
        let connections:Vec<_>=self.lease.connections()?.into_iter().map(|s|json!({"id":s.id,"serverId":s.server_id,"scope":scope_value(&s.scope),"health":match s.health {ConnectionHealth::Connecting=>"connecting",ConnectionHealth::Ready=>"ready",ConnectionHealth::Stopping=>"stopping",ConnectionHealth::Stopped=>"stopped",ConnectionHealth::Failed=>"failed"},"waitingForInput":self.lease.manager().waiting_for_input(&s.id).unwrap_or(false),"inspectionOnly":s.inspection_only,"diagnostic":s.diagnostic.map(|d|if d==super::manager::Diagnostic::PrivateValueEcho {"Server output contained a supplied private value and was withheld.".into()}else{format!("{d:?}")})})).collect();
        Ok(
            json!({"servers":rows,"connections":connections,"scope":scope_value(self.lease.scope()),"session":self.scope.session,"epoch":self.scope.epoch,"limits":{"connections":4,"selectedCapabilities":16,"selectedSchemaBytes":65536},"projectImportAvailable":self.project.is_some(),"privateEntryAvailable":self.private_entry.lock().map_err(|_|DomainError::Conflict)?.is_some(),"oauthAvailable":oauth_available,"authentication":"Private values and supported OAuth sign-in use native dialogs. Authentication does not activate a server or approve operations."}),
        )
    }
}
fn rows(catalog: &super::manager::ConnectedCatalog, kind: Kind) -> Vec<&Value> {
    match kind {
        Kind::Tool => catalog.tools.iter().map(|t| t.original()).collect(),
        Kind::Resource => catalog.resources.iter().collect(),
        Kind::ResourceTemplate => catalog.templates.iter().collect(),
        Kind::Prompt => catalog.prompts.iter().collect(),
    }
}

fn scope_value(scope: &CatalogScope) -> Value {
    match scope {
        CatalogScope::GeneralChat => json!({"kind":"generalChat"}),
        CatalogScope::Workspace(id) => json!({"kind":"workspace","workspace":id}),
    }
}
fn binding_value(b: &super::catalog::CatalogBinding) -> Value {
    json!({"serverId":b.server_id,"scope":scope_value(&b.scope),"configRevision":b.config_revision.to_string(),"authProfile":b.auth_profile,"credentialGeneration":b.credential_generation.to_string(),"connectionGeneration":b.connection_generation.to_string(),"protocol":b.protocol.as_str()})
}

fn public_record(record: &super::manager::ServerRecord) -> Value {
    let mut value = json!(record);
    // Scoped argument rules are not broadcast through another workspace's listing.
    if let Some(object) = value.as_object_mut() {
        object.remove("trust");
    }
    value
}
