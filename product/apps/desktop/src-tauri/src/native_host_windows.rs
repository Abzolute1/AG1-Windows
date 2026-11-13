//! Small Windows desktop host built on the real durable local-session path.
//!
//! It intentionally owns only local llama.cpp chat today. Linux worker features
//! stay unavailable until they have native implementations instead of becoming
//! commands which appear to work but cannot settle their side effects.

use ag1_core::{
    DomainError, FileRevision, SessionId, context::compiler::CompileOptions, inference::Scheduler,
    runtime::session::Configuration, storage::StoragePort,
};
use ag1_platform::{
    change_receipts::{self, ChangeReceipt},
    cutover::configuration::{
        ExpectedLaunch, ImportRequest, ModelImport, PinnedInput, import_configuration,
    },
    folder_access::{Change as FolderChange, FolderAccess},
    llama::{Material, ResolveInput, resolve_launch_plan},
    local_runtime::WindowsRuntimeConfig,
    process::{ProcessSpec, ProcessSupervisor},
    provider::http::{
        CancelSource, GenerationRuntime, ProviderHttp, cancellation, endpoint_origin,
    },
    session::{LocalProjectTools, LocalTurnInput, NativeSession, ProjectWriteAccess},
    workspace::{MAX_READ, Workspace, WorkspaceService, preserve_text},
};
use ag1_storage::{CatalogPosition, Store, digest};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::ImageReader;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

const INSTANCE: &str = "ag1w-native";
const WORKSPACE: &str = "general-chat";
const LEGACY_ORIGIN: &str = "ag1w-native-origin";
const MESSAGE_PAGE: usize = 50;
const HISTORY_PAGE: usize = 100;
const ENTRY_PAGE_BYTES: usize = 64 * 1024;
const LIVE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ATTACHMENT_UPLOAD_BYTES: usize = 4 * 1024 * 1024;
const MAX_ATTACHMENT_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ATTACHMENT_IMAGES: usize = 8;
const MAX_WORKSPACE_DIFF_BYTES: usize = 1024 * 1024;
const CHANGE_PAGE_BYTES: usize = 24 * 1024;
static SETUP_SERIAL: AtomicU64 = AtomicU64::new(0);

/// The three files AG1W needs to own a local llama.cpp runtime.
///
/// The profile remains the source for tuning. AG1W only replaces its server
/// path with the Windows executable selected here and then seals the result.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeModelSetup {
    pub model_id: String,
    pub server_path: PathBuf,
    pub model_path: PathBuf,
    pub profile_path: PathBuf,
    #[serde(default)]
    pub projector_path: Option<PathBuf>,
    pub port: u16,
    #[serde(default)]
    pub vram_mib: Option<u64>,
    #[serde(default)]
    pub system_prompt: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct HostConfig {
    models: Vec<Value>,
    selected: String,
    #[serde(default)]
    system_prompt: String,
    // A sealed capture is optional so the desktop can still attach to a
    // manually started loopback server. When present, AG1W owns the process.
    #[serde(default)]
    runtime: Option<WindowsRuntimeConfig>,
    // Existing AG1 configs carry more settings than this first Windows host
    // consumes. Retain them verbatim when saving a selected local model.
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

/// Only the current response lives here. The durable journal is still the
/// conversation source of truth after the model settles.
struct LiveActivity {
    id: String,
    blocks: Vec<Value>,
    phase: &'static str,
    started_ms: u64,
    first_delta_ms: Option<u64>,
    round: u8,
    revision: u64,
    usage: Value,
}

impl LiveActivity {
    fn new(id: String, started_ms: u64) -> Self {
        Self {
            id,
            blocks: Vec::new(),
            phase: "preparing",
            started_ms,
            first_delta_ms: None,
            round: 0,
            revision: 1,
            usage: json!({}),
        }
    }

    fn block_key(&self, key: &str) -> String {
        format!("{}:{key}", self.round)
    }

    fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    fn observe(&mut self, event: &ag1_core::provider::ProviderEvent) {
        use ag1_core::provider::{BlockKind, ProviderEvent, StopReason};

        match event {
            ProviderEvent::Begin {
                key,
                kind,
                id,
                name,
                arguments,
                ..
            } => {
                let key = self.block_key(key);
                let block = match kind {
                    BlockKind::Text => {
                        self.phase = "generating";
                        json!({"kind":"text","key":key,"text":""})
                    }
                    BlockKind::Thinking => {
                        self.phase = "thinking";
                        json!({"kind":"thinking","key":key,"text":""})
                    }
                    BlockKind::Tool => {
                        self.phase = "tool";
                        json!({
                            "kind":"tool",
                            "key":key,
                            "name":if name.is_empty() { "local tool" } else { name },
                            "call_id":if id.is_empty() { "pending" } else { id },
                            "arguments":arguments.clone().unwrap_or_else(|| json!({})),
                        })
                    }
                };
                self.blocks.push(block);
                self.touch();
            }
            ProviderEvent::Delta { key, text } => self.write_text(key, text, false),
            ProviderEvent::Replace { key, text, .. } => self.write_text(key, text, true),
            ProviderEvent::ToolIdentity { key, id, name } => {
                let key = self.block_key(key);
                if let Some(block) = self.blocks.iter_mut().find(|block| block["key"] == key) {
                    if let Some(id) = id {
                        block["call_id"] = json!(id);
                    }
                    if let Some(name) = name {
                        block["name"] = json!(name);
                    }
                    self.touch();
                }
            }
            ProviderEvent::Stop(StopReason::ToolUse) => {
                self.phase = "executing";
                self.round = self.round.saturating_add(1);
                self.touch();
            }
            ProviderEvent::Stop(_) => {
                self.phase = "generating";
                self.touch();
            }
            ProviderEvent::Fail(_) => {
                self.phase = "failed";
                self.touch();
            }
            ProviderEvent::Usage(usage) => {
                if let Some(value) = usage.input {
                    self.usage["input"] = json!(value);
                }
                if let Some(value) = usage.output {
                    self.usage["output"] = json!(value);
                }
                if let Some(value) = usage.cache_read {
                    self.usage["cacheRead"] = json!(value);
                }
                if let Some(value) = usage.cache_write {
                    self.usage["cacheWrite"] = json!(value);
                }
                if let Some(value) = usage.total {
                    self.usage["total"] = json!(value);
                }
                self.touch();
            }
            ProviderEvent::ResponseId(_)
            | ProviderEvent::ServiceTier(_)
            | ProviderEvent::Signature { .. }
            | ProviderEvent::End { .. } => {}
        }
    }

    fn write_text(&mut self, key: &str, text: &str, replace: bool) {
        let key = self.block_key(key);
        let index = self
            .blocks
            .iter()
            .position(|block| block["key"] == key)
            .unwrap_or_else(|| {
                self.blocks.push(json!({"kind":"text","key":key,"text":""}));
                self.blocks.len() - 1
            });
        let kind = self.blocks[index]["kind"].as_str().unwrap_or("text");
        self.phase = if kind == "thinking" {
            "thinking"
        } else {
            "generating"
        };
        self.first_delta_ms.get_or_insert_with(now_ms);

        let previous = self.blocks[index]["text"].as_str().unwrap_or_default();
        let shown_elsewhere = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, block)| block["text"].as_str().map_or(0, str::len))
            .sum::<usize>();
        let prefix = if replace { "" } else { previous };
        let available = LIVE_TEXT_BYTES.saturating_sub(shown_elsewhere + prefix.len());
        let addition = utf8_prefix(text, available);
        let limited = addition.len() < text.len()
            || shown_elsewhere + prefix.len() + text.len() > LIVE_TEXT_BYTES;
        self.blocks[index]["text"] = json!(format!("{prefix}{addition}"));
        if limited {
            self.blocks[index]["limited"] = json!(true);
        }
        self.touch();
    }
}

struct ActiveSession {
    id: SessionId,
    model: Value,
    session: Arc<NativeSession>,
    // General chat has no filesystem capability. A project chat carries one
    // verified root for the UI file browser; model tools remain separate work.
    workspace: Option<Arc<Workspace>>,
    folders: Option<Arc<FolderAccess>>,
    workspace_path: PathBuf,
    workspace_scope: String,
    // The composer sends this immediately before a prompt. It is kept only in
    // memory so a permissive choice never leaks into a new chat or restart.
    approval_mode: String,
    busy: bool,
    loading: bool,
    progress: String,
    diagnostic: Option<String>,
    cancel: Option<Arc<CancelSource>>,
    cancel_token: Option<ag1_platform::provider::http::CancelToken>,
    live: Option<LiveActivity>,
    last_usage: Option<Value>,
}

struct DiffReview {
    session: String,
    root: PathBuf,
    sha256: String,
    bytes: Vec<u8>,
}

enum ChangeDiff {
    Bytes(Vec<u8>),
    Unavailable(&'static str),
}

struct Inner {
    state: PathBuf,
    store: Arc<Store>,
    config: Mutex<Option<HostConfig>>,
    active: Mutex<Option<ActiveSession>>,
    runtime: Mutex<Option<Arc<dyn GenerationRuntime>>>,
    uploads: Mutex<BTreeMap<String, PendingUpload>>,
    uploaded_messages: Mutex<BTreeMap<String, UploadedMessage>>,
    diff_review: Mutex<Option<DiffReview>>,
    last_terminal: Mutex<Option<Value>>,
    scheduler: Scheduler,
    next: AtomicU64,
}

// The browser renderer owns the file picker and clipboard. Keep incomplete
// uploads only in memory so cancelled drafts never become durable artifacts.
struct PendingUpload {
    session: String,
    bytes: Vec<u8>,
}

// A finished upload lasts only until the matching prompt is admitted. The
// durable journal becomes the source of truth once the prompt starts.
struct UploadedMessage {
    session: String,
    message: Value,
}

/// The desktop-facing native host. `Store` and `NativeSession` remain the
/// source of truth; this type only selects models and builds UI projections.
#[derive(Clone)]
pub struct NativeHost {
    inner: Arc<Inner>,
}

impl NativeHost {
    /// Opening the desktop never loads a model. It only reads public model
    /// metadata from the private config and opens durable local storage.
    pub fn open(state: PathBuf) -> Result<Self, String> {
        let config = config_path(&state);
        Self::open_from_config(state, config)
    }

    // Keeping the file selection outside `open` gives deterministic tests and
    // lets a future installer validate a candidate catalog before publishing it.
    fn open_from_config(state: PathBuf, config_path: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&state).map_err(|error| format!("Create AG1W state: {error}"))?;
        let store = Store::open(&state.join("store")).map_err(describe)?;
        let config = read_config(&config_path)?;
        Ok(Self {
            inner: Arc::new(Inner {
                state,
                store: Arc::new(store),
                config: Mutex::new(config),
                active: Mutex::new(None),
                runtime: Mutex::new(None),
                uploads: Mutex::new(BTreeMap::new()),
                uploaded_messages: Mutex::new(BTreeMap::new()),
                diff_review: Mutex::new(None),
                last_terminal: Mutex::new(None),
                scheduler: Scheduler::default(),
                next: AtomicU64::new(0),
            }),
        })
    }

    pub fn state_path(&self) -> &Path {
        &self.inner.state
    }

    pub fn models(&self) -> Result<Value, String> {
        let config = self.config()?;
        let models = config
            .as_ref()
            .map(|config| config.models.iter().map(public_model).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(json!(models))
    }

    pub fn preferences(&self) -> Value {
        let mode = self
            .active()
            .ok()
            .and_then(|active| active.as_ref().map(|active| active.approval_mode.clone()))
            .unwrap_or_else(|| "interactive".into());
        json!({"approval_mode":mode})
    }

    /// Return the capability for one configured local model. Keeping this in
    /// the host means the picker and the model catalog cannot disagree.
    pub fn reasoning_controls(&self, provider: &str, id: &str) -> Result<Value, String> {
        let config = self.config()?;
        let model = config
            .as_ref()
            .and_then(|config| {
                config.models.iter().find(|model| {
                    model["provider"].as_str() == Some(provider) && model["id"].as_str() == Some(id)
                })
            })
            .ok_or("That model is not in AG1W's local catalog")?;
        Ok(model_reasoning_controls(model))
    }

    /// Give the sidebar the current native chat's state without rebuilding its
    /// history. AG1W owns one local generation at a time, so idle chats do not
    /// need entries here; the sidebar already has their durable metadata.
    pub fn conversation_statuses(&self) -> Result<Value, String> {
        let active = self.active()?;
        let Some(active) = active.as_ref() else {
            return Ok(json!([]));
        };
        let status = if active.busy {
            "streaming"
        } else if active.loading {
            "running"
        } else if active.diagnostic.is_some() {
            "failed"
        } else {
            "idle"
        };
        Ok(json!([{
            "session": active.id.as_str(),
            "workspace": active.workspace_path.to_string_lossy(),
            "status": status,
            "pending": 0,
            "error": active.diagnostic.clone(),
        }]))
    }

    /// Session navigation in the shared renderer waits for a named terminal
    /// receipt. Native forks settle synchronously after SQLite acknowledges
    /// them, so expose the last receipt without inventing a background worker.
    pub fn runtime_status(&self) -> Result<Value, String> {
        let last_terminal = self
            .inner
            .last_terminal
            .lock()
            .map_err(|_| "AG1W navigation status lock is unavailable".to_owned())?
            .clone();
        Ok(json!({"last_terminal":last_terminal}))
    }

    /// Record the user's explicit choice before a prompt starts. The mode is
    /// scoped to this open chat and governs only project-file tools.
    pub fn set_approval_policy(&self, mode: &str) -> Result<(), String> {
        if !matches!(mode, "interactive" | "autonomous" | "disabled") {
            return Err("Unknown approval mode".into());
        }
        self.ensure_session()?;
        let mut active = self.active()?;
        let active = active.as_mut().ok_or("Open a local chat first")?;
        if active.busy || active.loading {
            return Err("Choose tool approvals before the local model starts working".into());
        }
        active.approval_mode = mode.into();
        Ok(())
    }

    /// Update one idle chat's folder grants through the platform's durable
    /// identity-checked permission store.  The renderer supplies the desired
    /// change, never an already-resolved workspace or a filesystem handle.
    pub fn change_folder_access(&self, value: &Value) -> Result<Value, String> {
        let change: FolderChange = serde_json::from_value(value.clone())
            .map_err(|_| "Invalid Chat folders request".to_owned())?;
        let mut active = self.active()?;
        let active = active
            .as_mut()
            .ok_or("Open a project chat before changing Chat folders")?;
        if active.busy || active.loading {
            return Err("Wait for the local model to finish before changing Chat folders".into());
        }
        let folders = active
            .folders
            .as_ref()
            .ok_or("General chat has no project folder grants")?;
        let snapshot = folders.change(change).map_err(describe)?;
        let workspace = snapshot.primary();
        active.workspace_path = workspace.root().to_path_buf();
        active.workspace_scope = workspace.id().as_str().to_owned();
        active.workspace = Some(workspace);
        serde_json::to_value(snapshot.selection)
            .map_err(|_| "Encode Chat folder permissions".to_owned())
    }

    /// Store a user-selected working context limit in the durable journal.
    /// The model's configured maximum still wins; this lets one long-lived
    /// chat intentionally use a smaller working window without changing the
    /// catalog or silently affecting another chat.
    pub fn set_context_limit(&self, tokens: u64) -> Result<(), String> {
        if tokens < 1_024 {
            return Err("Working context must be at least 1024 tokens.".into());
        }
        let (session, session_id) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or("Open a local chat first")?;
            if active.busy || active.loading {
                return Err(
                    "Wait for the local model to finish before changing the working context".into(),
                );
            }
            let maximum = active.model["contextWindow"].as_u64().unwrap_or(0);
            if maximum == 0 || tokens > maximum {
                return Err(format!(
                    "Working context must be between 1024 and {maximum} tokens for this model."
                ));
            }
            (active.session.clone(), active.id.as_str().to_owned())
        };
        let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        session
            .select_context(
                &format!("{session_id}-context-{serial}"),
                &format!("{session_id}-context-level-{serial}"),
                &timestamp(now_ms()),
                tokens,
            )
            .map_err(describe)?;
        Ok(())
    }

    /// Accept the renderer's bounded, chunked attachment protocol. Images are
    /// parsed before they can become a model message, so a renderer bug cannot
    /// turn arbitrary bytes into a durable prompt.
    pub fn upload_attachment(&self, action: &str, args: &Value) -> Result<Value, String> {
        let (session, accepts_images) = self.attachment_session()?;
        if action == "upload_begin" {
            let mut uploads = self
                .inner
                .uploads
                .lock()
                .map_err(|_| "AG1W attachment upload lock is unavailable".to_owned())?;
            if uploads.len() >= 16 {
                return Err("Too many unfinished attachments. Finish or cancel one first.".into());
            }
            let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
            let upload = format!("upload-{}-{serial}", now_ms());
            uploads.insert(
                upload.clone(),
                PendingUpload {
                    session,
                    bytes: Vec::new(),
                },
            );
            return Ok(json!({"upload":upload,"offset":"0"}));
        }

        let upload = args["upload"]
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or("Missing attachment upload ID")?;
        match action {
            "upload_append" => {
                let offset = args["offset"]
                    .as_str()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or("Invalid attachment upload offset")?;
                let chunk = STANDARD
                    .decode(args["base64"].as_str().ok_or("Missing attachment data")?)
                    .map_err(|_| "Attachment data is not valid base64".to_owned())?;
                if chunk.len() > 32 * 1024 {
                    return Err("Attachment chunks must be at most 32 KiB.".into());
                }
                let mut uploads = self
                    .inner
                    .uploads
                    .lock()
                    .map_err(|_| "AG1W attachment upload lock is unavailable".to_owned())?;
                let pending = uploads
                    .get_mut(upload)
                    .ok_or("Attachment upload was not found")?;
                if pending.session != session {
                    return Err("Attachment upload belongs to a different chat.".into());
                }
                if offset != pending.bytes.len() {
                    return Err("Attachment upload arrived out of order.".into());
                }
                if pending.bytes.len() + chunk.len() > MAX_ATTACHMENT_UPLOAD_BYTES {
                    return Err("Attachments are limited to 4 MiB of message data.".into());
                }
                pending.bytes.extend(chunk);
                Ok(json!({"offset":pending.bytes.len().to_string()}))
            }
            "upload_finish" => {
                let mut uploads = self
                    .inner
                    .uploads
                    .lock()
                    .map_err(|_| "AG1W attachment upload lock is unavailable".to_owned())?;
                let pending = uploads
                    .get(upload)
                    .ok_or("Attachment upload was not found")?;
                if pending.session != session {
                    return Err("Attachment upload belongs to a different chat.".into());
                }
                if args["sha256"] != digest(&pending.bytes) {
                    return Err("Attachment upload checksum did not match.".into());
                }
                let message: Value = serde_json::from_slice(&pending.bytes)
                    .map_err(|_| "Attachment message is not valid JSON.".to_owned())?;
                validate_user_message(&message, accepts_images)?;
                let mut messages = self
                    .inner
                    .uploaded_messages
                    .lock()
                    .map_err(|_| "AG1W attachment message lock is unavailable".to_owned())?;
                if messages.len() >= 32 {
                    return Err(
                        "Too many completed attachment messages. Send or cancel one first.".into(),
                    );
                }

                // Keep the upload until there is room for the completed message.
                // Otherwise a full preview queue would silently throw away a paste.
                uploads.remove(upload);
                let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
                let artifact = format!("message-{serial}");
                messages.insert(artifact.clone(), UploadedMessage { session, message });
                Ok(json!({"artifact":artifact}))
            }
            "upload_cancel" => {
                let mut uploads = self
                    .inner
                    .uploads
                    .lock()
                    .map_err(|_| "AG1W attachment upload lock is unavailable".to_owned())?;
                if uploads
                    .get(upload)
                    .is_some_and(|pending| pending.session != session)
                {
                    return Err("Attachment upload belongs to a different chat.".into());
                }
                uploads.remove(upload);
                Ok(json!({"cancelled":true}))
            }
            _ => Err("Unknown attachment upload action.".into()),
        }
    }

    /// Return one small artifact catalog page for the selected chat. The
    /// catalog carries metadata only; the renderer must request bytes through
    /// `read_artifact`, which checks access again for every page.
    pub fn artifact_catalog(&self, args: &Value) -> Result<Value, String> {
        let before = catalog_position(args.get("before").unwrap_or(&Value::Null))?;
        let (session_id, branch, history) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or("Open a local chat first")?;
            (
                active.id.as_str().to_owned(),
                active.session.records().map_err(describe)?.as_ref().clone(),
                active.session.history_records().map_err(describe)?,
            )
        };

        let rows = if let Some(turn) = args["turn"].as_str() {
            if turn.is_empty() || turn.len() > 128 {
                return Err("Invalid turn ID for artifact review.".into());
            }
            let calls =
                ag1_platform::change_receipts::turn_calls(&branch, turn).map_err(describe)?;
            // A call ID must name one durable call across the whole session.
            // Without this check, a branch could make an old result look like
            // output from the turn the user is currently reviewing.
            let unique_calls = calls
                .into_iter()
                .filter(|call| count_tool_calls(&history, call) == 1)
                .collect::<Vec<_>>();
            self.inner
                .store
                .call_artifact_catalog(&session_id, &unique_calls, before.as_ref(), 20)
                .map_err(describe)?
        } else {
            self.inner
                .store
                .artifact_catalog(&session_id, before.as_ref(), 20, args["kind"].as_str())
                .map_err(describe)?
        };

        let next = rows.last().filter(|_| rows.len() == 20).map(|row| {
            json!({
                "created_at": row.position.created_at,
                "id": row.position.id,
                "origin": row.position.origin,
            })
        });
        let artifacts = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.position.id,
                    "kind": row.record["kind"],
                    "mime_type": row.record["mimeType"],
                    "bytes": row.record["bytes"],
                    "sha256": row.record["sha256"],
                    "created_at": row.position.created_at,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({"artifacts":artifacts,"next":next}))
    }

    /// Read at most one renderer-sized page after resolving the artifact's
    /// origin from the current chat's access closure. A renderer never gets to
    /// choose the origin itself.
    pub fn read_artifact(&self, artifact: &str, offset: u64, limit: u32) -> Result<Value, String> {
        if artifact.is_empty() || artifact.len() > 128 || limit == 0 || limit > 32_768 {
            return Err("Invalid artifact page request.".into());
        }
        let session_id = {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or("Open a local chat first")?
                .id
                .as_str()
                .to_owned()
        };
        let origin = self
            .inner
            .store
            .artifact_origin(&session_id, artifact)
            .map_err(describe)?;
        let page = self
            .inner
            .store
            .page(&session_id, &origin, artifact, offset, limit)
            .map_err(describe)?;
        Ok(json!({
            "artifact":artifact,
            "offset":page.offset.to_string(),
            "base64":STANDARD.encode(page.bytes),
            "next":page.next_offset.map(|next| next.to_string()),
        }))
    }

    /// Capture one immutable, bounded view of the selected project's tracked
    /// Git changes. The UI reads the snapshot in pages so a refresh cannot mix
    /// a file before and after an outside edit.
    pub fn workspace_diff(&self, args: &Value) -> Result<Value, String> {
        let offset = args["offset"]
            .as_str()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| "Invalid workspace diff offset".to_owned())?;
        let (session, root) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or("Open a project chat first")?;
            let workspace = active
                .workspace
                .as_ref()
                .ok_or("Open a project chat before reviewing Git changes")?;
            (
                active.id.as_str().to_owned(),
                workspace.root().to_path_buf(),
            )
        };

        if offset == 0 && args["sha256"].is_null() {
            let bytes = self.capture_workspace_diff(&root)?;
            let sha256 = digest(&bytes);
            *self
                .inner
                .diff_review
                .lock()
                .map_err(|_| "AG1W workspace review lock is unavailable".to_owned())? =
                Some(DiffReview {
                    session: session.clone(),
                    root: root.clone(),
                    sha256,
                    bytes,
                });
        }

        let review = self
            .inner
            .diff_review
            .lock()
            .map_err(|_| "AG1W workspace review lock is unavailable".to_owned())?;
        let review = review
            .as_ref()
            .ok_or("Refresh Git changes to create a review snapshot")?;
        if review.session != session
            || review.root != root
            || offset > review.bytes.len()
            || args["sha256"]
                .as_str()
                .is_some_and(|sha256| sha256 != review.sha256)
            || offset > 0 && args["sha256"].is_null()
        {
            return Err("The workspace review changed. Refresh and try again.".into());
        }
        let end = offset.saturating_add(24 * 1024).min(review.bytes.len());
        Ok(json!({
            "base64":STANDARD.encode(&review.bytes[offset..end]),
            "sha256":review.sha256,
            "bytes":review.bytes.len(),
            "next":(end < review.bytes.len()).then(|| end.to_string()),
        }))
    }

    /// List only the native file receipts whose model calls occurred after the
    /// selected user turn. A model result or renderer argument cannot invent a
    /// change here: every receipt is rebound to the current durable branch.
    pub fn turn_changes(&self, args: &Value) -> Result<Value, String> {
        let turn = args["turn"]
            .as_str()
            .filter(|turn| !turn.is_empty() && turn.len() <= 128)
            .ok_or("Invalid turn ID for file-change review")?;
        let offset = args["offset"].as_u64().unwrap_or(0);
        let offset = usize::try_from(offset).map_err(|_| "Invalid file-change page offset")?;
        let (session, records, workspace_scope, roots) = self.change_review_scope()?;
        let calls = change_receipts::turn_calls(&records, turn).map_err(describe)?;
        if offset > calls.len() {
            return Err("Invalid file-change page offset".into());
        }
        let end = offset.saturating_add(16).min(calls.len());
        let values = self
            .inner
            .store
            .native_file_changes(&session, &calls[offset..end])
            .map_err(describe)?;
        let mut changes = Vec::new();
        for value in values {
            let receipt: ChangeReceipt = serde_json::from_value(value.clone())
                .map_err(|_| "Invalid native file-change receipt".to_owned())?;
            if change_receipts::verify_branch(&receipt, &records, &session, &workspace_scope)
                .is_err()
                || !roots
                    .iter()
                    .any(|root| same_windows_path(Path::new(&receipt.root), root))
            {
                continue;
            }
            changes.push(change_receipts::summary(&value).map_err(describe)?);
        }
        Ok(json!({
            "turn":turn,
            "changes":changes,
            "next":(end < calls.len()).then_some(end),
            "scope":"selected_turn",
            "notice":"Native write/edit receipts only. Shell, rename and delete changes remain workspace changes.",
        }))
    }

    /// Read a bounded page of a receipt snapshot or display diff. The receipt
    /// must still belong to the active journal and project on every request.
    pub fn read_turn_change(&self, args: &Value) -> Result<Value, String> {
        let id = args["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Invalid file-change receipt ID")?;
        let offset = args["offset"]
            .as_str()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| "Invalid file-change read offset".to_owned())?;
        let (session, records, workspace_scope, roots) = self.change_review_scope()?;
        let value = self
            .inner
            .store
            .native_file_change(&session, id)
            .map_err(describe)?;
        let receipt: ChangeReceipt = serde_json::from_value(value)
            .map_err(|_| "Invalid native file-change receipt".to_owned())?;
        change_receipts::verify_branch(&receipt, &records, &session, &workspace_scope)
            .map_err(describe)?;
        if !roots
            .iter()
            .any(|root| same_windows_path(Path::new(&receipt.root), root))
        {
            return Err("That file-change receipt belongs to another project.".into());
        }

        let side = args["side"].as_str().unwrap_or("diff");
        let bytes = match side {
            "before" => receipt
                .before
                .as_ref()
                .map(|snapshot| {
                    change_receipts::read_snapshot(&self.inner.store, &session, snapshot)
                })
                .transpose()
                .map_err(describe)?
                .unwrap_or_default(),
            "after" => change_receipts::read_snapshot(&self.inner.store, &session, &receipt.after)
                .map_err(describe)?,
            "diff" => match self.change_diff(&receipt, &session)? {
                ChangeDiff::Bytes(bytes) => bytes,
                ChangeDiff::Unavailable(message) => return Ok(json!({"unavailable":message})),
            },
            _ => return Err("Choose before, after, or diff for file-change review.".into()),
        };
        if offset > bytes.len() {
            return Err("Invalid file-change read offset".into());
        }
        let end = offset.saturating_add(CHANGE_PAGE_BYTES).min(bytes.len());
        Ok(json!({
            "base64":STANDARD.encode(&bytes[offset..end]),
            "next":(end < bytes.len()).then(|| end.to_string()),
            "bytes":bytes.len(),
            "sha256":digest(&bytes),
            "side":side,
            "id":id,
            "state":receipt.state,
        }))
    }

    /// Describe whether a receipt can be reversed without guessing. The
    /// original bytes, revision, and recovery path all remain host-owned.
    pub fn change_undo_status(&self, args: &Value) -> Result<Value, String> {
        let (receipt, workspace, session, _, _) = self.change_undo_target(args)?;
        if let Some(outcome) = self
            .inner
            .store
            .native_file_undo(&session, &receipt.id)
            .map_err(describe)?
        {
            let recoverable = matches!(
                outcome["state"].as_str(),
                Some("prepared" | "recovery_required")
            ) && !workspace.exists(&receipt.path).unwrap_or(true)
                && outcome["recovery"]
                    .as_str()
                    .is_some_and(|path| workspace.exists(path).unwrap_or(false));
            return Ok(json!({
                "allowed":false,
                "outcome":outcome,
                "recoverable":recoverable,
            }));
        }
        let reason = if receipt.state != "committed" {
            Some("The native effect is not confirmed. Review its outcome before changing files.")
        } else if self.change_review_busy()? {
            Some("Finish or stop the current response before Undo.")
        } else if !receipt
            .after_revision
            .as_ref()
            .and_then(|revision| FileRevision::try_from(revision).ok())
            .is_some_and(|expected| {
                workspace
                    .snapshot(&receipt.path, MAX_READ)
                    .is_ok_and(|snapshot| snapshot.revision == expected)
            })
        {
            Some(
                "This file changed after the recorded operation. Undo would conflict with later work.",
            )
        } else {
            None
        };
        Ok(json!({
            "allowed":reason.is_none(),
            "reason":reason,
            "id":receipt.id,
            "path":receipt.path,
            "operation":if receipt.before.is_some(){"restore_original"}else{"remove_created"},
            "scope":"one_native_operation",
        }))
    }

    /// Apply the already-confirmed Undo protocol. This does not ask a model
    /// for paths or bytes; the selected receipt is the entire authority.
    pub fn change_undo(&self, args: &Value, recover: bool) -> Result<Value, String> {
        let (receipt, workspace, session, records, workspace_scope) =
            self.change_undo_target(args)?;
        if self.change_review_busy()? {
            return Err("Finish or stop the current response before Undo.".into());
        }
        let expected_id = session.clone();
        let expected_scope = workspace_scope.clone();
        let expected_root = workspace.root().to_path_buf();
        let host = self.clone();
        let deadline = Instant::now() + Duration::from_secs(10);
        let check = move |_| -> Result<(), DomainError> {
            if Instant::now() >= deadline {
                return Err(DomainError::TimedOut);
            }
            let active = host.active().map_err(|_| DomainError::Conflict)?;
            let active = active.as_ref().ok_or(DomainError::Conflict)?;
            if active.busy
                || active.loading
                || active.id.as_str() != expected_id
                || active.workspace_scope != expected_scope
                || NativeHost::active_has_granted_root(active, &expected_root).is_err()
            {
                return Err(DomainError::Conflict);
            }
            Ok(())
        };
        let result = if recover {
            ag1_platform::change_undo::recover(
                self.inner.store.as_ref(),
                workspace.as_ref(),
                &receipt,
                &records,
                &session,
                &workspace_scope,
                check,
            )
        } else {
            ag1_platform::change_undo::execute(
                self.inner.store.as_ref(),
                workspace.as_ref(),
                &receipt,
                &records,
                &session,
                &workspace_scope,
                check,
            )
        };
        result.map_err(describe)
    }

    /// Start a prompt using the message assembled by the renderer. This is
    /// separate from `upload_attachment` so an image can be pasted, previewed,
    /// and removed before the user presses Send.
    pub fn begin_uploaded_prompt(&self, artifact: &str) -> Result<Value, String> {
        if artifact.is_empty() || artifact.len() > 128 {
            return Err("Invalid attachment message ID.".into());
        }
        let session = self.attachment_session()?.0;
        let message = self
            .inner
            .uploaded_messages
            .lock()
            .map_err(|_| "AG1W attachment message lock is unavailable".to_owned())?
            .get(artifact)
            .filter(|stored| stored.session == session)
            .map(|stored| stored.message.clone())
            .ok_or("Attachment message was not found for this chat.")?;
        self.begin_message_prompt(&message)?;
        self.inner
            .uploaded_messages
            .lock()
            .map_err(|_| "AG1W attachment message lock is unavailable".to_owned())?
            .remove(artifact);
        Ok(message)
    }

    pub fn open_general(&self) -> Result<Value, String> {
        self.open_workspace(None)?;
        self.view()
    }

    /// Open a user-selected Windows project. Registration captures the
    /// directory identity now, so later file reads cannot escape through a
    /// junction or silently follow a moved root.
    pub fn open_project(&self, path: PathBuf) -> Result<Value, String> {
        let service = WorkspaceService::new().map_err(describe)?;
        let workspace = service.register(&path).map_err(describe)?;
        self.open_workspace(Some(workspace))?;
        self.view()
    }

    pub fn new_chat(&self) -> Result<Value, String> {
        let mut active = self.active()?;
        if active
            .as_ref()
            .is_some_and(|session| session.busy || session.loading)
        {
            return Err("Wait for the local model to finish before starting a new chat".into());
        }
        // A New chat stays in the same user-selected project. It creates a
        // fresh journal but does not alter which files the user granted.
        let workspace = active
            .as_ref()
            .and_then(|session| session.workspace.clone());
        *active = None;
        drop(active);
        self.create_session(workspace)?;
        self.view()
    }

    /// Forking creates another durable chat rather than mutating the current
    /// branch in memory. The original session stays selectable in the sidebar,
    /// while the new one inherits only records before the chosen user message.
    pub fn fork_before_user_message(&self, entry: &str) -> Result<Value, String> {
        if entry.is_empty() || entry.len() > 128 {
            return Err("Invalid user message for a local branch.".into());
        }
        let (parent_id, parent, model, workspace) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or("Open a local chat first")?;
            if active.busy || active.loading {
                return Err("Wait for the local model to finish before branching this chat".into());
            }
            (
                active.id.clone(),
                active.session.clone(),
                active.model.clone(),
                active.workspace.clone(),
            )
        };
        let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        let id = SessionId::new(format!("win-session-{}-{serial}", now_ms()))
            .map_err(|error| format!("Create Windows branch ID: {error:?}"))?;
        let origin = session_origin(&id);
        let stamp = timestamp(now_ms());
        let session = parent
            .fork_before_user_message(id.clone(), &origin, &stamp, entry)
            .map_err(describe)?;
        let operation = format!("native-fork-{}", id.as_str());
        // A fork gets its own grant record. It starts with the current working
        // folder only, so an old conversation's extra folder access cannot
        // silently spread into a new branch.
        let folders = workspace
            .as_ref()
            .map(|workspace| FolderAccess::open(workspace.clone(), &self.inner.state, id.as_str()))
            .transpose()
            .map_err(describe)?;
        let workspace = folders
            .as_ref()
            .map(|folders| folders.snapshot().map(|snapshot| snapshot.primary()))
            .transpose()
            .map_err(describe)?;
        let (workspace_path, workspace_scope) = self.workspace_view(&workspace);

        // The storage fork checks the parent's revision. Check the active
        // selection again before publication so an older UI command cannot
        // replace a different chat that was opened while the fork was saved.
        let mut active = self.active()?;
        if active
            .as_ref()
            .is_none_or(|current| current.id != parent_id || current.busy || current.loading)
        {
            return Err("The chat changed before this branch could open. The saved branch remains in history.".into());
        }
        *active = Some(ActiveSession {
            id,
            model,
            session: Arc::new(session),
            workspace,
            folders,
            workspace_path,
            workspace_scope,
            // Branches start conservative. An auto-approve choice is scoped to
            // the original chat and must never silently carry into a new one.
            approval_mode: "interactive".into(),
            busy: false,
            loading: false,
            progress: String::new(),
            diagnostic: None,
            cancel: None,
            cancel_token: None,
            live: None,
            last_usage: None,
        });
        drop(active);
        *self
            .inner
            .last_terminal
            .lock()
            .map_err(|_| "AG1W navigation status lock is unavailable".to_owned())? =
            Some(json!({"operation":operation,"status":"completed"}));
        Ok(json!({"operation":operation}))
    }

    /// Switch the current chat to an earlier durable branch point. A selected
    /// user message becomes composer text, so the active branch ends before
    /// it. A summary request is accepted first and finishes in the background
    /// because a local model may take time to write the durable summary.
    pub fn select_history_entry(&self, target: &str, summarize: bool) -> Result<Value, String> {
        if target.is_empty() || target.len() > 128 {
            return Err("Invalid local branch target.".into());
        }
        if summarize {
            return Err(
                "Start this branch summary through the asynchronous native command.".into(),
            );
        }
        let (session, session_id) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or("Open a local chat first")?;
            if active.busy || active.loading {
                return Err(
                    "Wait for the local model to finish before changing this branch".into(),
                );
            }
            (active.session.clone(), active.id.as_str().to_owned())
        };
        let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        let operation = format!("{session_id}-tree-{serial}");
        session
            .select_history_entry(&operation, target)
            .map_err(describe)?;
        *self
            .inner
            .last_terminal
            .lock()
            .map_err(|_| "AG1W navigation status lock is unavailable".to_owned())? =
            Some(json!({"operation":operation,"status":"completed"}));
        Ok(json!({"operation":operation}))
    }

    /// Reserve a tree selection so the renderer can return immediately and
    /// poll the normal runtime status while an optional summary is generated.
    pub async fn begin_history_selection(
        &self,
        target: &str,
        summarize: bool,
    ) -> Result<Value, String> {
        if target.is_empty() || target.len() > 128 {
            return Err("Invalid local branch target.".into());
        }
        if !summarize {
            return self.select_history_entry(target, false);
        }

        // An attached loopback server needs no startup work. An AG1W-owned
        // server, however, must pass its sealed runtime checks before we tell
        // the renderer that the summary has started.
        self.ensure_runtime().await?;
        let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        let stamp = now_ms();
        let (cancel_source, cancel_token) = cancellation();
        let operation = {
            let mut active = self.active()?;
            let active = active.as_mut().ok_or("Open a local chat first")?;
            if active.busy || active.loading {
                return Err(
                    "Wait for the local model to finish before changing this branch".into(),
                );
            }
            active.busy = true;
            active.progress = "Summarizing the abandoned branch".into();
            active.diagnostic = None;
            active.last_usage = None;
            active.cancel = Some(Arc::new(cancel_source));
            active.cancel_token = Some(cancel_token);
            active.live = Some(LiveActivity::new(
                format!("{}-tree-{stamp}", active.id.as_str()),
                stamp,
            ));
            format!("{}-tree-{serial}", active.id.as_str())
        };
        *self
            .inner
            .last_terminal
            .lock()
            .map_err(|_| "AG1W navigation status lock is unavailable".to_owned())? =
            Some(json!({"operation":operation,"status":"running"}));
        Ok(json!({"operation":operation,"pending":true}))
    }

    /// Run a previously admitted branch summary. It shares normal prompt
    /// cancellation, scheduler admission, and live progress so Stop works
    /// even though this generation does not create a chat reply.
    pub async fn run_started_history_selection(
        &self,
        target: String,
        operation: String,
    ) -> Result<(), String> {
        let outcome = async {
            let (session, model, runtime, stamp, mut cancel) = {
                let mut active = self.active()?;
                let active = active.as_mut().ok_or("Open a local chat first")?;
                if !active.busy || active.loading {
                    return Err("The local branch summary is no longer active".into());
                }
                let cancel = active
                    .cancel_token
                    .take()
                    .ok_or("The local branch summary has no cancellation token")?;
                (
                    active.session.clone(),
                    active.model.clone(),
                    self.runtime()?,
                    now_ms(),
                    cancel,
                )
            };
            let base_url = model["baseUrl"]
                .as_str()
                .ok_or("Selected local model has no baseUrl")?
                .to_owned();
            let origin = endpoint_origin(&base_url).map_err(describe)?;
            let mut http =
                ProviderHttp::new(&[origin], Duration::from_secs(10)).map_err(describe)?;
            if let Some(runtime) = runtime {
                http = http.with_runtime(runtime);
            }
            let host = self.clone();
            session
                .select_history_entry_with_summary(
                    &operation,
                    &target,
                    &timestamp(stamp),
                    &self.inner.scheduler,
                    &http,
                    &mut cancel,
                    move |event| {
                        host.note_event(event);
                        Ok(())
                    },
                )
                .await
                .map_err(describe)?;
            Ok(())
        }
        .await;

        self.finish_prompt(outcome.as_ref().err().cloned());
        let status = if outcome.is_ok() {
            "completed"
        } else {
            "failed"
        };
        *self
            .inner
            .last_terminal
            .lock()
            .map_err(|_| "AG1W navigation status lock is unavailable".to_owned())? =
            Some(json!({"operation":operation,"status":status}));
        outcome
    }

    /// Reopen an existing, durable journal. A session keeps the local model it
    /// was created with, so changing the picker never rewrites old history.
    pub fn open_session(&self, value: &str) -> Result<Value, String> {
        let id = SessionId::new(value.to_owned())
            .map_err(|_| "Invalid local chat identifier".to_owned())?;
        {
            let active = self.active()?;
            if active
                .as_ref()
                .is_some_and(|session| session.busy || session.loading)
            {
                return Err("Wait for the local model to finish before changing chats".into());
            }
            if active.as_ref().is_some_and(|session| session.id == id) {
                drop(active);
                return self.view();
            }
        }

        let (fallback_model, system_prompt, configured) = self.selected_configuration()?;
        let journal = self.inner.store.journal_snapshot(&id).map_err(describe)?;
        let header: Value = serde_json::from_slice(&journal.header)
            .map_err(|_| "Invalid durable local chat header".to_owned())?;
        let saved_directory = header["cwd"]
            .as_str()
            .ok_or("Durable local chat has no project location")?;
        let (workspace, _, _) = self.restore_workspace(saved_directory)?;
        // The durable journal remembers the folder that created the chat.
        // The chat-folder record can later select another granted folder as
        // working root, so reopen that record before constructing the tools.
        let folders = workspace
            .as_ref()
            .map(|workspace| FolderAccess::open(workspace.clone(), &self.inner.state, id.as_str()))
            .transpose()
            .map_err(describe)?;
        let workspace = folders
            .as_ref()
            .map(|folders| folders.snapshot().map(|snapshot| snapshot.primary()))
            .transpose()
            .map_err(describe)?;
        let (workspace_path, workspace_scope) = self.workspace_view(&workspace);
        // Older AG1W journals all used one origin. New journals record a
        // session-specific origin in their header so separate chats cannot
        // collide while retaining backwards compatibility here.
        let origin = header["ag1wOrigin"]
            .as_str()
            .unwrap_or(LEGACY_ORIGIN)
            .to_owned();
        let session = NativeSession::resume(
            self.inner.store.clone(),
            id.clone(),
            &origin,
            1,
            session_configuration(fallback_model, &system_prompt, workspace.is_some())?,
        )
        .map_err(describe)?;
        let model = session.configuration().map_err(describe)?.model;

        // The journal chooses its original model, but the current catalog must
        // still explicitly authorize that endpoint before AG1W can talk to it.
        if !configured
            .iter()
            .any(|candidate| same_local_model(candidate, &model))
        {
            return Err(
                "This chat's local model is no longer present in AG1W's native configuration"
                    .into(),
            );
        }

        *self.active()? = Some(ActiveSession {
            id,
            model,
            session: Arc::new(session),
            workspace,
            folders,
            workspace_path,
            workspace_scope,
            approval_mode: "interactive".into(),
            busy: false,
            loading: false,
            progress: String::new(),
            diagnostic: None,
            cancel: None,
            cancel_token: None,
            live: None,
            last_usage: None,
        });
        self.view()
    }

    pub fn select_model(&self, provider: &str, id: &str) -> Result<(), String> {
        {
            let active = self.active()?;
            if active
                .as_ref()
                .is_some_and(|session| session.busy || session.loading)
            {
                return Err("Wait for the local model to finish before changing models".into());
            }
        }
        let changed = {
            let mut config = self.config()?;
            let config = config
                .as_mut()
                .ok_or("Add a native local model configuration before selecting a model")?;
            let valid = config.models.iter().any(|model| {
                model["provider"].as_str() == Some(provider) && model["id"].as_str() == Some(id)
            });
            if !valid {
                return Err("That model is not present in AG1W's native configuration".into());
            }
            if config.selected == id {
                false
            } else {
                config.selected = id.into();
                if std::env::var_os("AG1_NATIVE_CONFIG").is_none() {
                    write_config(&self.inner.state, config)?;
                }
                true
            }
        };
        if changed {
            // A durable journal keeps the model it began with. Start a fresh
            // journal in the same project instead of silently rewriting old
            // history or leaving the shared UI without an active chat.
            let workspace = {
                let mut active = self.active()?;
                active.take().and_then(|session| session.workspace)
            };
            self.create_session(workspace)?;
        }
        Ok(())
    }

    /// Seal one Windows llama.cpp server, model and tuning profile into this
    /// desktop state. This only reads and hashes files; it never starts a model.
    pub async fn configure_owned_runtime(&self, input: NativeModelSetup) -> Result<Value, String> {
        self.configure_owned_runtimes(vec![input]).await
    }

    /// Add one or more models to one owned llama.cpp runtime as a single
    /// publish operation. If a later GGUF fails verification, the picker keeps
    /// its previous configuration instead of showing a half-added model list.
    pub async fn configure_owned_runtimes(
        &self,
        inputs: Vec<NativeModelSetup>,
    ) -> Result<Value, String> {
        if inputs.is_empty() {
            return Err("Choose at least one native model to configure.".into());
        }
        if std::env::var_os("AG1_NATIVE_CONFIG").is_some() {
            return Err(
                "AG1W is using an externally supplied native configuration. Remove AG1_NATIVE_CONFIG before replacing its local runtime setup."
                    .into(),
            );
        }
        if self.runtime()?.is_some() {
            return Err("Stop and restart AG1W before replacing a runtime it already owns.".into());
        }
        if self.active()?.is_some() {
            return Err(
                "Create the native model setup before opening a chat, then restart AG1W so existing chat history keeps its original model configuration."
                    .into(),
            );
        }

        // Copy the catalog before hashing. This lets a second model join the
        // same owned server without replacing the model the user already set up.
        let existing = self.config()?.clone();
        let state = self.inner.state.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let mut prepared = existing;
            for input in inputs {
                prepared = Some(prepare_runtime_setup(&state, prepared.as_ref(), input)?);
            }
            prepared.ok_or("AG1W did not receive a native model setup.".to_owned())
        })
        .await
        .map_err(|_| "AG1W runtime setup worker stopped unexpectedly".to_owned())??;

        // A setup operation can take a while to hash a GGUF. Check again
        // before publishing so a chat opened meanwhile cannot be retargeted.
        if self.runtime()?.is_some() || self.active()?.is_some() {
            return Err(
                "A local chat opened while AG1W was preparing the runtime. The verified setup was not published; restart and try again."
                    .into(),
            );
        }
        write_config(&self.inner.state, &prepared)?;
        *self.config()? = Some(prepared);
        self.models()
    }

    /// Reserve the chat before the command returns to the desktop. The actual
    /// provider work runs afterward, so the UI can poll a real live response.
    pub fn begin_prompt(&self, text: &str) -> Result<(), String> {
        self.begin_message_prompt(&inline_message(text))
    }

    /// Reserve the chat for one already-validated user message. Keeping this
    /// path shared makes pasted images obey the same stop and busy rules as
    /// plain text.
    pub fn begin_message_prompt(&self, message: &Value) -> Result<(), String> {
        self.ensure_session()?;
        let mut active = self.active()?;
        let active = active.as_mut().ok_or("Open a local chat first")?;
        if active.busy || active.loading {
            return Err("The local model is already working on this chat".into());
        }
        validate_user_message(message, model_accepts_images(&active.model))?;
        let stamp = now_ms();
        let (cancel_source, cancel_token) = cancellation();
        active.busy = true;
        active.progress = "Preparing local request".into();
        active.diagnostic = None;
        active.last_usage = None;
        active.cancel = Some(Arc::new(cancel_source));
        active.cancel_token = Some(cancel_token);
        active.live = Some(LiveActivity::new(
            format!("{}-{stamp}", active.id.as_str()),
            stamp,
        ));
        Ok(())
    }

    /// Keep the direct Rust API useful for tests and callers which do not use
    /// Tauri's command queue. The desktop itself calls `begin_prompt` and then
    /// starts `run_started_prompt` on its own runtime.
    pub async fn prompt(&self, text: String) -> Result<Value, String> {
        self.begin_prompt(&text)?;
        self.run_started_prompt(text).await
    }

    pub async fn run_started_prompt(&self, text: String) -> Result<Value, String> {
        self.run_started_message(inline_message(&text)).await
    }

    /// Run the specific content blocks which were admitted by the desktop.
    /// The journal retains these blocks unchanged, including image bytes.
    pub async fn run_started_message(&self, message: Value) -> Result<Value, String> {
        let outcome = async {
            self.ensure_runtime().await?;
            let (
                session,
                model,
                runtime,
                tools,
                command,
                user_id,
                assistant_id,
                timestamp,
                timestamp_ms,
                mut cancel,
            ) = {
                let mut active = self.active()?;
                let active = active.as_mut().ok_or("Open a local chat first")?;
                if !active.busy || active.loading {
                    return Err("The local generation is no longer active".into());
                }
                let cancel = active
                    .cancel_token
                    .take()
                    .ok_or("The local generation has no cancellation token")?;
                let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
                let stamp = now_ms();
                let session_id = active.id.as_str().to_owned();
                (
                    active.session.clone(),
                    active.model.clone(),
                    self.runtime()?,
                    active
                        .folders
                        .as_ref()
                        .map(|folders| {
                            // Freeze the grants with prompt admission. A
                            // renderer click while the model is working must
                            // not alter the set of paths that turn can reach.
                            let snapshot = folders.snapshot().map_err(describe)?;
                            let write_access = project_write_access(&active.approval_mode);
                            Ok::<LocalProjectTools, String>(LocalProjectTools::with_folder_access(
                                snapshot,
                                write_access,
                                self.inner.state.join("project-command-logs"),
                            ))
                        })
                        .transpose()?,
                    format!("{session_id}-turn-{serial}"),
                    format!("{session_id}-user-{serial}"),
                    format!("{session_id}-assistant-{serial}"),
                    timestamp(stamp),
                    stamp,
                    cancel,
                )
            };

            let base_url = model["baseUrl"]
                .as_str()
                .ok_or("Selected local model has no baseUrl")?
                .to_owned();
            // Requests retain the full base URL (including /v1), while ProviderHttp
            // allow-lists origins. Giving it the full URL rejects its own request
            // before it can reach the loopback llama.cpp server.
            let origin = endpoint_origin(&base_url).map_err(describe)?;
            let mut http =
                ProviderHttp::new(&[origin], Duration::from_secs(10)).map_err(describe)?;
            if let Some(runtime) = runtime {
                http = http.with_runtime(runtime);
            }
            let host = self.clone();
            session
                .run_local_turn_with_tools(
                    LocalTurnInput {
                        command,
                        user_id,
                        assistant_id,
                        timestamp,
                        timestamp_ms,
                        message,
                    },
                    &self.inner.scheduler,
                    &http,
                    &mut cancel,
                    tools.as_ref(),
                    move |event| {
                        host.note_event(event);
                        Ok(())
                    },
                )
                .await
                .map_err(describe)?;
            Ok(())
        }
        .await;

        self.finish_prompt(outcome.as_ref().err().cloned());
        outcome?;
        self.view()
    }

    fn finish_prompt(&self, diagnostic: Option<String>) {
        if let Ok(mut active) = self.active()
            && let Some(active) = active.as_mut()
        {
            active.busy = false;
            active.progress.clear();
            active.cancel = None;
            active.cancel_token = None;
            active.last_usage = active
                .live
                .as_ref()
                .filter(|live| !live.usage.as_object().is_none_or(|usage| usage.is_empty()))
                .map(|live| usage_with_rate(live.usage.clone(), live.first_delta_ms, now_ms()));
            active.live = None;
            if diagnostic.is_some() {
                active.diagnostic = diagnostic;
            }
        }
    }

    /// Reserve the current chat for an explicit model load. The slow work runs
    /// after the desktop command returns, so the UI can keep polling progress
    /// instead of timing out while llama.cpp maps a large GGUF.
    pub fn begin_model_load(&self, provider: &str, id: &str) -> Result<(), String> {
        self.ensure_session()?;
        let mut active = self.active()?;
        let active = active.as_mut().ok_or("Open a local chat first")?;
        if active.busy || active.loading {
            return Err("The local model is already working on this chat".into());
        }
        if active.model["provider"].as_str() != Some(provider)
            || active.model["id"].as_str() != Some(id)
        {
            return Err("Choose that model before loading it".into());
        }
        active.loading = true;
        active.progress = format!("Preparing owned local model · {id}");
        active.diagnostic = None;
        Ok(())
    }

    /// Complete a model load reserved by `begin_model_load`. This method owns
    /// the terminal state so either a manual load or a normal first prompt
    /// leaves the chat usable again after a server error.
    pub async fn run_started_model_load(&self, id: String) -> Result<Value, String> {
        let outcome = async {
            self.ensure_runtime().await?;
            let runtime = self.runtime()?.ok_or(
                "Loading requires a sealed AG1W runtime capture. An attached local server can answer prompts, but AG1W does not own or load it.",
            )?;
            runtime.load_model(&id).await.map_err(describe)
        }
        .await;
        let mut active = self.active()?;
        if let Some(active) = active.as_mut() {
            active.loading = false;
            active.progress.clear();
            if let Err(error) = &outcome {
                active.diagnostic = Some(describe(error.clone()));
            }
        }
        drop(active);
        outcome?;
        self.view()
    }

    /// Keep the direct Rust API useful for tests and callers that do not use
    /// the Tauri command queue. The desktop uses the two-stage form above.
    pub async fn load_model(&self, provider: &str, id: &str) -> Result<Value, String> {
        self.begin_model_load(provider, id)?;
        self.run_started_model_load(id.to_owned()).await
    }

    /// Stop the current local request. The cancellation token exists before
    /// server startup, so Esc works during preparation as well as generation.
    pub fn cancel(&self) -> Result<(), String> {
        let mut active = self.active()?;
        let active = active.as_mut().ok_or("No local chat is open")?;
        let cancel = active
            .cancel
            .as_ref()
            .ok_or("The local model is not generating")?;
        cancel.cancel();
        active.progress = "Stopping local generation".into();
        Ok(())
    }

    /// The project browser is a deliberately small native capability. It uses
    /// the same checked Workspace that protects future tool access, but it
    /// does not expose files to the model or run any command.
    pub fn list_files(&self) -> Result<Value, String> {
        let workspace = self.project_workspace()?;
        Ok(json!(workspace.scan(1_000).map_err(editor_error)?))
    }

    pub fn read_file(&self, path: &str) -> Result<Value, String> {
        let workspace = self.project_workspace()?;
        let current = workspace.snapshot(path, 32 * 1024).map_err(editor_error)?;
        let text = String::from_utf8(current.bytes)
            .map_err(|_| "This is a binary file; use an artifact view instead.".to_owned())?;
        Ok(json!({
            "path": path,
            "text": text,
            "revision": editor_revision(&current.revision)?,
        }))
    }

    pub fn save_file(&self, path: &str, text: &str, token: &str) -> Result<Value, String> {
        if text.len() > 32 * 1024 {
            return Err("Editor save exceeds the 32 KiB native bridge limit.".into());
        }
        let workspace = self.project_workspace()?;
        let current = workspace.snapshot(path, 32 * 1024).map_err(editor_error)?;
        if editor_revision(&current.revision)? != token {
            return Err("File conflict: disk changed; your unsaved text is retained.".into());
        }
        // Retain the file's BOM and line-ending convention so an ordinary
        // save from the editor does not create an unrelated whole-file diff.
        let bytes = preserve_text(&current, text).map_err(editor_error)?;
        if bytes.len() > 32 * 1024 {
            return Err("Editor save exceeds the 32 KiB native bridge limit.".into());
        }
        workspace
            .replace(path, &current.revision, &bytes)
            .map_err(editor_error)?;
        self.read_file(path)
    }

    /// Export the currently selected durable branch to a new HTML file in the
    /// opened project. The journal remains authoritative; image bytes stay in
    /// its private artifacts rather than being copied into an ordinary file.
    pub fn export_session_html(&self, path: &str) -> Result<Value, String> {
        let filename = Path::new(path);
        if path.is_empty()
            || path.len() > 240
            || !path.to_ascii_lowercase().ends_with(".html")
            || filename.components().count() != 1
        {
            return Err("Choose one new .html filename in the opened project.".into());
        }
        let (session, workspace) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or("Open a local chat first")?;
            if active.busy || active.loading {
                return Err(
                    "Wait for the local model to finish before exporting this chat.".into(),
                );
            }
            let workspace = active.workspace.clone().ok_or_else(|| {
                "Open a project before exporting. General chat has no project folder.".to_owned()
            })?;
            (active.session.clone(), workspace)
        };
        let records = session.records().map_err(describe)?;
        let html = session_export_html(&records).map_err(describe)?;
        let revision = workspace
            .create_checked(path, html.as_bytes(), false, || Ok(()))
            .map_err(editor_error)?;
        Ok(json!({
            "path":path,
            "sha256":revision.sha256,
            "bytes":html.len(),
            "messages":records.iter().filter(|record| record["type"] == "message").count(),
            "notice":"Complete active-branch text; binary attachments remain in journal custody",
        }))
    }

    /// Rebuild sidebar rows from durable journals. The catalog is only UI
    /// metadata; journal headers and entries remain the source of truth.
    pub fn catalog_sessions(&self, offset: usize, current_only: bool) -> Result<Value, String> {
        let ids = if current_only {
            self.active()?
                .as_ref()
                .map(|active| vec![active.id.as_str().to_owned()])
                .unwrap_or_default()
        } else {
            self.inner.store.client_sessions(1_000).map_err(describe)?
        };
        let mut chats = Vec::new();
        for id in ids.iter().skip(offset).take(50) {
            let session = SessionId::new(id.clone()).map_err(describe)?;
            let journal = self
                .inner
                .store
                .journal_snapshot(&session)
                .map_err(describe)?;
            let header: Value = serde_json::from_slice(&journal.header)
                .map_err(|_| "Invalid durable local chat header".to_owned())?;
            let saved = header["cwd"]
                .as_str()
                .ok_or("Durable local chat has no project location")?;
            let workspace = self.catalog_workspace(id, saved);
            let records = journal
                .entries
                .iter()
                .filter_map(|entry| serde_json::from_slice::<Value>(&entry.normalized).ok())
                .collect::<Vec<_>>();
            let title = records
                .iter()
                .find_map(project_message)
                .filter(|message| message["role"] == "user")
                .and_then(|message| message["text"].as_str().map(str::to_owned))
                .map(|text| text.chars().take(90).collect::<String>())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "New chat".into());
            let empty = !records
                .iter()
                .filter_map(project_message)
                .any(|message| message["role"] == "user");
            let updated = records
                .last()
                .and_then(|record| record["timestamp"].as_str())
                .unwrap_or_else(|| header["timestamp"].as_str().unwrap_or(""));
            chats.push(json!({
                "session": id,
                "workspace": workspace,
                "title": title,
                "empty": empty,
                "updated": updated,
            }));
        }
        Ok(json!({
            "chats": chats,
            "next": if offset + chats.len() < ids.len() { Some(offset + chats.len()) } else { None::<usize> },
        }))
    }

    fn catalog_workspace(&self, session: &str, saved: &str) -> String {
        if same_windows_path(Path::new(saved), &self.inner.state)
            || same_windows_path(Path::new(saved), &self.general_workspace_path())
        {
            return self.general_workspace_path().to_string_lossy().into_owned();
        }
        // The immutable journal header identifies the chat's original root.
        // Its per-chat grant file is the authority for a later working-folder
        // selection, so use it for sidebar grouping when the root is still
        // present and has the same identity.
        let Ok(storage) =
            WorkspaceService::new().and_then(|service| service.register(Path::new(saved)))
        else {
            return saved.into();
        };
        let Ok(folders) = FolderAccess::open(storage, &self.inner.state, session) else {
            return saved.into();
        };
        folders
            .snapshot()
            .map(|snapshot| snapshot.primary().root().to_string_lossy().into_owned())
            .unwrap_or_else(|_| saved.into())
    }

    /// Return an older window of the active journal for the conversation UI.
    /// The UI starts with the newest page and asks for earlier pages only when
    /// someone scrolls up, so a long evidence history does not become one render.
    pub fn session_messages(&self, before: usize) -> Result<Value, String> {
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a local chat first")?;
        let messages = active
            .session
            .records()
            .map_err(describe)?
            .iter()
            .filter_map(project_message)
            .collect::<Vec<_>>();
        let total = messages.len();
        let end = before.min(total);
        let start = end.saturating_sub(MESSAGE_PAGE);
        Ok(json!({
            "messages": messages[start..end].to_vec(),
            "next_before": if start > 0 { Some(start) } else { None::<usize> },
            "start": start,
            "total": total,
        }))
    }

    /// The history panel needs every stored record, not just the current
    /// context branch. It receives compact metadata first and asks for a full
    /// entry only when a user opens that record.
    pub fn session_history(&self, offset: usize) -> Result<Value, String> {
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a local chat first")?;
        let records = active.session.history_records().map_err(describe)?;
        let end = offset.saturating_add(HISTORY_PAGE).min(records.len());
        let entries = records[offset.min(records.len())..end]
            .iter()
            .map(history_entry)
            .collect::<Vec<_>>();
        Ok(json!({
            "entries":entries,
            "total":records.len(),
            "next":if end < records.len() { Some(end) } else { None::<usize> },
        }))
    }

    /// `SessionHistory` restores a forked user's original text into the
    /// composer. This page API keeps a long pasted prompt out of one renderer
    /// bridge response and never accepts a path or other external reference.
    pub fn session_user_text(&self, id: &str, offset: usize) -> Result<Value, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid durable journal entry ID".into());
        }
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a local chat first")?;
        let records = active.session.history_records().map_err(describe)?;
        let record = records
            .iter()
            .find(|record| record["id"].as_str() == Some(id))
            .ok_or("That journal entry is not in the active local chat")?;
        let message = project_message(record).filter(|message| message["role"] == "user");
        let text = message
            .and_then(|message| message["text"].as_str().map(str::to_owned))
            .ok_or("That journal entry is not a user message")?;
        let bytes = text.as_bytes();
        if offset > bytes.len() {
            return Err("Journal entry offset is outside the stored entry".into());
        }
        let end = offset.saturating_add(ENTRY_PAGE_BYTES).min(bytes.len());
        Ok(json!({
            "base64":STANDARD.encode(&bytes[offset..end]),
            "next":if end < bytes.len() { Some(end.to_string()) } else { None::<String> },
        }))
    }

    /// Give the renderer a bounded page from one durable journal record. This
    /// is used only when an abbreviated message or tool output is expanded.
    pub fn session_entry(&self, id: &str, offset: usize) -> Result<Value, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid durable journal entry ID".into());
        }
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a local chat first")?;
        let records = active.session.history_records().map_err(describe)?;
        let record = records
            .iter()
            .find(|record| record["id"].as_str() == Some(id))
            .ok_or("That journal entry is not in the active local chat")?;
        let bytes = serde_json::to_vec(record).map_err(|_| "Encode durable journal entry")?;
        if offset > bytes.len() {
            return Err("Journal entry offset is outside the stored entry".into());
        }
        let end = offset.saturating_add(ENTRY_PAGE_BYTES).min(bytes.len());
        Ok(json!({
            "base64": STANDARD.encode(&bytes[offset..end]),
            "next": if end < bytes.len() { Some(end.to_string()) } else { None::<String> },
        }))
    }

    /// Called by all desktop exit paths. A successful return means the runtime
    /// bundle stopped the exact Job-owned process before AG1W exits.
    pub async fn shutdown(&self) -> Result<(), String> {
        let cancel = self
            .active()?
            .as_ref()
            .and_then(|active| active.cancel.clone());
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        if let Some(runtime) = self.runtime()? {
            runtime.shutdown().await.map_err(describe)?;
        }
        Ok(())
    }

    pub fn view(&self) -> Result<Value, String> {
        let active = self.active()?;
        let Some(active) = active.as_ref() else {
            return Ok(json!({
                "snapshot": Value::Null,
                "error": Value::Null,
                "progress": "",
                "details": empty_details(),
            }));
        };

        let records = active.session.records().map_err(describe)?;
        let all_messages = records
            .iter()
            .filter_map(project_message)
            .collect::<Vec<_>>();
        let total_messages = all_messages.len();
        let messages = all_messages
            .into_iter()
            .skip(total_messages.saturating_sub(MESSAGE_PAGE))
            .collect::<Vec<_>>();
        let revision = active.session.journal_revision().map_err(describe)?;
        let configuration = active.session.configuration().map_err(describe)?;
        let folder_access = active
            .folders
            .as_ref()
            .map(|folders| {
                let snapshot = folders.snapshot().map_err(describe)?;
                serde_json::to_value(snapshot.selection)
                    .map_err(|_| "Encode Chat folder permissions".to_owned())
            })
            .transpose()?;
        let runtime_progress = self
            .runtime()?
            .and_then(|runtime| runtime.progress())
            .and_then(format_runtime_progress);
        let status = if active.busy {
            "streaming"
        } else if active.loading {
            "running"
        } else {
            "idle"
        };
        let scope = json!({
            "workspace": active.workspace_scope,
            "session": active.id.as_str(),
            "instance": INSTANCE,
            "epoch": "1",
        });
        let snapshot_messages = messages
            .iter()
            .map(|message| {
                json!({
                    "id": message["id"],
                    "role": message["role"],
                    "text": message["text"],
                    "evidence": message["evidence"],
                })
            })
            .collect::<Vec<_>>();
        // This is the exact small tool list the model receives on its next
        // prompt. General chat has no project root, so it gets no file tools.
        let write_access = project_write_access(&active.approval_mode);
        let tools = if active.workspace.is_some() {
            LocalProjectTools::definitions_for_command_runner(write_access, true)
        } else {
            vec![]
        };
        let mut capabilities = vec![
            "local-llamacpp-chat",
            "durable-session",
            "context-virtualization",
            "owned-runtime-when-configured",
            "project-file-browser",
        ];
        if active.workspace.is_some() {
            capabilities.push("project-read-tools");
            capabilities.push(if write_access == ProjectWriteAccess::AllowWrites {
                "project-write-tools"
            } else {
                "project-write-tools-gated"
            });
            if write_access == ProjectWriteAccess::AllowWrites {
                capabilities.push("project-command-tools");
            }
        }
        // Live blocks are deliberately transient. They make an in-progress
        // response visible, then the persisted journal replaces them exactly.
        let (observations, live_revision) = match active.live.as_ref() {
            Some(live) => (
                json!({
                    "phase":live.phase,
                    "started_ms":live.started_ms,
                    "first_delta_ms":live.first_delta_ms,
                    "live":{"id":live.id,"blocks":live.blocks,"committed":false},
                }),
                live.revision,
            ),
            None => (
                json!({
                    "phase":if active.loading {"loading"} else {"idle"},
                    "live":Value::Null,
                }),
                0,
            ),
        };
        let latest_usage = active
            .live
            .as_ref()
            .map(|live| usage_with_rate(live.usage.clone(), live.first_delta_ms, now_ms()))
            .or_else(|| active.last_usage.clone())
            .filter(|usage| !usage.as_object().is_none_or(|usage| usage.is_empty()))
            .map(|usage| json!({"usage":usage}))
            .unwrap_or(Value::Null);
        Ok(json!({
            "snapshot": {
                "version": {"major":1,"minor":0,"schema":ag1_protocol::schema_hash()},
                "scope": scope,
                "revision": revision.to_string(),
                "watermark": revision.to_string(),
                "workspace_path": active.workspace_path.to_string_lossy(),
                "status": status,
                "active_operation": if active.busy { json!("local-generation") } else if active.loading { json!("local-model-load") } else { Value::Null },
                "messages": snapshot_messages,
                "message_count": total_messages,
                "pending": [],
                "model": active.model["id"],
                "diagnostic": active.diagnostic,
                "session_file": Value::Null,
                "fixture_provider": false,
            },
            "error": active.diagnostic,
            "progress": runtime_progress.unwrap_or_else(|| active.progress.clone()),
            "details": {
                "authority":"windows-native-local",
                "messages":messages,
                "task":Value::Null,
                "context":{"mode":"real-ag1-context-virtualization","working_context_limit":configuration.context_limit(),"maximum_context_limit":active.model["contextWindow"],"tools":tools,"selected_model":{"provider":active.model["provider"],"id":active.model["id"]},"approval_policy":{"mode":active.approval_mode,"scope":"current-chat"},"folder_access":folder_access},
                "artifacts":[],
                "memory":Value::Null,
                "activity":{
                    "phase":if active.busy {"generating"} else if active.loading {"loading"} else {"idle"},
                    "runtime_preparation":self.runtime()?.and_then(|runtime| runtime.progress()),
                    "observations":observations,
                    "live_revision":live_revision,
                    "latest_assistant_usage":latest_usage,
                },
                "models":[public_model(&active.model)],
                "capabilities":capabilities,
            },
        }))
    }

    pub fn unsupported(&self, name: &str) -> String {
        format!(
            "{name} is not available in the Windows-native host yet. AG1W will not route it through the Linux worker or report a simulated result."
        )
    }

    fn ensure_session(&self) -> Result<(), String> {
        if self.active()?.is_some() {
            return Ok(());
        }
        self.create_session(None)
    }

    fn open_workspace(&self, workspace: Option<Arc<Workspace>>) -> Result<(), String> {
        let mut active = self.active()?;
        if active
            .as_ref()
            .is_some_and(|session| session.busy || session.loading)
        {
            return Err("Wait for the local model to finish before changing projects".into());
        }
        let already_open = match (active.as_ref(), workspace.as_ref()) {
            (Some(current), Some(next)) => current
                .workspace
                .as_ref()
                .is_some_and(|current| current.id() == next.id()),
            (Some(current), None) => current.workspace.is_none(),
            _ => false,
        };
        if already_open {
            return Ok(());
        }
        *active = None;
        drop(active);
        self.create_session(workspace)
    }

    fn create_session(&self, workspace: Option<Arc<Workspace>>) -> Result<(), String> {
        let (model, system_prompt, _) = self.selected_configuration()?;
        let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        // Including the clock prevents a restart from reusing the first
        // in-process counter value and colliding with an older journal.
        let id = SessionId::new(format!("win-session-{}-{serial}", now_ms()))
            .map_err(|error| format!("Create Windows session ID: {error:?}"))?;
        let timestamp = timestamp(now_ms());
        // Folder grants are stored per chat. Take the initial snapshot before
        // creating the session so its durable working directory and the live
        // file tools agree from the first prompt.
        let folders = workspace
            .as_ref()
            .map(|workspace| FolderAccess::open(workspace.clone(), &self.inner.state, id.as_str()))
            .transpose()
            .map_err(describe)?;
        let workspace = folders
            .as_ref()
            .map(|folders| folders.snapshot().map(|snapshot| snapshot.primary()))
            .transpose()
            .map_err(describe)?;
        let has_project_tools = workspace.is_some();
        let (workspace_path, workspace_scope) = self.workspace_view(&workspace);
        let origin = session_origin(&id);
        let session = NativeSession::create(
            self.inner.store.clone(),
            id.clone(),
            &origin,
            &workspace_path.to_string_lossy(),
            &timestamp,
            1,
            session_configuration(model.clone(), &system_prompt, has_project_tools)?,
        )
        .map_err(describe)?;
        *self.active()? = Some(ActiveSession {
            id,
            model,
            session: Arc::new(session),
            workspace,
            folders,
            workspace_path,
            workspace_scope,
            approval_mode: "interactive".into(),
            busy: false,
            loading: false,
            progress: String::new(),
            diagnostic: None,
            cancel: None,
            cancel_token: None,
            live: None,
            last_usage: None,
        });
        Ok(())
    }

    fn restore_workspace(
        &self,
        saved: &str,
    ) -> Result<(Option<Arc<Workspace>>, PathBuf, String), String> {
        let path = PathBuf::from(saved);
        if same_windows_path(&path, &self.inner.state)
            || same_windows_path(&path, &self.general_workspace_path())
        {
            return Ok((None, self.general_workspace_path(), WORKSPACE.into()));
        }
        let service = WorkspaceService::new().map_err(describe)?;
        let workspace = service.register(&path).map_err(|error| match error {
            DomainError::NotFound => {
                "This chat's project folder no longer exists. Open the project again before resuming this chat.".into()
            }
            DomainError::PermissionDenied => {
                "AG1W can no longer access this chat's project folder. Check its Windows permissions, then open the project again.".into()
            }
            other => describe(other),
        })?;
        let (workspace_path, workspace_scope) = self.workspace_view(&Some(workspace.clone()));
        Ok((Some(workspace), workspace_path, workspace_scope))
    }

    fn general_workspace_path(&self) -> PathBuf {
        self.inner.state.join(WORKSPACE)
    }

    fn workspace_view(&self, workspace: &Option<Arc<Workspace>>) -> (PathBuf, String) {
        match workspace {
            Some(workspace) => (
                workspace.root().to_path_buf(),
                workspace.id().as_str().to_owned(),
            ),
            None => (self.general_workspace_path(), WORKSPACE.into()),
        }
    }

    fn project_workspace(&self) -> Result<Arc<Workspace>, String> {
        let active = self.active()?;
        if active
            .as_ref()
            .is_some_and(|session| session.busy || session.loading)
        {
            return Err("Wait for the local model to finish before browsing project files.".into());
        }
        active
            .as_ref()
            .and_then(|session| session.workspace.clone())
            .ok_or_else(|| {
                "Open a project before browsing files. General chat has no project folder.".into()
            })
    }

    fn change_review_scope(&self) -> Result<(String, Vec<Value>, String, Vec<PathBuf>), String> {
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a project chat first")?;
        let roots = Self::active_granted_roots(active)?;
        Ok((
            active.id.as_str().to_owned(),
            active.session.records().map_err(describe)?.as_ref().clone(),
            active.workspace_scope.clone(),
            roots,
        ))
    }

    fn change_undo_target(
        &self,
        args: &Value,
    ) -> Result<(ChangeReceipt, Arc<Workspace>, String, Vec<Value>, String), String> {
        let id = args["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Invalid file-change receipt ID")?;
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a project chat first")?;
        let session = active.id.as_str().to_owned();
        let records = active.session.records().map_err(describe)?.as_ref().clone();
        let workspace_scope = active.workspace_scope.clone();
        let value = self
            .inner
            .store
            .native_file_change(&session, id)
            .map_err(describe)?;
        let receipt: ChangeReceipt = serde_json::from_value(value)
            .map_err(|_| "Invalid native file-change receipt".to_owned())?;
        change_receipts::verify_branch(&receipt, &records, &session, &workspace_scope)
            .map_err(describe)?;
        let workspace = Self::active_workspace_for_root(active, Path::new(&receipt.root))?;
        Ok((receipt, workspace, session, records, workspace_scope))
    }

    fn active_granted_roots(active: &ActiveSession) -> Result<Vec<PathBuf>, String> {
        if let Some(folders) = &active.folders {
            let snapshot = folders.snapshot().map_err(describe)?;
            let roots = snapshot
                .roots()
                .iter()
                .map(|workspace| workspace.root().to_path_buf())
                .collect::<Vec<_>>();
            if !roots.is_empty() {
                return Ok(roots);
            }
        }
        active
            .workspace
            .as_ref()
            .map(|workspace| vec![workspace.root().to_path_buf()])
            .ok_or("Open a project chat before reviewing file changes".into())
    }

    fn active_workspace_for_root(
        active: &ActiveSession,
        root: &Path,
    ) -> Result<Arc<Workspace>, String> {
        if let Some(folders) = &active.folders {
            let snapshot = folders.snapshot().map_err(describe)?;
            if let Some(workspace) = snapshot
                .roots()
                .iter()
                .find(|workspace| same_windows_path(workspace.root(), root))
            {
                return Ok(workspace.clone());
            }
        }
        active
            .workspace
            .as_ref()
            .filter(|workspace| same_windows_path(workspace.root(), root))
            .cloned()
            .ok_or(
                "That file-change receipt belongs to a folder no longer granted to this chat."
                    .into(),
            )
    }

    fn active_has_granted_root(active: &ActiveSession, root: &Path) -> Result<(), String> {
        Self::active_workspace_for_root(active, root).map(|_| ())
    }

    fn change_review_busy(&self) -> Result<bool, String> {
        let active = self.active()?;
        Ok(active
            .as_ref()
            .is_some_and(|active| active.busy || active.loading))
    }

    fn change_diff(&self, receipt: &ChangeReceipt, session: &str) -> Result<ChangeDiff, String> {
        if receipt.before.as_ref().map_or(0, |snapshot| snapshot.bytes) + receipt.after.bytes
            > 256 * 1024
        {
            return Ok(ChangeDiff::Unavailable(
                "This change is too large for an inline diff. Its exact before and after bytes are available in pages.",
            ));
        }
        let before = receipt
            .before
            .as_ref()
            .map(|snapshot| change_receipts::read_snapshot(&self.inner.store, session, snapshot))
            .transpose()
            .map_err(describe)?
            .unwrap_or_default();
        let after = change_receipts::read_snapshot(&self.inner.store, session, &receipt.after)
            .map_err(describe)?;
        match (std::str::from_utf8(&before), std::str::from_utf8(&after)) {
            (Ok(before), Ok(after)) if !before.contains('\0') && !after.contains('\0') => {
                match ag1_core::tools::diff::generate(before, after, 3) {
                    Ok(diff) => Ok(ChangeDiff::Bytes(diff.diff.into_bytes())),
                    Err(DomainError::LimitExceeded(_)) => Ok(ChangeDiff::Unavailable(
                        "This diff exceeds the display budget. Review the before and after source pages.",
                    )),
                    Err(error) => Err(describe(error)),
                }
            }
            _ => Ok(ChangeDiff::Unavailable(
                "Binary change. Exact before and after bytes are available for download.",
            )),
        }
    }

    fn selected_configuration(&self) -> Result<(Value, String, Vec<Value>), String> {
        let config = self.config()?;
        let config = config
            .as_ref()
            .ok_or_else(|| missing_config(&self.inner.state))?;
        let model = config
            .models
            .iter()
            .find(|model| model["id"].as_str() == Some(&config.selected))
            .cloned()
            .ok_or("Selected model is missing from AG1W's native configuration")?;
        Ok((model, config.system_prompt.clone(), config.models.clone()))
    }

    async fn ensure_runtime(&self) -> Result<(), String> {
        if self.runtime()?.is_some() {
            return Ok(());
        }
        let (runtime, models) = {
            let config = self.config()?;
            let config = config
                .as_ref()
                .ok_or_else(|| missing_config(&self.inner.state))?;
            (config.runtime.clone(), config.models.clone())
        };
        let Some(runtime) = runtime else {
            return Ok(());
        };

        // This verifies the sealed model catalog before retaining anything in
        // the desktop. The actual model remains unloaded until this turn's
        // request reaches the provider transport.
        let opened = runtime
            .open(&models, &self.inner.state)
            .await
            .map_err(describe)?;
        let mut current = self
            .inner
            .runtime
            .lock()
            .map_err(|_| "AG1W local runtime lock is unavailable".to_owned())?;
        if current.is_none() {
            *current = Some(opened);
        }
        Ok(())
    }

    fn capture_workspace_diff(&self, root: &Path) -> Result<Vec<u8>, String> {
        // Do not let a repository's aliases, global configuration, pager, or
        // external diff program run as part of simply opening the review pane.
        // These are fixed read-only Git subcommands, not a general terminal.
        let status = self.run_review_git(root, &["status", "--short", "--branch"])?;
        if status.code != 0 {
            return Err(git_failure("status", &status));
        }
        let unstaged = self.run_review_git(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--no-color",
            ],
        )?;
        if unstaged.code != 0 {
            return Err(git_failure("diff", &unstaged));
        }
        let staged = self.run_review_git(
            root,
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--no-color",
            ],
        )?;
        if staged.code != 0 {
            return Err(git_failure("diff --cached", &staged));
        }
        let head = self.run_review_git(root, &["rev-parse", "--verify", "--short", "HEAD"])?;
        let head = if head.code == 0 {
            String::from_utf8_lossy(&head.bytes).trim().to_owned()
        } else {
            "unborn HEAD".into()
        };
        let mut bytes = format!(
            "HEAD {head}\n{}\n{}{}",
            String::from_utf8_lossy(&status.bytes),
            String::from_utf8_lossy(&unstaged.bytes),
            String::from_utf8_lossy(&staged.bytes),
        )
        .into_bytes();
        if bytes.len() > MAX_WORKSPACE_DIFF_BYTES {
            return Err("Workspace review exceeds the 1 MiB display limit.".into());
        }
        // A Git command may end with no newline. Keeping each section separate
        // preserves a valid diff header for the renderer's display-only parser.
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        Ok(bytes)
    }

    fn run_review_git(&self, root: &Path, arguments: &[&str]) -> Result<ReviewGitOutput, String> {
        let executable = review_git_executable()?;
        let spool_directory = self.inner.state.join("review");
        fs::create_dir_all(&spool_directory)
            .map_err(|error| format!("Create AG1W review log folder: {error}"))?;
        let serial = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        let spool = spool_directory.join(format!("git-review-{serial}.log"));
        let result = (|| {
            let mut argv = vec![
                "-c".to_owned(),
                "core.fsmonitor=false".to_owned(),
                "-c".to_owned(),
                "core.untrackedCache=false".to_owned(),
            ];
            argv.extend(arguments.iter().map(|argument| (*argument).to_owned()));
            let supervisor = ProcessSupervisor::new(&self.inner.state).map_err(describe)?;
            let mut child = supervisor
                .spawn(
                    ProcessSpec {
                        executable,
                        argv,
                        cwd: root.to_path_buf(),
                        env: review_git_environment(),
                        timeout_ms: 10_000,
                        grace_ms: 500,
                        resident: false,
                    },
                    &spool,
                )
                .map_err(describe)?;
            let status = child.wait(Duration::from_secs(10)).map_err(describe)?;
            let summary = child.tail(1).map_err(describe)?;
            if summary.total_seen > MAX_WORKSPACE_DIFF_BYTES as u64 || summary.truncated {
                return Err("Git review output exceeds the 1 MiB display limit.".into());
            }
            let expected = usize::try_from(summary.total_seen)
                .map_err(|_| "Git review output is too large".to_owned())?;
            let mut bytes = Vec::with_capacity(expected);
            while bytes.len() < expected {
                let page = child
                    .log_page(bytes.len() as u64, 32 * 1024)
                    .map_err(describe)?;
                if page.is_empty() {
                    return Err("Git review output ended unexpectedly.".into());
                }
                bytes.extend_from_slice(&page);
            }
            Ok(ReviewGitOutput {
                code: status.code().unwrap_or(-1),
                bytes,
            })
        })();
        let _ = fs::remove_file(&spool);
        result
    }

    fn note_event(&self, event: &ag1_core::provider::ProviderEvent) {
        let text = match event {
            // A tool call is part of the same model turn. Say what is happening
            // before the local executor runs it so the activity line never looks stuck.
            ag1_core::provider::ProviderEvent::Begin {
                kind: ag1_core::provider::BlockKind::Tool,
                name,
                ..
            } => format!("Using {name} in the current project"),
            ag1_core::provider::ProviderEvent::Delta { text, .. }
            | ag1_core::provider::ProviderEvent::Replace { text, .. } => {
                format!("Generating local response · {} characters", text.len())
            }
            ag1_core::provider::ProviderEvent::Usage(usage) => {
                format!(
                    "Receiving local response · {} output tokens",
                    usage.output.unwrap_or(0)
                )
            }
            ag1_core::provider::ProviderEvent::Stop(ag1_core::provider::StopReason::ToolUse) => {
                "Running the requested project tool".into()
            }
            ag1_core::provider::ProviderEvent::Fail(_) => "Local model reported an error".into(),
            _ => "Receiving local response".into(),
        };
        if let Ok(mut active) = self.active()
            && let Some(active) = active.as_mut()
        {
            if let Some(live) = active.live.as_mut() {
                live.observe(event);
            }
            active.progress = text;
        }
    }

    fn config(&self) -> Result<std::sync::MutexGuard<'_, Option<HostConfig>>, String> {
        self.inner
            .config
            .lock()
            .map_err(|_| "AG1W model configuration lock is unavailable".into())
    }

    fn attachment_session(&self) -> Result<(String, bool), String> {
        self.ensure_session()?;
        let active = self.active()?;
        let active = active.as_ref().ok_or("Open a local chat first")?;
        Ok((
            active.id.as_str().to_owned(),
            model_accepts_images(&active.model),
        ))
    }

    fn active(&self) -> Result<std::sync::MutexGuard<'_, Option<ActiveSession>>, String> {
        self.inner
            .active
            .lock()
            .map_err(|_| "AG1W session lock is unavailable".into())
    }

    fn runtime(&self) -> Result<Option<Arc<dyn GenerationRuntime>>, String> {
        Ok(self
            .inner
            .runtime
            .lock()
            .map_err(|_| "AG1W local runtime lock is unavailable")?
            .clone())
    }
}

fn config_path(state: &Path) -> PathBuf {
    std::env::var_os("AG1_NATIVE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| state.join("native-host.json"))
}

fn catalog_position(value: &Value) -> Result<Option<CatalogPosition>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let created_at = value["created_at"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or("Invalid artifact catalog position")?;
    let id = value["id"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or("Invalid artifact catalog position")?;
    let origin = value["origin"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or("Invalid artifact catalog position")?;
    Ok(Some(CatalogPosition {
        created_at: created_at.into(),
        id: id.into(),
        origin: origin.into(),
    }))
}

struct ReviewGitOutput {
    code: i32,
    bytes: Vec<u8>,
}

fn review_git_executable() -> Result<PathBuf, String> {
    for folder in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = folder.join("git.exe");
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|error| format!("Resolve Git for workspace review: {error}"));
        }
    }
    Err("AG1W could not find git.exe on PATH for workspace review.".into())
}

fn review_git_environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for name in [
        "APPDATA",
        "LOCALAPPDATA",
        "PATHEXT",
        "PATH",
        "SystemRoot",
        "TEMP",
        "TMP",
        "USERPROFILE",
    ] {
        if let Some(value) = std::env::var_os(name).and_then(|value| value.into_string().ok()) {
            environment.insert(name.into(), value);
        }
    }
    // `NUL` is the native Windows empty configuration file. Local repository
    // data is still read, but user/global aliases and external tools cannot
    // change this fixed review operation.
    environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    environment.insert("GIT_CONFIG_GLOBAL".into(), "NUL".into());
    environment.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
    environment.insert("GIT_PAGER".into(), "cat".into());
    environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    environment
}

fn git_failure(action: &str, output: &ReviewGitOutput) -> String {
    let detail = String::from_utf8_lossy(&output.bytes).trim().to_owned();
    if detail.is_empty() {
        format!("Git {action} failed with exit code {}.", output.code)
    } else {
        format!(
            "Git {action} failed with exit code {}: {detail}",
            output.code
        )
    }
}

fn count_tool_calls(records: &[Value], call: &str) -> usize {
    records
        .iter()
        .flat_map(|record| {
            record["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter(|block| block["type"] == "toolCall" && block["id"] == call)
        .count()
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn read_config(path: &Path) -> Result<Option<HostConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|_| "Cannot read AG1W native model configuration")?;
    if bytes.len() > 1024 * 1024 {
        return Err("AG1W native model configuration exceeds 1 MiB".into());
    }
    let mut config: HostConfig =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid AG1W native model configuration")?;
    if config.models.len() > 64 || config.selected.is_empty() {
        return Err("Invalid AG1W model catalog".into());
    }
    // Older AG1W catalogs predate the Qwen capability flag. Upgrade their
    // in-memory entries on read; a later ordinary catalog save writes it back.
    for model in &mut config.models {
        normalize_local_model_capabilities(model);
    }
    Ok(Some(config))
}

fn write_config(state: &Path, config: &HostConfig) -> Result<(), String> {
    let target = state.join("native-host.json");
    let temporary = state.join("native-host.next.json");
    let bytes = serde_json::to_vec_pretty(config).map_err(|_| "Serialize AG1W model catalog")?;
    if bytes.len() > 1024 * 1024 {
        return Err("AG1W native model configuration exceeds 1 MiB".into());
    }

    // The selected model controls future process ownership. Sync it before a
    // write-through replacement so a crash cannot leave a half-written JSON
    // file which points at an unexpected local endpoint on the next startup.
    let write = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("Write AG1W model catalog: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Sync AG1W model catalog: {error}"))
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let source = wide(&temporary);
    let destination = wide(&target);
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(&temporary);
        return Err(format!("Publish AG1W model catalog: {error}"));
    }
    Ok(())
}

fn prepare_runtime_setup(
    state: &Path,
    existing: Option<&HostConfig>,
    input: NativeModelSetup,
) -> Result<HostConfig, String> {
    if input.port == 0
        || input.model_id.is_empty()
        || input.model_id.len() > 128
        || !input
            .model_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "Use a model ID with up to 128 letters, numbers, dots, dashes, or underscores.".into(),
        );
    }
    if input
        .vram_mib
        .is_some_and(|value| !(512..=1_048_576).contains(&value))
    {
        return Err("VRAM must be between 512 MiB and 1 TiB when supplied.".into());
    }
    if existing.is_some_and(|config| {
        config
            .models
            .iter()
            .any(|model| model["id"].as_str() == Some(input.model_id.as_str()))
    }) {
        return Err("This model is already in the AG1W model picker.".into());
    }
    if existing.is_some_and(|config| !config.models.is_empty() && config.runtime.is_none()) {
        return Err(
            "AG1W cannot add an owned model beside a manually attached server. Remove that server configuration first."
                .into(),
        );
    }

    let server = selected_file(&input.server_path, "llama-server executable")?;
    let model = selected_file(&input.model_path, "GGUF model")?;
    let profile = selected_file(&input.profile_path, "model profile")?;
    let profile_bytes = fs::read(&profile).map_err(|_| "Cannot read the selected model profile")?;
    if profile_bytes.len() > 1024 * 1024 {
        return Err("The selected model profile exceeds 1 MiB.".into());
    }
    let mut source: Value = serde_json::from_slice(&profile_bytes)
        .map_err(|_| "The selected model profile is not valid JSON.")?;
    if source["version"] != json!(1) {
        return Err("The selected model profile must have version 1.".into());
    }
    let models = source["models"]
        .as_object_mut()
        .ok_or("The selected profile has no models object.")?;
    let mut model_profile = models
        .remove(&input.model_id)
        .ok_or("The selected profile has no entry for this model ID.")?;
    if !model_profile.is_object() {
        return Err("The selected model profile entry is invalid.".into());
    }

    // A profile may carry the old Linux executable path. Keep every tuning
    // field but make this sealed capture point at the Windows binary selected
    // by the user.
    model_profile["serverPath"] = json!(server.display().to_string());
    let profile_projector = model_profile["mmprojPath"]
        .as_str()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    // A copied Linux profile can name a path Windows cannot see. Prefer an
    // explicit Windows choice, otherwise use a readable profile entry, then
    // safely inspect only the selected GGUF's directory for one mmproj file.
    let projector = match input.projector_path.clone() {
        Some(path) => Some(selected_file(&path, "vision projector")?),
        None => match profile_projector.filter(|path| path.is_file()) {
            Some(path) => Some(selected_file(&path, "vision projector")?),
            None => discover_projector(&model)?,
        },
    };
    if let Some(projector) = &projector {
        model_profile["mmprojPath"] = json!(projector.display().to_string());
    } else if let Some(fields) = model_profile.as_object_mut() {
        fields.remove("mmprojPath");
    }

    let captured_profile = json!({
        "version": 1,
        "models": { input.model_id.clone(): model_profile.clone() },
    });
    let environment = BTreeMap::from([
        ("AG1_LLAMA_CPP_HOST".to_owned(), "127.0.0.1".to_owned()),
        ("AG1_LLAMA_CPP_PORT".to_owned(), input.port.to_string()),
    ]);
    let profile_entry = serde_json::to_vec(&model_profile)
        .map_err(|_| "Cannot prepare the selected model profile.")?;
    let server_hash = hash_file(&server, "llama-server executable")?;
    let model_hash = hash_file(&model, "GGUF model")?;
    let mut materials = vec![
        Material {
            path: server.display().to_string(),
            sha256: server_hash.clone(),
            kind: "executable".into(),
        },
        Material {
            path: model.display().to_string(),
            sha256: model_hash,
            kind: "weight".into(),
        },
    ];
    if let Some(projector) = &projector {
        materials.push(Material {
            path: projector.display().to_string(),
            sha256: hash_file(projector, "vision projector")?,
            kind: "projector".into(),
        });
    }

    let logical_cores = std::thread::available_parallelism()
        .map(|count| count.get() as u64)
        .unwrap_or(1);
    // Profiles from AG1 carry explicit decode/batch thread values. Using the
    // logical count as the fallback avoids guessing at a physical-core number
    // when Windows has processor groups or a virtual CPU topology.
    let plan = resolve_launch_plan(ResolveInput {
        model_id: input.model_id.clone(),
        model_path: model.display().to_string(),
        profile_bytes: profile_entry,
        environment: environment.clone(),
        context_override: None,
        forced_moe: None,
        cached_moe: None,
        vram_mib: input.vram_mib,
        physical_cores: logical_cores,
        logical_cores,
        discovered_projector: projector
            .as_ref()
            .map(|projector| projector.display().to_string()),
        materials: materials.clone(),
    })
    .map_err(describe)?;

    let serial = SETUP_SERIAL.fetch_add(1, Ordering::Relaxed) + 1;
    let inputs = state.join("runtime-inputs");
    let captures = state.join("runtime-captures");
    fs::create_dir_all(&inputs).map_err(|error| format!("Create AG1W runtime inputs: {error}"))?;
    fs::create_dir_all(&captures)
        .map_err(|error| format!("Create AG1W runtime captures: {error}"))?;
    let label = format!("{}-{}-{serial}", now_ms(), std::process::id());
    let profile_path = inputs.join(format!("{label}-profile.json"));
    let environment_path = inputs.join(format!("{label}-environment.json"));
    let capture_path = captures.join(format!("{label}-capture.json"));
    let stored_profile = serde_json::to_vec(&captured_profile)
        .map_err(|_| "Cannot serialize AG1W runtime profile.")?;
    let stored_environment = serde_json::to_vec(&environment)
        .map_err(|_| "Cannot serialize AG1W runtime environment.")?;
    publish_new(&profile_path, &stored_profile, "runtime profile")?;
    publish_new(
        &environment_path,
        &stored_environment,
        "runtime environment",
    )?;

    let capture = match import_configuration(
        &capture_path,
        &ImportRequest {
            profile: PinnedInput {
                path: profile_path,
                sha256: digest(&stored_profile),
            },
            inherited_environment: PinnedInput {
                path: environment_path,
                sha256: digest(&stored_environment),
            },
            additional_inputs: BTreeMap::new(),
            physical_cores: logical_cores,
            logical_cores,
            vram_mib: input.vram_mib,
            models: vec![ModelImport {
                model_id: plan.model_id.clone(),
                model_path: plan.model_path.clone(),
                context_override: None,
                forced_moe: None,
                cached_moe: None,
                discovered_projector: None,
                materials,
                expected: ExpectedLaunch {
                    command: plan.command.clone(),
                    args: plan.args.clone(),
                    environment: plan.environment.clone(),
                    context_window: plan.requested_context,
                    max_tokens: plan.max_tokens,
                },
            }],
        },
    ) {
        Ok(capture) => capture,
        Err(error) => {
            // The capture is the only file which could be partially published
            // by the importer. Inputs are retained as harmless evidence so a
            // user can inspect what failed instead of AG1W rewriting a profile.
            let _ = fs::remove_file(&capture_path);
            return Err(describe(error));
        }
    };

    let model_entry = json!({
        "id": plan.model_id,
        "provider": "llamacpp",
        "api": "openai-completions",
        "baseUrl": format!("http://127.0.0.1:{}/v1", input.port),
        "contextWindow": plan.requested_context,
        "maxTokens": plan.max_tokens,
        "input": if plan.projector.is_some() { json!(["text", "image"]) } else { json!(["text"]) },
        "reasoning": model_supports_thinking(&input.model_id),
    });
    let (mut saved_models, mut captures, process_host, system_prompt, extra) = match existing {
        Some(config) => {
            let runtime = config
                .runtime
                .as_ref()
                .ok_or("AG1W has no owned runtime to extend.")?;
            if runtime.process_host.sha256 != server_hash {
                return Err(
                    "Use the same llama-server executable for every model in one AG1W picker."
                        .into(),
                );
            }
            (
                config.models.clone(),
                runtime.captures(),
                runtime.process_host.clone(),
                if input.system_prompt.is_empty() {
                    config.system_prompt.clone()
                } else {
                    input.system_prompt.clone()
                },
                config.extra.clone(),
            )
        }
        None => (
            Vec::new(),
            Vec::new(),
            Material {
                path: server.display().to_string(),
                sha256: server_hash,
                kind: "process-host".into(),
            },
            input.system_prompt.clone(),
            BTreeMap::new(),
        ),
    };
    saved_models.push(model_entry);
    captures.push(capture);

    Ok(HostConfig {
        models: saved_models,
        selected: input.model_id,
        system_prompt,
        runtime: Some(WindowsRuntimeConfig {
            captures,
            capture: None,
            // The Windows process supervisor owns llama-server directly. This
            // separately pinned reference ties all model plans to one binary.
            process_host,
        }),
        extra,
    })
}

fn discover_projector(model: &Path) -> Result<Option<PathBuf>, String> {
    let folder = model
        .parent()
        .ok_or("The selected GGUF has no model folder.")?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(folder)
        .map_err(|_| "Cannot inspect the model folder for a vision projector.")?
    {
        let entry = entry.map_err(|_| "Cannot inspect the model folder for a vision projector.")?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
            && name.starts_with("mmproj")
            && name.ends_with(".gguf")
        {
            candidates.push(selected_file(&path, "vision projector")?);
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => Err("Several vision projectors are beside this model. Choose the matching mmproj GGUF explicitly so AG1W does not guess.".into()),
    }
}

fn selected_file(path: &Path, name: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("The {name} path must be absolute."));
    }
    let path = path
        .canonicalize()
        .map_err(|_| format!("Cannot find the selected {name}."))?;
    if !path.is_file() {
        return Err(format!("The selected {name} is not a file."));
    }
    Ok(path)
}

fn hash_file(path: &Path, name: &str) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|_| format!("Cannot hash the selected {name}."))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| format!("Cannot hash the selected {name}."))?;
        if count == 0 {
            return Ok(format!("{:x}", hash.finalize()));
        }
        hash.update(&buffer[..count]);
    }
}

fn publish_new(path: &Path, bytes: &[u8], name: &str) -> Result<(), String> {
    if path.exists() {
        return Err(format!(
            "AG1W already has this {name} receipt. Retry the setup."
        ));
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("Write AG1W {name}: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Sync AG1W {name}: {error}"))
}

fn session_configuration(
    mut model: Value,
    system_prompt: &str,
    has_project_tools: bool,
) -> Result<Configuration, String> {
    normalize_local_model_capabilities(&mut model);
    if model["provider"].as_str() != Some("llamacpp")
        || model["api"].as_str() != Some("openai-completions")
        || model["id"].as_str().is_none_or(str::is_empty)
        || model["baseUrl"].as_str().is_none_or(str::is_empty)
        || model["contextWindow"].as_u64().is_none()
    {
        return Err(
            "AG1W currently accepts only local llama.cpp OpenAI-completions models with id, baseUrl, and contextWindow"
                .into(),
        );
    }
    Ok(Configuration {
        thinking: if model["reasoning"].as_bool() == Some(true) {
            // AG1W exposes Qwen's one native reasoning mode as always-on. This
            // sends llama.cpp the chat-template flag instead of pretending the
            // model supports cloud-style effort choices.
            "high".into()
        } else {
            "off".into()
        },
        model,
        // General chats describe no tools at all. A project chat journals the
        // three read-only definitions. Per-prompt write definitions are added
        // only after the host sees an explicit allow-changes mode.
        tools: if has_project_tools {
            LocalProjectTools::definitions()
        } else {
            vec![]
        },
        system_prompt: system_prompt.into(),
        prompt: None,
        compiler: CompileOptions::default(),
    })
}

fn session_origin(session: &SessionId) -> String {
    // Storage gives evidence an origin owner. A distinct ID per session keeps
    // two otherwise independent Windows chats from claiming the same owner.
    format!("ag1w-native-{}", session.as_str())
}

fn project_write_access(mode: &str) -> ProjectWriteAccess {
    if matches!(mode, "autonomous" | "disabled") {
        ProjectWriteAccess::AllowWrites
    } else {
        ProjectWriteAccess::ReadOnly
    }
}

fn same_local_model(candidate: &Value, saved: &Value) -> bool {
    // A catalog entry may contain display-only fields, but these fields decide
    // the actual provider request. All of them must still match the journal.
    [
        "provider",
        "id",
        "api",
        "baseUrl",
        "contextWindow",
        "maxTokens",
    ]
    .iter()
    .all(|field| candidate[*field] == saved[*field])
}

fn model_supports_thinking(id: &str) -> bool {
    // The local Qwen profiles use the model's Qwen chat template. It emits
    // reasoning natively; AG1W does not need a cloud-provider effort setting.
    id.starts_with("Qwen3.")
}

fn normalize_local_model_capabilities(model: &mut Value) {
    let Some(id) = model["id"].as_str() else {
        return;
    };
    if !model_supports_thinking(id) {
        return;
    }

    // llama.cpp uses this to choose the Qwen chat template's native thinking
    // branch. It affects only the in-memory entry until the normal model
    // catalog writer next saves the user's settings.
    model["reasoning"] = json!(true);
    if !model["compat"].is_object() {
        model["compat"] = json!({});
    }
    if model["compat"]["thinkingFormat"].is_null() {
        model["compat"]["thinkingFormat"] = json!("qwen-chat-template");
    }
}

fn model_reasoning_controls(model: &Value) -> Value {
    let supported = model["reasoning"].as_bool() == Some(true)
        || model["id"].as_str().is_some_and(model_supports_thinking);
    if supported {
        // This is intentionally one fixed mode. The copied llama.cpp profile
        // enables Qwen thinking at the template level, and pretending it has
        // selectable cloud-style effort levels would make the UI lie.
        json!({
            "kind":"unknown",
            "modes":[{"id":"on","label":"On","level":"high"}],
            "source":"qwen-chat-template"
        })
    } else {
        json!({"kind":"unsupported","modes":[],"source":"windows-native-local"})
    }
}

fn public_model(model: &Value) -> Value {
    let controls = model_reasoning_controls(model);
    json!({
        "id":model["id"],
        "provider":model["provider"],
        "contextWindow":model["contextWindow"],
        "input":model["input"],
        "reasoning":controls["kind"] != "unsupported",
        "reasoningControls":controls,
    })
}

fn model_accepts_images(model: &Value) -> bool {
    model["input"]
        .as_array()
        .is_some_and(|input| input.iter().any(|kind| kind == "image"))
}

fn inline_message(text: &str) -> Value {
    json!({"role":"user","content":[{"type":"text","text":text}]})
}

fn validate_user_message(message: &Value, accepts_images: bool) -> Result<(), String> {
    let fields = message
        .as_object()
        .ok_or("Attachment message must be a JSON object.")?;
    if fields.keys().any(|key| key != "role" && key != "content") || message["role"] != "user" {
        return Err("Attachment message must contain only a user role and content.".into());
    }
    let blocks = message["content"]
        .as_array()
        .filter(|blocks| !blocks.is_empty() && blocks.len() <= 16)
        .ok_or("Attachment message needs between one and sixteen content blocks.")?;
    let mut has_content = false;
    let mut image_count = 0usize;
    let mut image_bytes = 0usize;
    let mut text_bytes = 0usize;
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => {
                let text = block["text"]
                    .as_str()
                    .ok_or("A text attachment block is invalid.")?;
                text_bytes = text_bytes.saturating_add(text.len());
                has_content |= !text.trim().is_empty();
            }
            Some("image") => {
                if !accepts_images {
                    return Err("The selected model is text-only. Choose a vision-enabled model with its matching mmproj projector before sending an image.".into());
                }
                let mime = block["mimeType"]
                    .as_str()
                    .ok_or("Image MIME type is missing.")?;
                if !matches!(
                    mime,
                    "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                ) {
                    return Err("Use PNG, JPEG, WebP, or GIF images.".into());
                }
                let bytes = STANDARD
                    .decode(block["data"].as_str().ok_or("Image data is missing.")?)
                    .map_err(|_| "Image data is not valid base64.".to_owned())?;
                image_count += 1;
                image_bytes = image_bytes.saturating_add(bytes.len());
                let format = image::guess_format(&bytes)
                    .map_err(|_| "Image format is invalid.".to_owned())?;
                if Some(format.to_mime_type()) != Some(mime) {
                    return Err("Image MIME type does not match its file data.".into());
                }
                let dimensions = ImageReader::new(std::io::Cursor::new(&bytes))
                    .with_guessed_format()
                    .map_err(|_| "Image format is invalid.".to_owned())?
                    .into_dimensions()
                    .map_err(|_| "Image dimensions are invalid.".to_owned())?;
                if dimensions.0 > 16_384
                    || dimensions.1 > 16_384
                    || u64::from(dimensions.0) * u64::from(dimensions.1) > 32 * 1024 * 1024
                {
                    return Err("Image dimensions are too large.".into());
                }
                has_content = true;
            }
            _ => return Err("Attachment content must be text or an image.".into()),
        }
    }
    if !has_content
        || text_bytes > LIVE_TEXT_BYTES
        || image_count > MAX_ATTACHMENT_IMAGES
        || image_bytes > MAX_ATTACHMENT_IMAGE_BYTES
    {
        return Err("Attachments may contain up to eight images totaling 2 MiB, plus at most 4 MiB of text.".into());
    }
    Ok(())
}

fn project_message(record: &Value) -> Option<Value> {
    let message = record.get("message")?;
    let role = message["role"].as_str()?;
    if !matches!(role, "user" | "assistant" | "toolResult") {
        return None;
    }
    let content = &message["content"];
    let text = content.as_str().map(str::to_owned).unwrap_or_else(|| {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });
    // The desktop's compact activity component needs structured tool blocks,
    // not a flattened transcript. Keep the journal record as the source here.
    let blocks = if let Some(text) = content.as_str() {
        vec![json!({"kind":"text","text":text})]
    } else {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| match block["type"].as_str() {
                Some("text") => Some(json!({"kind":"text","text":block["text"]})),
                Some("thinking") => Some(json!({"kind":"thinking","text":block["thinking"]})),
                Some("toolCall") => Some(json!({
                    "kind":"tool",
                    "name":block["name"],
                    "call_id":block["id"],
                    "arguments":block["arguments"],
                })),
                Some("image") => Some(json!({
                    "kind":"attachment",
                    "media_type":block["mimeType"],
                    "reference":record["id"],
                })),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let tool = if role == "toolResult" {
        json!({
            "call_id":message["toolCallId"],
            "name":message["toolName"],
            "failed":message["isError"].as_bool().unwrap_or(true),
        })
    } else {
        Value::Null
    };
    Some(json!({
        "id":record["id"],
        "role":role,
        "text":text,
        "evidence":record["id"],
        "blocks":blocks,
        "tool":tool,
    }))
}

fn history_entry(record: &Value) -> Value {
    let projected = project_message(record);
    let preview = projected
        .as_ref()
        .and_then(|message| message["text"].as_str())
        .map(|text| utf8_prefix(text, 240).to_owned())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| record["type"].as_str().unwrap_or("journal entry").into());
    json!({
        "id":record["id"],
        "parent":record["parentId"],
        "kind":record["type"],
        "role":projected.and_then(|message| message["role"].as_str().map(str::to_owned)),
        "preview":preview,
    })
}

fn session_export_html(records: &[Value]) -> Result<String, DomainError> {
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }

    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>AG1 session</title><style>body{max-width:80ch;margin:3rem auto;font:16px system-ui}pre{white-space:pre-wrap;overflow-wrap:anywhere}article{border-bottom:1px solid #bbb;padding:1rem}</style></head><body><h1>AG1 session</h1><p>Complete active-branch text. Attachment bytes remain in the native journal.</p>",
    );
    for record in records.iter().filter(|record| record["type"] == "message") {
        let mut content = record["message"]["content"].clone();
        if let Some(blocks) = content.as_array_mut() {
            for block in blocks {
                if block["type"] == "image" {
                    *block = json!({
                        "type":"attachment",
                        "mimeType":block["mimeType"],
                        "journalEntry":record["id"],
                        "binary":"Retained in journal",
                    });
                }
            }
        }
        let rendered = serde_json::to_string_pretty(&content)
            .map_err(|_| DomainError::InvalidInput("HTML export content".into()))?;
        html.push_str(&format!(
            "<article><h2>{}</h2><small>{}</small><pre>{}</pre></article>",
            escape(record["message"]["role"].as_str().unwrap_or("unknown")),
            escape(record["id"].as_str().unwrap_or("")),
            escape(&rendered),
        ));
        if html.len() > MAX_READ.saturating_sub(4096) {
            return Err(DomainError::LimitExceeded(
                "HTML export exceeds 8 MiB; use the lossless storage archive API".into(),
            ));
        }
    }
    html.push_str("<footer><pre>");
    html.push_str(&escape(include_str!("../../../../LICENSE")));
    html.push_str(
        "</pre><p>No third-party JavaScript is embedded in this export.</p></footer></body></html>",
    );
    if html.len() > MAX_READ {
        return Err(DomainError::LimitExceeded(
            "HTML export exceeds 8 MiB; use the lossless storage archive API".into(),
        ));
    }
    Ok(html)
}

fn empty_details() -> Value {
    json!({
        "authority":"windows-native-local",
        "messages":[],
        "task":Value::Null,
        "context":{},
        "artifacts":[],
        "memory":Value::Null,
        "activity":{},
        "models":[],
        "capabilities":["local-llamacpp-chat","durable-session","context-virtualization","owned-runtime-when-configured"],
    })
}

fn format_runtime_progress(value: Value) -> Option<String> {
    let phase = value["phase"].as_str()?;
    let model = value["model"].as_str().unwrap_or("local model");
    let percent = value["percent"].as_u64();
    Some(match percent {
        Some(percent) => format!("Checking {model} files · {percent}%"),
        None => format!("Local model {phase} · {model}"),
    })
}

fn utf8_prefix(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn usage_with_rate(mut usage: Value, first_delta_ms: Option<u64>, now: u64) -> Value {
    let Some(started) = first_delta_ms else {
        return usage;
    };
    let Some(output) = usage["output"].as_u64() else {
        return usage;
    };
    let elapsed_ms = now.saturating_sub(started).max(1);
    usage["tokensPerSecond"] = json!(output as f64 * 1_000.0 / elapsed_ms as f64);
    usage
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn timestamp(milliseconds: u64) -> String {
    // Session records are validated against AG1's canonical timestamp shape.
    // UTC avoids a Windows locale or daylight-saving setting changing history.
    let seconds = (milliseconds / 1_000).min(i64::MAX as u64) as i64;
    let nanos = ((milliseconds % 1_000) * 1_000_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, nanos)
        .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn editor_revision(revision: &FileRevision) -> Result<String, String> {
    let value = json!({
        "sha": revision.sha256,
        "dev": revision.device,
        "ino": revision.inode,
        "size": revision.size,
        "mtime": revision.modified_ns.to_string(),
        "ctime": revision.changed_ns.to_string(),
        "mode": revision.mode,
    });
    let bytes = serde_json::to_vec(&value).map_err(|_| "Cannot encode file revision".to_owned())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn editor_error(error: DomainError) -> String {
    match error {
        DomainError::Conflict => {
            "File conflict: disk changed; your unsaved text is retained.".into()
        }
        DomainError::NotFound => "File no longer exists inside the selected project.".into(),
        DomainError::PermissionDenied => {
            "That path is outside the selected project or crosses a Windows reparse point.".into()
        }
        DomainError::LimitExceeded(_) => "This file exceeds the 32 KiB native editor limit.".into(),
        other => describe(other),
    }
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    fn normalize(path: &Path) -> String {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_start_matches(r"\\?\")
            .trim_end_matches('\\')
            .to_owned()
    }
    normalize(left).eq_ignore_ascii_case(&normalize(right))
}

fn missing_config(state: &Path) -> String {
    format!(
        "No native model catalog. Create {} or set AG1_NATIVE_CONFIG; opening AG1W never starts a model automatically.",
        state.join("native-host.json").display()
    )
}

fn describe(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag1_core::provider::{BlockKind, ProviderEvent, StopReason, UsagePatch};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::Command,
        sync::mpsc,
        thread,
    };
    use tempfile::tempdir;

    #[test]
    fn qwen_local_models_report_native_thinking() {
        // Existing state files from before this fix stored reasoning as false.
        // The model family is the source of truth for those old entries.
        let mut qwen = json!({
            "id": "Qwen3.6-35B-A3B-UD-Q4_K_XL",
            "reasoning": false
        });
        let other = json!({"id": "plain-local-model", "reasoning": false});

        normalize_local_model_capabilities(&mut qwen);
        assert_eq!(qwen["reasoning"], true);
        assert_eq!(qwen["compat"]["thinkingFormat"], "qwen-chat-template");
        assert_eq!(model_reasoning_controls(&qwen)["kind"], "unknown");
        assert_eq!(model_reasoning_controls(&qwen)["modes"][0]["id"], "on");
        assert_eq!(model_reasoning_controls(&other)["kind"], "unsupported");
    }

    #[test]
    fn qwen_session_configuration_enables_native_thinking() {
        let configuration = session_configuration(
            json!({
                "id":"Qwen3.6-35B-A3B-UD-Q4_K_XL",
                "provider":"llamacpp",
                "api":"openai-completions",
                "baseUrl":"http://127.0.0.1:18084/v1",
                "contextWindow":90000,
                "maxTokens":65536,
                "input":["text"],
                "reasoning":false
            }),
            "Fixture",
            false,
        )
        .unwrap();

        assert_eq!(configuration.thinking, "high");
        assert_eq!(configuration.model["reasoning"], true);
        assert_eq!(
            configuration.model["compat"]["thinkingFormat"],
            "qwen-chat-template"
        );
    }

    fn write_fixture_config(state: &Path, id: &str) -> PathBuf {
        let path = state.join("native-host.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":id,
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":"http://127.0.0.1:19092/v1",
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":id,
                "system_prompt":"Project fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        path
    }

    fn read_http_request(socket: &mut std::net::TcpStream) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let count = socket.read(&mut chunk)?;
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP request ended before its body",
                ));
            }
            bytes.extend_from_slice(&chunk[..count]);
            let Some(header_end) = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|offset| offset + 4)
            else {
                if bytes.len() > 64 * 1024 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "HTTP request headers are too large",
                    ));
                }
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "HTTP request has no content length",
                    )
                })?;
            let total = header_end.saturating_add(content_length);
            if bytes.len() >= total {
                bytes.truncate(total);
                return Ok(bytes);
            }
        }
    }

    #[test]
    fn live_activity_keeps_each_tool_round_visible_until_the_journal_commits() {
        let mut live = LiveActivity::new("live-test".into(), 42);
        live.observe(&ProviderEvent::Begin {
            key: "reason".into(),
            kind: BlockKind::Thinking,
            id: String::new(),
            name: String::new(),
            arguments: None,
            redacted: false,
        });
        live.observe(&ProviderEvent::Delta {
            key: "reason".into(),
            text: "Read the project first.".into(),
        });
        live.observe(&ProviderEvent::Begin {
            key: "call".into(),
            kind: BlockKind::Tool,
            id: "pending-call".into(),
            name: "read_project_file".into(),
            arguments: Some(json!({"path":"src/main.rs"})),
            redacted: false,
        });
        live.observe(&ProviderEvent::Stop(StopReason::ToolUse));
        live.observe(&ProviderEvent::Begin {
            // Providers can reuse a stream key after a tool result. The round
            // prefix keeps the second answer distinct from the first call.
            key: "reason".into(),
            kind: BlockKind::Text,
            id: String::new(),
            name: String::new(),
            arguments: None,
            redacted: false,
        });
        live.observe(&ProviderEvent::Delta {
            key: "reason".into(),
            text: "The entry point is in main.rs.".into(),
        });
        live.observe(&ProviderEvent::Usage(UsagePatch {
            input: Some(21),
            output: Some(13),
            ..Default::default()
        }));

        assert_eq!(live.phase, "generating");
        assert_eq!(live.blocks[0]["kind"], "thinking");
        assert_eq!(live.blocks[0]["key"], "0:reason");
        assert_eq!(live.blocks[0]["text"], "Read the project first.");
        assert_eq!(live.blocks[1]["kind"], "tool");
        assert_eq!(live.blocks[1]["name"], "read_project_file");
        assert_eq!(live.blocks[2]["key"], "1:reason");
        assert_eq!(live.blocks[2]["text"], "The entry point is in main.rs.");
        assert_eq!(live.usage["input"], 21);
        assert_eq!(live.usage["output"], 13);
        assert!(live.first_delta_ms.is_some());
        assert!(live.revision > 1);
    }

    #[test]
    fn live_text_limit_does_not_split_a_utf8_character() {
        assert_eq!(utf8_prefix("å", 1), "");
        assert_eq!(utf8_prefix("å", 2), "å");
    }

    #[test]
    fn token_rate_uses_the_first_received_token_as_its_start() {
        let usage = usage_with_rate(json!({"output": 10}), Some(1_000), 1_500);
        assert_eq!(usage["tokensPerSecond"], 20.0);
    }

    #[test]
    fn prompt_admission_publishes_a_live_view_before_the_model_runs() {
        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"live-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":"http://127.0.0.1:19091/v1",
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":"live-fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();
        host.begin_prompt("Show the native live state.").unwrap();

        let view = host.view().unwrap();
        assert_eq!(view["snapshot"]["status"], "streaming");
        assert_eq!(
            view["details"]["activity"]["observations"]["phase"],
            "preparing"
        );
        assert_eq!(
            view["details"]["activity"]["observations"]["live"]["committed"],
            false
        );
        assert!(
            view["details"]["activity"]["live_revision"]
                .as_u64()
                .unwrap()
                > 0
        );
    }

    #[test]
    fn conversation_status_tracks_the_active_native_chat() {
        let state = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "status-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();

        assert_eq!(host.conversation_statuses().unwrap(), json!([]));

        let opened = host.open_general().unwrap();
        let session = opened["snapshot"]["scope"]["session"].clone();
        let workspace = opened["snapshot"]["workspace_path"].clone();
        let idle = host.conversation_statuses().unwrap();
        assert_eq!(idle[0]["session"], session);
        assert_eq!(idle[0]["workspace"], workspace);
        assert_eq!(idle[0]["status"], "idle");
        assert_eq!(idle[0]["pending"], 0);
        assert_eq!(idle[0]["error"], Value::Null);

        host.begin_prompt("Show the sidebar that work has started.")
            .unwrap();
        assert_eq!(
            host.conversation_statuses().unwrap()[0]["status"],
            "streaming"
        );

        host.finish_prompt(Some("fixture failure".into()));
        let failed = host.conversation_statuses().unwrap();
        assert_eq!(failed[0]["status"], "failed");
        assert_eq!(failed[0]["error"], "fixture failure");
    }

    #[test]
    fn artifact_review_reads_only_the_selected_chat_and_uses_bounded_pages() {
        let state = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "artifact-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();

        let session = {
            let active = host.active().unwrap();
            active.as_ref().unwrap().id.clone()
        };
        let id = "art_00000000000000000000000000000001";
        let bytes = b"artifact preview bytes".to_vec();
        host.inner
            .store
            .put_artifact(&ag1_storage::ArtifactInput {
                origin: session_origin(&session),
                session: session.as_str().into(),
                record: json!({
                    "version":1,
                    "id":id,
                    "sessionId":session.as_str(),
                    "kind":"fixture-output",
                    "mimeType":"text/plain",
                    "encoding":"utf8",
                    "createdAt":"2026-11-13T00:00:00.000Z",
                    "sha256":digest(&bytes),
                    "bytes":bytes.len(),
                    "lineCount":1,
                    "salientLines":[],
                    "producer":{"type":"tool","name":"fixture","callId":"fixture-call"},
                })
                .to_string()
                .into_bytes(),
                bytes: bytes.clone(),
            })
            .unwrap();

        let catalog = host.artifact_catalog(&json!({"before":null})).unwrap();
        assert_eq!(catalog["artifacts"][0]["id"], id);
        assert_eq!(catalog["artifacts"][0]["kind"], "fixture-output");
        let page = host.read_artifact(id, 0, 8).unwrap();
        assert_eq!(
            STANDARD.decode(page["base64"].as_str().unwrap()).unwrap(),
            bytes[..8]
        );
        assert_eq!(page["next"], "8");
        assert!(host.artifact_catalog(&json!({"before":{"id":id}})).is_err());

        // Creating another chat must not give it access to the older chat's
        // retained bytes, even though both chats use the same local store.
        host.new_chat().unwrap();
        assert!(host.read_artifact(id, 0, 32).is_err());
    }

    #[test]
    fn workspace_review_pages_an_immutable_snapshot_for_one_project_chat() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "review-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_project(project.path().to_path_buf()).unwrap();

        let (session, root) = {
            let active = host.active().unwrap();
            let active = active.as_ref().unwrap();
            (
                active.id.as_str().to_owned(),
                active.workspace.as_ref().unwrap().root().to_path_buf(),
            )
        };
        let bytes = vec![b'x'; 24 * 1024 + 5];
        let sha256 = digest(&bytes);
        *host.inner.diff_review.lock().unwrap() = Some(DiffReview {
            session,
            root,
            sha256: sha256.clone(),
            bytes,
        });

        let first = host
            .workspace_diff(&json!({"offset":"0","sha256":sha256}))
            .unwrap();
        assert_eq!(
            STANDARD
                .decode(first["base64"].as_str().unwrap())
                .unwrap()
                .len(),
            24 * 1024
        );
        assert_eq!(first["next"], (24 * 1024).to_string());
        assert!(host.workspace_diff(&json!({"offset":"1"})).is_err());

        // A snapshot belongs to the chat that created it. A fresh project chat
        // must refresh rather than receiving the previous chat's cached bytes.
        host.new_chat().unwrap();
        assert!(
            host.workspace_diff(&json!({"offset":"0","sha256":first["sha256"]}))
                .is_err()
        );
    }

    #[test]
    fn workspace_review_runs_only_fixed_read_only_git_queries() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let git = review_git_executable().unwrap();
        assert!(
            Command::new(&git)
                .args(["init", "--quiet"])
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "NUL")
                .current_dir(project.path())
                .status()
                .unwrap()
                .success()
        );
        fs::write(project.path().join("tracked.txt"), "line one\nline two\n").unwrap();
        assert!(
            Command::new(&git)
                .args(["add", "--", "tracked.txt"])
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "NUL")
                .current_dir(project.path())
                .status()
                .unwrap()
                .success()
        );

        let config_path = write_fixture_config(state.path(), "git-review-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_project(project.path().to_path_buf()).unwrap();
        let result = host.workspace_diff(&json!({})).unwrap();
        let text = String::from_utf8(STANDARD.decode(result["base64"].as_str().unwrap()).unwrap())
            .unwrap();
        assert!(text.contains("A  tracked.txt"));
        assert!(text.contains("diff --git a/tracked.txt b/tracked.txt"));
        assert!(text.contains("+line one"));
    }

    #[test]
    fn project_change_review_shows_only_verified_receipts_from_the_selected_turn() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        fs::write(project.path().join("notes.txt"), "first line\n").unwrap();
        let config_path = write_fixture_config(state.path(), "change-review-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_project(project.path().to_path_buf()).unwrap();

        let (session_id, session, workspace, workspace_scope) = {
            let active = host.active().unwrap();
            let active = active.as_ref().unwrap();
            (
                active.id.clone(),
                active.session.clone(),
                active.workspace.as_ref().unwrap().clone(),
                active.workspace_scope.clone(),
            )
        };
        session
            .append_message(
                "review-user",
                "review-user",
                "2026-11-13T00:00:01.000Z",
                json!({"role":"user","content":"Update notes.txt."}),
            )
            .unwrap();
        let call = json!({
            "type":"toolCall",
            "id":"review-edit-notes",
            "name":"edit_project_file",
            "arguments":{"path":"notes.txt","oldText":"first","newText":"second"},
        });
        session
            .append_message(
                "review-assistant",
                "review-assistant",
                "2026-11-13T00:00:02.000Z",
                json!({"role":"assistant","content":[call]}),
            )
            .unwrap();
        let before = workspace.snapshot("notes.txt", MAX_READ).unwrap();
        let after = b"second line\n";
        let pending = ag1_platform::change_receipts::PendingChange::prepare_local(
            host.inner.store.as_ref(),
            workspace.as_ref(),
            session_id.as_str(),
            &workspace_scope,
            "review-assistant",
            "review-edit-notes",
            "edit_project_file",
            "notes.txt",
            Some(&before.revision),
            after,
            1_763_000_002_000,
        )
        .unwrap()
        .unwrap();
        let published = workspace.replace("notes.txt", &before.revision, after);
        pending
            .settle(host.inner.store.as_ref(), &published)
            .unwrap();
        published.unwrap();

        let listed = host
            .turn_changes(&json!({"turn":"review-user","offset":0}))
            .unwrap();
        assert_eq!(listed["changes"].as_array().unwrap().len(), 1);
        let id = listed["changes"][0]["id"].as_str().unwrap();
        assert_eq!(listed["changes"][0]["path"], "notes.txt");
        assert_eq!(listed["changes"][0]["state"], "committed");
        let diff = host
            .read_turn_change(&json!({"id":id,"side":"diff","offset":"0"}))
            .unwrap();
        let diff =
            String::from_utf8(STANDARD.decode(diff["base64"].as_str().unwrap()).unwrap()).unwrap();
        assert!(diff.contains("first line"), "{diff}");
        assert!(diff.contains("second line"), "{diff}");

        let undo = host.change_undo_status(&json!({"id":id})).unwrap();
        assert_eq!(undo["allowed"], true);
        let outcome = host.change_undo(&json!({"id":id}), false).unwrap();
        assert_eq!(outcome["state"], "completed");
        assert_eq!(
            fs::read_to_string(project.path().join("notes.txt")).unwrap(),
            "first line\n"
        );
        assert_eq!(
            host.change_undo_status(&json!({"id":id})).unwrap()["allowed"],
            false
        );

        // A new project chat may use the same files, but it must not inspect
        // receipts that belong to the old journal branch.
        host.new_chat().unwrap();
        assert!(
            host.turn_changes(&json!({"turn":"review-user","offset":0}))
                .is_err()
        );
    }

    #[test]
    fn stop_is_available_while_a_local_request_is_still_preparing() {
        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"stop-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":"http://127.0.0.1:19095/v1",
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":"stop-fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();
        host.begin_prompt("Stop before startup finishes.").unwrap();

        host.cancel().unwrap();
        assert_eq!(
            host.view().unwrap()["progress"],
            "Stopping local generation"
        );
    }

    #[test]
    fn changing_models_keeps_the_same_project_but_starts_a_fresh_chat() {
        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[
                    {
                        "id":"first",
                        "provider":"llamacpp",
                        "api":"openai-completions",
                        "baseUrl":"http://127.0.0.1:19091/v1",
                        "contextWindow":4096,
                        "maxTokens":256,
                        "input":["text"]
                    },
                    {
                        "id":"second",
                        "provider":"llamacpp",
                        "api":"openai-completions",
                        "baseUrl":"http://127.0.0.1:19092/v1",
                        "contextWindow":4096,
                        "maxTokens":256,
                        "input":["text"]
                    }
                ],
                "selected":"first"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        let first = host.open_general().unwrap();
        let first_session = first["snapshot"]["scope"]["session"]
            .as_str()
            .unwrap()
            .to_owned();

        host.select_model("llamacpp", "second").unwrap();
        let second = host.view().unwrap();
        assert_eq!(second["snapshot"]["model"], "second");
        assert_ne!(
            second["snapshot"]["scope"]["session"],
            Value::String(first_session)
        );
        assert_eq!(
            second["snapshot"]["workspace_path"],
            first["snapshot"]["workspace_path"]
        );
        assert_eq!(second["snapshot"]["message_count"], 0);
    }

    #[test]
    fn a_second_native_chat_gets_its_own_artifact_origin() {
        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"chat-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":"http://127.0.0.1:19093/v1",
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":"chat-fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        let first = host.open_general().unwrap();
        host.new_chat().unwrap();
        let second = host.view().unwrap();

        assert_ne!(
            first["snapshot"]["scope"]["session"],
            second["snapshot"]["scope"]["session"]
        );
        assert_eq!(second["snapshot"]["message_count"], 0);
        let session = host.active().unwrap().as_ref().unwrap().session.clone();
        session
            .compile_context("new-chat-context", "2026-11-13T00:00:00.000Z")
            .unwrap();
    }

    #[test]
    fn legacy_single_origin_session_still_resumes_its_context_store() {
        use ag1_storage::{SessionBootstrap, SessionSeed};

        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        let model = json!({
            "id":"legacy-fixture",
            "provider":"llamacpp",
            "api":"openai-completions",
            "baseUrl":"http://127.0.0.1:19094/v1",
            "contextWindow":4096,
            "maxTokens":256,
            "input":["text"]
        });
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({"models":[model],"selected":"legacy-fixture"})).unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        let id = SessionId::new("legacy-windows-chat").unwrap();
        let header = serde_json::to_vec(&json!({
            "type":"session",
            "version":3,
            "id":id.as_str(),
            "cwd":state.path().join(WORKSPACE).to_string_lossy(),
            "timestamp":"2026-11-13T00:00:00.000Z",
            // This deliberately has no ag1wOrigin: it models the first native
            // Windows sessions published before origins became per-chat.
            "ag1wRuntime":{"model":model,"systemPrompt":"Legacy prompt"}
        }))
        .unwrap();
        host.inner
            .store
            .bootstrap_session(SessionBootstrap {
                command: "bootstrap-legacy-windows-chat",
                session: id.as_str(),
                origin: LEGACY_ORIGIN,
                cursor_key: &[7; 32],
                seed: SessionSeed::Fresh {
                    header: &header,
                    records: &[],
                },
            })
            .unwrap();

        host.open_session(id.as_str()).unwrap();
        let session = host.active().unwrap().as_ref().unwrap().session.clone();
        session
            .append_message(
                "legacy-user",
                "legacy-user",
                "2026-11-13T00:00:01.000Z",
                json!({"role":"user","content":"Keep this local history."}),
            )
            .unwrap();
        session
            .compile_context("legacy-context", "2026-11-13T00:00:02.000Z")
            .unwrap();
    }

    #[test]
    fn project_message_keeps_tool_calls_and_results_for_the_compact_activity_view() {
        let call = project_message(&json!({
            "id":"assistant-tool-call",
            "message":{
                "role":"assistant",
                "content":[
                    {"type":"thinking","thinking":"I need the project file list."},
                    {
                        "type":"toolCall",
                        "id":"list-project-files",
                        "name":"list_project_files",
                        "arguments":{}
                    }
                ]
            }
        }))
        .unwrap();
        assert_eq!(call["blocks"][0]["kind"], "thinking");
        assert_eq!(call["blocks"][1]["kind"], "tool");
        assert_eq!(call["blocks"][1]["name"], "list_project_files");
        assert_eq!(call["blocks"][1]["call_id"], "list-project-files");

        let result = project_message(&json!({
            "id":"project-file-result",
            "message":{
                "role":"toolResult",
                "toolCallId":"list-project-files",
                "toolName":"list_project_files",
                "isError":false,
                "content":[{"type":"text","text":"src/main.rs"}]
            }
        }))
        .unwrap();
        assert_eq!(result["role"], "toolResult");
        assert_eq!(result["tool"]["call_id"], "list-project-files");
        assert_eq!(result["tool"]["name"], "list_project_files");
        assert_eq!(result["tool"]["failed"], false);
        assert_eq!(result["blocks"][0]["text"], "src/main.rs");
    }

    #[test]
    fn native_history_pages_durable_messages_and_entries() {
        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"history-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":"http://127.0.0.1:19092/v1",
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":"history-fixture",
                "system_prompt":"History fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();
        let session = host.active().unwrap().as_ref().unwrap().session.clone();
        for index in 0..(MESSAGE_PAGE + 2) {
            let id = format!("history-user-{index}");
            session
                .append_message(
                    &id,
                    &id,
                    "2026-11-12T00:00:01.000Z",
                    json!({"role":"user","content":format!("Message {index}")}),
                )
                .unwrap();
        }

        let view = host.view().unwrap();
        assert_eq!(view["snapshot"]["message_count"], MESSAGE_PAGE + 2);
        assert_eq!(
            view["details"]["messages"].as_array().unwrap().len(),
            MESSAGE_PAGE
        );

        let page = host.session_messages(MESSAGE_PAGE + 2).unwrap();
        assert_eq!(page["start"], 2);
        assert_eq!(page["total"], MESSAGE_PAGE + 2);
        assert_eq!(page["next_before"], 2);

        let entry = host.session_entry("history-user-0", 0).unwrap();
        let bytes = STANDARD.decode(entry["base64"].as_str().unwrap()).unwrap();
        let stored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["message"]["content"], "Message 0");
        assert_eq!(entry["next"], Value::Null);
    }

    #[test]
    fn session_export_writes_a_new_safe_html_file_in_the_project() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "export-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_project(project.path().to_path_buf()).unwrap();
        let session = host.active().unwrap().as_ref().unwrap().session.clone();
        session
            .append_message(
                "export-user",
                "export-user",
                &timestamp(1),
                json!({"role":"user","content":[
                    {"type":"text","text":"Keep this visible in the export."},
                    {"type":"image","mimeType":"image/png","data":"secret-image-bytes"}
                ]}),
            )
            .unwrap();

        let exported = host.export_session_html("conversation.html").unwrap();
        assert_eq!(exported["path"], "conversation.html");
        assert_eq!(exported["messages"], 1);
        let html = fs::read_to_string(project.path().join("conversation.html")).unwrap();
        assert!(html.contains("Keep this visible in the export."));
        assert!(html.contains("Retained in journal"));
        assert!(!html.contains("secret-image-bytes"));
        assert!(host.export_session_html("conversation.html").is_err());
        assert!(host.export_session_html("..\\outside.html").is_err());
    }

    #[test]
    fn native_context_limit_is_durable_and_scoped_to_one_chat() {
        let state = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "context-fixture");
        let host =
            NativeHost::open_from_config(state.path().to_path_buf(), config_path.clone()).unwrap();
        host.open_general().unwrap();
        let session = host.view().unwrap()["snapshot"]["scope"]["session"]
            .as_str()
            .unwrap()
            .to_owned();

        host.set_context_limit(2_048).unwrap();
        assert_eq!(
            host.view().unwrap()["details"]["context"]["working_context_limit"],
            2_048
        );
        assert!(
            host.set_context_limit(1_023)
                .unwrap_err()
                .contains("at least")
        );
        assert!(
            host.set_context_limit(4_097)
                .unwrap_err()
                .contains("between")
        );
        drop(host);

        let resumed =
            NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        resumed.open_session(&session).unwrap();
        assert_eq!(
            resumed.view().unwrap()["details"]["context"]["working_context_limit"],
            2_048
        );
        resumed.new_chat().unwrap();
        assert_eq!(
            resumed.view().unwrap()["details"]["context"]["working_context_limit"],
            4_096
        );
    }

    #[test]
    fn native_fork_creates_a_resumable_child_before_the_selected_user_message() {
        let state = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "fork-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();
        let parent = host.active().unwrap().as_ref().unwrap().session.clone();
        let parent_id = parent.session_id().as_str().to_owned();
        parent
            .append_message(
                "fork-user-one",
                "fork-user-one",
                &timestamp(1),
                inline_message("Keep this first request."),
            )
            .unwrap();
        parent
            .append_message(
                "fork-user-two",
                "fork-user-two",
                &timestamp(2),
                inline_message("Restore this into the new composer."),
            )
            .unwrap();

        let text = host.session_user_text("fork-user-two", 0).unwrap();
        assert_eq!(
            String::from_utf8(STANDARD.decode(text["base64"].as_str().unwrap()).unwrap()).unwrap(),
            "Restore this into the new composer."
        );
        assert_eq!(text["next"], Value::Null);

        let history = host.session_history(0).unwrap();
        assert_eq!(history["total"], 2);
        assert_eq!(history["entries"][1]["role"], "user");
        assert_eq!(history["entries"][1]["parent"], "fork-user-one");

        let tree = host.select_history_entry("fork-user-one", false).unwrap();
        assert_eq!(
            host.runtime_status().unwrap()["last_terminal"]["operation"],
            tree["operation"]
        );
        assert_eq!(
            host.session_messages(10).unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert!(
            host.select_history_entry("fork-user-one", true)
                .unwrap_err()
                .contains("asynchronous native command")
        );

        let fork = host.fork_before_user_message("fork-user-two").unwrap();
        let child_id = host.view().unwrap()["snapshot"]["scope"]["session"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(child_id, parent_id);
        assert_eq!(
            host.runtime_status().unwrap()["last_terminal"]["operation"],
            fork["operation"]
        );
        assert_eq!(
            host.session_messages(10).unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // Reopening the child proves its new artifact origin was persisted in
        // the fork header rather than reusing the parent's origin by accident.
        host.open_session(&parent_id).unwrap();
        host.open_session(&child_id).unwrap();
        let child = host.active().unwrap().as_ref().unwrap().session.clone();
        child
            .compile_context("fork-context", &timestamp(3))
            .expect("forked journal should compile with its own origin");
    }

    #[tokio::test]
    async fn branch_summary_uses_the_local_model_and_stays_durable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sent, request_received) = mpsc::channel();
        let server = thread::spawn(move || {
            let result = (|| -> std::io::Result<Vec<u8>> {
                let (mut socket, _) = listener.accept()?;
                let request = read_http_request(&mut socket)?;
                let body = concat!(
                    "data: {\"id\":\"tree-summary-fixture\",\"choices\":[{\"delta\":{\"content\":\"The abandoned branch checked the second request.\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":8,\"total_tokens\":17}}\n\n",
                    "data: [DONE]\n\n"
                );
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
                socket.flush()?;
                Ok(request)
            })();
            let _ = request_sent.send(result.map_err(|error| error.to_string()));
        });

        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"tree-summary-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":format!("http://{address}/v1"),
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"],
                    "reasoning":false
                }],
                "selected":"tree-summary-fixture",
                "system_prompt":"Local tree summary fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();
        let session = host.active().unwrap().as_ref().unwrap().session.clone();
        session
            .append_message(
                "summary-user-one",
                "summary-user-one",
                &timestamp(1),
                inline_message("Keep this request in the restored composer."),
            )
            .unwrap();
        session
            .append_message(
                "summary-user-two",
                "summary-user-two",
                &timestamp(2),
                inline_message("This is the abandoned request to summarize."),
            )
            .unwrap();

        let started = host
            .begin_history_selection("summary-user-one", true)
            .await
            .unwrap();
        let operation = started["operation"].as_str().unwrap().to_owned();
        assert_eq!(started["pending"], true);
        assert_eq!(host.view().unwrap()["snapshot"]["status"], "streaming");

        host.run_started_history_selection("summary-user-one".into(), operation.clone())
            .await
            .unwrap();
        assert_eq!(
            host.runtime_status().unwrap()["last_terminal"]["operation"],
            operation
        );
        assert_eq!(
            host.runtime_status().unwrap()["last_terminal"]["status"],
            "completed"
        );
        assert_eq!(
            host.session_messages(10).unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let history = host.session_history(0).unwrap();
        let summary_id = history["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "branch_summary")
            .and_then(|entry| entry["id"].as_str())
            .expect("branch summary should be retained in history");
        let summary = host.session_entry(summary_id, 0).unwrap();
        let summary: Value = serde_json::from_slice(
            &STANDARD
                .decode(summary["base64"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert!(
            summary["summary"]
                .as_str()
                .is_some_and(|text| text.contains("abandoned branch"))
        );
        let compiled = session
            .compile_context("summary-context", &timestamp(3))
            .expect("the summarized branch should still compile");
        assert!(
            compiled
                .projection
                .messages
                .iter()
                .any(|message| message["role"] == "branchSummary")
        );

        let request = request_received
            .recv_timeout(Duration::from_secs(2))
            .expect("summary fixture never received an HTTP connection")
            .expect("summary fixture could not read the request");
        let body_offset = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let payload: Value = serde_json::from_slice(&request[body_offset..]).unwrap();
        assert!(
            payload["messages"]
                .to_string()
                .contains("abandoned request to summarize")
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn cancelling_a_branch_summary_keeps_the_original_branch_selected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (received_send, received) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (server_result, server_done) = mpsc::channel();
        let server = thread::spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let (mut socket, _) = listener.accept()?;
                let _ = read_http_request(&mut socket)?;
                let _ = received_send.send(());
                let _ = released.recv_timeout(Duration::from_secs(2));
                // The client normally closes after cancellation. Ignore a
                // broken pipe here because the request receipt is the fact
                // this fixture is exercising, not a successful response.
                let body = "data: [DONE]\n\n";
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.flush();
                Ok(())
            })();
            let _ = server_result.send(result.map_err(|error| error.to_string()));
        });

        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"tree-cancel-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":format!("http://{address}/v1"),
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":"tree-cancel-fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();
        let session = host.active().unwrap().as_ref().unwrap().session.clone();
        for (id, text) in [
            ("cancel-user-one", "Keep this request."),
            (
                "cancel-user-two",
                "This abandoned request must remain in history.",
            ),
        ] {
            session
                .append_message(id, id, &timestamp(now_ms()), inline_message(text))
                .unwrap();
        }

        let started = host
            .begin_history_selection("cancel-user-one", true)
            .await
            .unwrap();
        let operation = started["operation"].as_str().unwrap().to_owned();
        let worker = {
            let host = host.clone();
            tokio::spawn(async move {
                host.run_started_history_selection("cancel-user-one".into(), operation)
                    .await
            })
        };
        tokio::task::spawn_blocking(move || received.recv_timeout(Duration::from_secs(2)))
            .await
            .expect("summary fixture wait worker stopped")
            .expect("summary fixture never received the request");
        host.cancel().unwrap();
        let _ = release.send(());
        let outcome = tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("cancelled summary did not settle")
            .expect("summary worker panicked");
        assert!(outcome.is_err());
        assert_eq!(host.view().unwrap()["snapshot"]["status"], "idle");
        assert_eq!(
            host.runtime_status().unwrap()["last_terminal"]["status"],
            "failed"
        );
        let history = host.session_history(0).unwrap();
        assert!(
            !history["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["kind"] == "branch_summary")
        );
        assert_eq!(
            host.session_messages(10).unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(
            server_done
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_ok()
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn local_host_runs_a_durable_loopback_turn() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (server_result, server_done) = mpsc::channel();
        let server = thread::spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let (mut socket, _) = listener.accept()?;
                let mut request = [0u8; 16 * 1024];
                let _ = socket.read(&mut request)?;
                let body = concat!(
                    "data: {\"id\":\"desktop-fixture\",\"choices\":[{\"delta\":{\"content\":\"Desktop reply\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2,\"total_tokens\":9}}\n\n",
                    "data: [DONE]\n\n"
                );
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
                socket.flush()
            })();
            let _ = server_result.send(result.map_err(|error| error.to_string()));
        });

        let state = tempdir().unwrap();
        fs::write(
            state.path().join("native-host.json"),
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"desktop-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":format!("http://{address}/v1"),
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"],
                    "reasoning":false
                }],
                "selected":"desktop-fixture",
                "system_prompt":"Local desktop fixture"
            }))
            .unwrap(),
        )
        .unwrap();

        let host = NativeHost::open_from_config(
            state.path().to_path_buf(),
            state.path().join("native-host.json"),
        )
        .unwrap();
        let initial = host.open_general().unwrap();
        assert_eq!(initial["snapshot"]["model"], "desktop-fixture");

        let complete = host.prompt("Reply with the fixture text.".into()).await;
        if let Err(error) = &complete {
            panic!("local host rejected the fixture request before connecting: {error}");
        }
        let served = server_done
            .recv_timeout(Duration::from_secs(2))
            .expect("fixture never received an HTTP connection");
        assert!(served.is_ok(), "fixture error: {served:?}");
        let complete = complete.unwrap();
        assert_eq!(complete["snapshot"]["status"], "idle");
        assert_eq!(
            complete["snapshot"]["messages"].as_array().unwrap().len(),
            2
        );
        assert_eq!(complete["details"]["messages"][1]["text"], "Desktop reply");
        server.join().unwrap();

        let session = complete["snapshot"]["scope"]["session"]
            .as_str()
            .unwrap()
            .to_owned();
        drop(host);

        // Changing the picker must not change the model a saved journal uses
        // when the user clicks that chat in the sidebar later.
        let config_path = state.path().join("native-host.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        let mut alternate = config["models"][0].clone();
        alternate["id"] = json!("new-default");
        config["models"].as_array_mut().unwrap().push(alternate);
        config["selected"] = json!("new-default");
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();

        let resumed =
            NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        let resumed_view = resumed.open_session(&session).unwrap();
        assert_eq!(resumed_view["snapshot"]["model"], "desktop-fixture");
        assert_eq!(
            resumed_view["snapshot"]["workspace_path"].as_str(),
            Some(state.path().join(WORKSPACE).to_string_lossy().as_ref())
        );
        assert_eq!(
            resumed_view["snapshot"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn pasted_vision_image_reaches_the_local_openai_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sent, request_received) = mpsc::channel();
        let server = thread::spawn(move || {
            let result = (|| -> std::io::Result<Vec<u8>> {
                let (mut socket, _) = listener.accept()?;
                let request = read_http_request(&mut socket)?;
                let body = concat!(
                    "data: {\"id\":\"vision-fixture\",\"choices\":[{\"delta\":{\"content\":\"I can see it.\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2,\"total_tokens\":9}}\n\n",
                    "data: [DONE]\n\n"
                );
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
                socket.flush()?;
                Ok(request)
            })();
            let _ = request_sent.send(result.map_err(|error| error.to_string()));
        });

        let state = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"Qwen3.6-35B-A3B-UD-Q4_K_XL",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":format!("http://{address}/v1"),
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text", "image"],
                    "reasoning":false
                }],
                "selected":"Qwen3.6-35B-A3B-UD-Q4_K_XL",
                "system_prompt":"Local vision fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();

        let png = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEElEQVR4nGNgaGjAjoaWBADiMEABWMSWggAAAABJRU5ErkJggg==";
        let message = json!({"role":"user","content":[
            {"type":"text","text":"Describe this image."},
            {"type":"image","mimeType":"image/png","data":png}
        ]});
        let bytes = serde_json::to_vec(&message).unwrap();
        let upload = host.upload_attachment("upload_begin", &json!({})).unwrap();
        let upload_id = upload["upload"].as_str().unwrap();
        host.upload_attachment(
            "upload_append",
            &json!({"upload":upload_id,"offset":"0","base64":STANDARD.encode(&bytes)}),
        )
        .unwrap();
        let artifact = host
            .upload_attachment(
                "upload_finish",
                &json!({"upload":upload_id,"sha256":digest(&bytes)}),
            )
            .unwrap();
        let message = host
            .begin_uploaded_prompt(artifact["artifact"].as_str().unwrap())
            .unwrap();
        let complete = host.run_started_message(message).await;
        assert!(complete.is_ok(), "vision fixture failed: {complete:?}");

        let request = request_received
            .recv_timeout(Duration::from_secs(2))
            .expect("vision fixture never received an HTTP request")
            .expect("vision fixture could not read the request");
        let body_offset = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let payload: Value = serde_json::from_slice(&request[body_offset..]).unwrap();
        let user = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "user")
            .unwrap();
        assert_eq!(
            user["content"][0],
            json!({"type":"text","text":"Describe this image."})
        );
        assert_eq!(user["content"][1]["type"], "image_url");
        assert_eq!(
            user["content"][1]["image_url"]["url"],
            format!("data:image/png;base64,{png}")
        );
        assert!(user["content"][1].get("inlineData").is_none());
        assert_eq!(
            payload["chat_template_kwargs"]["enable_thinking"], true,
            "Qwen's native thinking option reaches llama.cpp"
        );
        server.join().unwrap();
    }

    #[test]
    fn model_catalog_is_published_before_a_new_host_reads_it() {
        let state = tempdir().unwrap();
        let config = HostConfig {
            models: vec![json!({
                "id":"saved-model",
                "provider":"llamacpp",
                "api":"openai-completions",
                "baseUrl":"http://127.0.0.1:18080/v1",
                "contextWindow":4096
            })],
            selected: "saved-model".into(),
            system_prompt: "Persisted local prompt".into(),
            runtime: None,
            extra: BTreeMap::new(),
        };

        write_config(state.path(), &config).unwrap();
        let reopened = read_config(&state.path().join("native-host.json"))
            .unwrap()
            .unwrap();
        assert_eq!(reopened.selected, "saved-model");
        assert_eq!(reopened.system_prompt, "Persisted local prompt");
    }

    #[tokio::test]
    async fn runtime_setup_seals_files_without_loading_a_model() {
        let state = tempdir().unwrap();
        let source = tempdir().unwrap();
        let server =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32\\cmd.exe");
        let model = source.path().join("fixture.gguf");
        let profile = source.path().join("profile.json");
        fs::write(&model, b"small fixture model").unwrap();
        fs::write(
            &profile,
            serde_json::to_vec(&json!({
                "version": 1,
                "models": {
                    "fixture-model": {
                        "contextWindow": 4096,
                        "maxTokens": 256,
                        "decodeThreads": 1,
                        "batchThreads": 1,
                        "nGpuLayers": 0,
                        "flashAttention": false
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let host = NativeHost::open(state.path().to_path_buf()).unwrap();
        let models = host
            .configure_owned_runtime(NativeModelSetup {
                model_id: "fixture-model".into(),
                server_path: server,
                model_path: model,
                profile_path: profile,
                projector_path: None,
                port: 19091,
                vram_mib: None,
                system_prompt: "Fixture local prompt".into(),
            })
            .await
            .unwrap();

        assert_eq!(models[0]["id"], "fixture-model");
        let config = read_config(&state.path().join("native-host.json"))
            .unwrap()
            .unwrap();
        assert_eq!(config.selected, "fixture-model");
        assert_eq!(config.system_prompt, "Fixture local prompt");
        assert_eq!(
            config.runtime.as_ref().unwrap().captures()[0]
                .verify()
                .unwrap()[0]
                .requested_context,
            4096
        );
        assert!(!state.path().join("owned-llama-runtime").exists());
    }

    #[tokio::test]
    async fn runtime_setup_keeps_two_models_in_one_picker_without_loading_them() {
        let state = tempdir().unwrap();
        let source = tempdir().unwrap();
        let server =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32\\cmd.exe");
        let first_model = source.path().join("first.gguf");
        let second_model = source.path().join("second.gguf");
        let profile = source.path().join("profile.json");
        fs::write(&first_model, b"first fixture model").unwrap();
        fs::write(&second_model, b"second fixture model").unwrap();
        fs::write(
            &profile,
            serde_json::to_vec(&json!({
                "version": 1,
                "models": {
                    "first-model": {
                        "contextWindow": 4096,
                        "maxTokens": 256,
                        "nGpuLayers": 0,
                        "flashAttention": false
                    },
                    "second-model": {
                        "contextWindow": 8192,
                        "maxTokens": 512,
                        "nGpuLayers": 0,
                        "flashAttention": false
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let host = NativeHost::open(state.path().to_path_buf()).unwrap();
        let models = host
            .configure_owned_runtimes(vec![
                NativeModelSetup {
                    model_id: "first-model".into(),
                    server_path: server.clone(),
                    model_path: first_model,
                    profile_path: profile.clone(),
                    projector_path: None,
                    port: 19094,
                    vram_mib: None,
                    system_prompt: "First local prompt".into(),
                },
                NativeModelSetup {
                    model_id: "second-model".into(),
                    server_path: server,
                    model_path: second_model,
                    profile_path: profile,
                    projector_path: None,
                    port: 19094,
                    vram_mib: None,
                    system_prompt: String::new(),
                },
            ])
            .await
            .unwrap();

        assert_eq!(models.as_array().unwrap().len(), 2);
        let config = read_config(&state.path().join("native-host.json"))
            .unwrap()
            .unwrap();
        assert_eq!(config.selected, "second-model");
        assert_eq!(config.system_prompt, "First local prompt");
        assert_eq!(config.runtime.as_ref().unwrap().captures().len(), 2);
        assert!(!state.path().join("owned-llama-runtime").exists());
    }

    #[tokio::test]
    async fn runtime_setup_detects_one_sibling_projector_without_loading_a_model() {
        let state = tempdir().unwrap();
        let source = tempdir().unwrap();
        let server =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32\\cmd.exe");
        let model = source.path().join("qwen-fixture.gguf");
        let projector = source.path().join("mmproj-qwen-fixture.gguf");
        let profile = source.path().join("profile.json");
        fs::write(&model, b"small fixture model").unwrap();
        fs::write(&projector, b"small fixture projector").unwrap();
        fs::write(
            &profile,
            serde_json::to_vec(&json!({
                "version": 1,
                "models": {
                    "vision-fixture": {
                        "contextWindow": 4096,
                        "maxTokens": 256,
                        "decodeThreads": 1,
                        "batchThreads": 1,
                        "nGpuLayers": 0,
                        "flashAttention": false
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let host = NativeHost::open(state.path().to_path_buf()).unwrap();
        let models = host
            .configure_owned_runtime(NativeModelSetup {
                model_id: "vision-fixture".into(),
                server_path: server,
                model_path: model,
                profile_path: profile,
                projector_path: None,
                port: 19093,
                vram_mib: None,
                system_prompt: String::new(),
            })
            .await
            .unwrap();

        assert_eq!(models[0]["input"], json!(["text", "image"]));
        let config = read_config(&state.path().join("native-host.json"))
            .unwrap()
            .unwrap();
        let captures = config.runtime.unwrap().captures();
        let plan = &captures[0].verify().unwrap()[0];
        assert!(
            plan.projector
                .as_deref()
                .is_some_and(|path| same_windows_path(Path::new(path), &projector))
        );
        assert!(!state.path().join("owned-llama-runtime").exists());
    }

    #[test]
    fn project_files_stay_inside_a_durable_windows_workspace() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let config_path = state.path().join("native-host.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "models":[{
                    "id":"project-fixture",
                    "provider":"llamacpp",
                    "api":"openai-completions",
                    "baseUrl":"http://127.0.0.1:19092/v1",
                    "contextWindow":4096,
                    "maxTokens":256,
                    "input":["text"]
                }],
                "selected":"project-fixture",
                "system_prompt":"Project fixture"
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(project.path().join("notes.txt"), "first line\r\n").unwrap();
        let project_path = project.path().to_string_lossy().into_owned();

        let host =
            NativeHost::open_from_config(state.path().to_path_buf(), config_path.clone()).unwrap();
        let opened = host.open_project(project.path().to_path_buf()).unwrap();
        assert!(
            opened["snapshot"]["workspace_path"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&project_path))
        );
        assert_eq!(
            opened["details"]["context"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            opened["details"]["context"]["tools"][0]["name"],
            "list_project_files"
        );
        assert_eq!(
            opened["details"]["context"]["tools"][2]["name"],
            "search_project_files"
        );
        assert!(
            opened["details"]["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|capability| capability == "project-read-tools")
        );
        assert!(
            opened["details"]["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|capability| capability == "project-write-tools-gated")
        );
        assert_eq!(
            opened["details"]["context"]["approval_policy"]["mode"],
            "interactive"
        );
        assert_eq!(host.list_files().unwrap(), json!(["notes.txt"]));

        let before = host.read_file("notes.txt").unwrap();
        assert_eq!(before["text"], "first line\r\n");
        let saved = host
            .save_file(
                "notes.txt",
                "second line\n",
                before["revision"].as_str().unwrap(),
            )
            .unwrap();
        assert_eq!(saved["text"], "second line\r\n");
        assert!(host.read_file("../outside.txt").is_err());

        let session = opened["snapshot"]["scope"]["session"]
            .as_str()
            .unwrap()
            .to_owned();
        let catalog = host.catalog_sessions(0, true).unwrap();
        assert!(
            catalog["chats"][0]["workspace"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&project_path))
        );
        drop(host);

        // A restart reconstructs the workspace from the durable journal and
        // must not downgrade a project chat into unrestricted general chat.
        let reopened =
            NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        let resumed = reopened.open_session(&session).unwrap();
        assert!(
            resumed["snapshot"]["workspace_path"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&project_path))
        );
        assert_eq!(
            reopened.read_file("notes.txt").unwrap()["text"],
            "second line\r\n"
        );
    }

    #[test]
    fn chat_folder_grants_are_durable_and_change_the_native_working_root() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let extra = tempdir().unwrap();
        let config = write_fixture_config(state.path(), "folder-fixture");
        fs::write(project.path().join("main.txt"), "project").unwrap();
        fs::write(extra.path().join("extra.txt"), "extra").unwrap();

        let host =
            NativeHost::open_from_config(state.path().to_path_buf(), config.clone()).unwrap();
        let opened = host.open_project(project.path().to_path_buf()).unwrap();
        let session = opened["snapshot"]["scope"]["session"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            opened["details"]["context"]["folder_access"]["folders"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let extra_path = extra.path().to_string_lossy().into_owned();
        let added = host
            .change_folder_access(&json!({
                "revision":1,"action":"add","path":extra_path,"mode":"read_only"
            }))
            .unwrap();
        assert_eq!(added["folders"].as_array().unwrap().len(), 2);
        assert_eq!(added["folders"][1]["mode"], "read_only");

        let selected = host
            .change_folder_access(&json!({
                "revision":2,"action":"primary","path":extra_path
            }))
            .unwrap();
        assert!(
            selected["primary"]
                .as_str()
                .is_some_and(|path| path.eq_ignore_ascii_case(&extra_path))
        );
        assert_eq!(host.list_files().unwrap(), json!(["extra.txt"]));
        assert!(host.read_file("../main.txt").is_err());
        assert!(
            host.catalog_sessions(0, true).unwrap()["chats"][0]["workspace"]
                .as_str()
                .is_some_and(|path| path.eq_ignore_ascii_case(&extra_path))
        );

        drop(host);
        let reopened = NativeHost::open_from_config(state.path().to_path_buf(), config).unwrap();
        let view = reopened.open_session(&session).unwrap();
        assert!(
            view["snapshot"]["workspace_path"]
                .as_str()
                .is_some_and(|path| path.eq_ignore_ascii_case(&extra_path))
        );
        assert!(
            view["details"]["context"]["folder_access"]["primary"]
                .as_str()
                .is_some_and(|path| path.eq_ignore_ascii_case(&extra_path))
        );
        assert_eq!(reopened.list_files().unwrap(), json!(["extra.txt"]));
    }

    #[test]
    fn project_write_permission_is_scoped_to_the_open_chat() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "policy-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_project(project.path().to_path_buf()).unwrap();

        host.set_approval_policy("autonomous").unwrap();
        let enabled = host.view().unwrap();
        assert_eq!(
            enabled["details"]["context"]["approval_policy"]["mode"],
            "autonomous"
        );
        assert_eq!(host.preferences()["approval_mode"], "autonomous");
        assert_eq!(
            enabled["details"]["context"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        assert_eq!(
            enabled["details"]["context"]["tools"][3]["name"],
            "edit_project_file"
        );
        assert_eq!(
            enabled["details"]["context"]["tools"][5]["name"],
            "run_project_command"
        );

        host.new_chat().unwrap();
        let fresh = host.view().unwrap();
        assert_eq!(
            fresh["details"]["context"]["approval_policy"]["mode"],
            "interactive"
        );
        assert!(host.set_approval_policy("always").is_err());
    }

    #[test]
    fn approval_change_after_a_prior_turn_updates_the_next_request_tools() {
        let state = tempdir().unwrap();
        let project = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "approval-fixture");
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_project(project.path().to_path_buf()).unwrap();
        {
            let active = host.active().unwrap();
            let active = active.as_ref().unwrap();
            active
                .session
                .append_message(
                    "prior-user",
                    "prior-user",
                    &timestamp(1),
                    inline_message("This chat already has history."),
                )
                .unwrap();
        }

        host.set_approval_policy("autonomous").unwrap();
        let updated = host.view().unwrap();
        assert_eq!(updated["snapshot"]["message_count"], 1);
        assert_eq!(
            updated["details"]["context"]["approval_policy"]["mode"],
            "autonomous"
        );
        assert_eq!(
            updated["details"]["context"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        assert_eq!(
            updated["details"]["context"]["tools"][5]["name"],
            "run_project_command"
        );
    }

    #[test]
    fn pasted_images_are_checked_then_retained_for_a_vision_model() {
        let state = tempdir().unwrap();
        let config_path = write_fixture_config(state.path(), "vision-fixture");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["models"][0]["input"] = json!(["text", "image"]);
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let host = NativeHost::open_from_config(state.path().to_path_buf(), config_path).unwrap();
        host.open_general().unwrap();

        let png = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEElEQVR4nGNgaGjAjoaWBADiMEABWMSWggAAAABJRU5ErkJggg==";
        let message = json!({"role":"user","content":[
            {"type":"text","text":"Describe this image."},
            {"type":"image","mimeType":"image/png","data":png}
        ]});
        let bytes = serde_json::to_vec(&message).unwrap();

        let upload = host.upload_attachment("upload_begin", &json!({})).unwrap();
        let id = upload["upload"].as_str().unwrap();
        host.upload_attachment(
            "upload_append",
            &json!({"upload":id,"offset":"0","base64":STANDARD.encode(&bytes)}),
        )
        .unwrap();
        let finished = host
            .upload_attachment(
                "upload_finish",
                &json!({"upload":id,"sha256":digest(&bytes)}),
            )
            .unwrap();
        let admitted = host
            .begin_uploaded_prompt(finished["artifact"].as_str().unwrap())
            .unwrap();
        assert_eq!(admitted, message);

        let projected = project_message(&json!({"id":"user-image","message":admitted})).unwrap();
        assert_eq!(projected["blocks"][1]["kind"], "attachment");
        assert_eq!(projected["blocks"][1]["reference"], "user-image");
        host.finish_prompt(None);
    }

    #[tokio::test]
    #[ignore = "requires a configured local AG1W model and GPU"]
    async fn live_owned_model_loads_answers_and_stops() {
        // This is intentionally opt-in. It exercises the same sealed runtime
        // path as the desktop without needing to drive a browser window.
        let state = std::env::var_os("AG1W_LIVE_STATE")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("AG1W"))
            })
            .expect("set AG1W_LIVE_STATE or LOCALAPPDATA");
        let model = std::env::var("AG1W_LIVE_MODEL")
            .expect("set AG1W_LIVE_MODEL to a configured local model ID");
        let host = NativeHost::open(state).expect("open AG1W state");

        let load = tokio::time::timeout(
            Duration::from_secs(900),
            host.load_model("llamacpp", &model),
        )
        .await;
        let prompt = if matches!(load, Ok(Ok(_))) {
            Some(
                tokio::time::timeout(
                    Duration::from_secs(180),
                    host.prompt("Reply with exactly: AG1W runtime smoke passed.".into()),
                )
                .await,
            )
        } else {
            None
        };

        let stopped = tokio::time::timeout(Duration::from_secs(30), host.shutdown()).await;
        assert!(matches!(&load, Ok(Ok(_))), "model load failed: {load:?}");
        assert!(
            matches!(&prompt, Some(Ok(Ok(_)))),
            "prompt failed: {prompt:?}"
        );
        let response = prompt
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .and_then(|result| result.as_ref().ok())
            .expect("a completed prompt returns the refreshed chat view");
        let answer = response["snapshot"]["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "assistant")
            })
            .and_then(|message| message["text"].as_str())
            .unwrap_or_default();
        assert!(
            !answer.trim().is_empty(),
            "the model finished without an assistant answer: {response}"
        );
        assert!(
            response["error"].is_null(),
            "the completed chat has a diagnostic: {response}"
        );
        assert!(
            matches!(&stopped, Ok(Ok(_))),
            "shutdown failed: {stopped:?}"
        );
    }
}
