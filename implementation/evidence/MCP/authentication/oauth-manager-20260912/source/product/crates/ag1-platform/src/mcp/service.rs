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
            manager,
            helper,
            private_entry: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "linux")]
            oauth_entry: Arc::new(Mutex::new(None)),
        })
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
            private_entry: self.private_entry.clone(),
            #[cfg(target_os = "linux")]
            oauth_entry: self.oauth_entry.clone(),
            scope,
            lease,
            discovery,
            pending: Mutex::new(BTreeMap::new()),
        }))
    }
}
struct Pending {
    review: ConnectionReview,
    consent: ConnectionConsent,
    expires: Instant,
}
pub struct Session {
    private_entry: Arc<Mutex<Option<Arc<dyn super::manager::PrivateEntry>>>>,
    #[cfg(target_os = "linux")]
    oauth_entry: Arc<Mutex<Option<Arc<dyn super::manager::OAuthEntry>>>>,
    scope: Scope,
    lease: Arc<SessionLease>,
    discovery: Arc<crate::session::mcp::discovery::Discovery>,
    pending: Mutex<BTreeMap<String, Pending>>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    List,
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
    },
    Edit {
        id: String,
        revision: u64,
        draft: Draft,
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
            Action::Add { draft } => {
                if draft.provenance != Provenance::Manual {
                    return Err(DomainError::PermissionDenied);
                }
                Ok(json!({"ids":manager.add_drafts(vec![draft])?,"active":false}))
            }
            Action::Edit {
                id,
                revision,
                draft,
            } => {
                self.current(&id, revision)?;
                if draft.provenance != Provenance::Manual {
                    return Err(DomainError::PermissionDenied);
                }
                manager.edit(&id, revision, draft)?;
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
                self.current(&id, revision)?;
                let review = self.lease.review(&id, test)?;
                let consent = match &review.record().draft.configuration {
                    Configuration::Local { .. } => ConnectionConsent::LocalHostAccountExecution,
                    Configuration::Http {
                        loopback_development: true,
                        ..
                    } => ConnectionConsent::LoopbackDevelopmentEndpoint,
                    _ => ConnectionConsent::RemoteEndpoint,
                };
                let id = format!("mcp-review-{}", super::manager::new_id()?);
                let display = json!({"review":id,"server":review.record(),"scope":scope_value(review.scope()),"executableSha256":review.executable_sha256(),"inspectionOnly":test,"expiresInSeconds":60,"warning":match consent {ConnectionConsent::LocalHostAccountExecution=>"This starts the exact command below with your operating-system account permissions. Package runners may download and execute code. Scope availability is not an OS sandbox.",ConnectionConsent::LoopbackDevelopmentEndpoint=>"This connects to the reviewed local development endpoint. The server can observe the connection.",ConnectionConsent::RemoteEndpoint=>"This connects to the reviewed remote endpoint. The server can observe the connection."},"purpose":"Handshake and capability discovery only. No sample tool call, resource read or prompt retrieval."});
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
                Ok(
                    json!({"original":original,"originalJson":original.to_string(),"binding":binding_value(&catalog.binding),"identity":identity,"wireName":selected.as_ref().ok().map(|d|d.identity().wire_name()),"schemaSupported":selected.is_ok(),"executable":selected.is_ok() && active,"classification":"unknown","classificationBasis":"No native read/write rule has been reviewed. Server annotations do not grant authority.","approval":"Exact confirmation required for server operations.","unavailableReason":unavailable}),
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
                Ok(json!({"availableHere":r.availability.allows(self.lease.scope()),"record":r,"authentication":public_auth}))
            })
            .collect::<Result<_,DomainError>>()?;
        let connections:Vec<_>=self.lease.connections()?.into_iter().map(|s|json!({"id":s.id,"serverId":s.server_id,"scope":scope_value(&s.scope),"health":match s.health {ConnectionHealth::Connecting=>"connecting",ConnectionHealth::Ready=>"ready",ConnectionHealth::Stopping=>"stopping",ConnectionHealth::Stopped=>"stopped",ConnectionHealth::Failed=>"failed"},"inspectionOnly":s.inspection_only,"diagnostic":s.diagnostic.map(|d|if d==super::manager::Diagnostic::PrivateValueEcho {"Server output contained a supplied private value and was withheld.".into()}else{format!("{d:?}")})})).collect();
        Ok(
            json!({"servers":rows,"connections":connections,"scope":scope_value(self.lease.scope()),"session":self.scope.session,"epoch":self.scope.epoch,"limits":{"connections":4,"selectedCapabilities":16,"selectedSchemaBytes":65536},"privateEntryAvailable":self.private_entry.lock().map_err(|_|DomainError::Conflict)?.is_some(),"oauthAvailable":oauth_available,"authentication":"Private values and supported OAuth sign-in use native dialogs. Authentication does not activate a server or approve operations."}),
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
