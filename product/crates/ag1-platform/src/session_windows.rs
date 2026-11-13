//! Windows-native durable session base.
//!
//! This keeps AG1's journal and context compiler as the source of truth. It
//! deliberately does not pretend that Linux-only MCP, extension, GitHub, or
//! process adapters are available while their Windows implementations are
//! still being composed.

use crate::{
    change_receipts::PendingChange,
    folder_access::Snapshot as FolderSnapshot,
    mcp::{catalog::CatalogScope, manager::Manager as McpManager},
    process::{ProcessSpec, ProcessSupervisor},
    provider::{
        generation::{GenerationLimits, GenerationRequest, request_generation},
        http::{CancelToken, ProviderHttp, endpoint_origin},
        request_http::{HttpCredential, generation_http_request},
        requests::{RequestContext, session_options},
    },
    web_http::{SystemResolver, WebHttp, WebPolicy},
    web_tools::{SearchConfig, SearchService, WebExecution, WebService},
    workspace::{MAX_READ, Workspace, preserve_text},
};
use ag1_core::{
    DomainError, OperationId, SecretBytes, SessionId,
    context::{
        ContextArtifact, ContextArtifactPort,
        compaction::{self, Settings},
        compiler::Compilation,
        features::Features,
        summary_request::{self, SummaryRequest},
    },
    inference::{EndpointKind, Scheduler},
    provider::{ApiFamily, BlockKind, ProviderEvent, StopReason, assistant_message},
    runtime::{
        SessionRuntime, context as runtime_context, messages as runtime_messages,
        planning::{self, Plan, PlanRef, Selection, Status},
        session::{CompactionCapture, Configuration, PreparationAction},
        tree,
    },
    storage::StoragePort,
    tools::{
        approval::{Authorizer, Preflight, PreflightContext},
        policy::{Mode as ToolMode, Policy},
        registry::{Registry, Scope},
        research, web_presentation,
    },
};
use ag1_storage::{SessionBootstrap, SessionSeed, Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};

const PROJECT_TOOL_MAX_TEXT: usize = 32 * 1024;
const PROJECT_SEARCH_MAX_FILES: usize = 200;
const PROJECT_SEARCH_MAX_MATCHES: usize = 100;
const PROJECT_SEARCH_LINE_BYTES: usize = 400;
const MAX_LOCAL_TOOL_CALLS: usize = 8;
const PROJECT_COMMAND_MAX_ARGS: usize = 64;
const PROJECT_COMMAND_MAX_ARG_BYTES: usize = 4 * 1024;
const PROJECT_COMMAND_MAX_TIMEOUT_MS: u64 = 120_000;
const PROJECT_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
const WEB_TOOL_TIMEOUT: Duration = Duration::from_secs(45);
static PROJECT_COMMAND_SERIAL: AtomicU64 = AtomicU64::new(0);

/// One committed point in the journal. Callers can render this without seeing
/// the mutable session owner or its private storage handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Commit {
    pub revision: u64,
    pub sequence: u64,
}

/// The compiled provider input plus the custody records retained for it.
/// Keeping these references lets later desktop layers explain what context was
/// used without treating an in-memory transcript as durable evidence.
pub struct CompiledContext {
    pub projection: Compilation,
    pub original_journal: ag1_core::context::ContextArtifact,
    pub original_request: ag1_core::context::ContextArtifact,
    pub revision: u64,
}

/// The input for one local model turn. The caller supplies stable IDs so a
/// desktop retry can tell a failed request from a new user action.
pub struct LocalTurnInput {
    pub command: String,
    pub user_id: String,
    pub assistant_id: String,
    pub timestamp: String,
    pub timestamp_ms: u64,
    pub message: Value,
    // A planning request is still a normal durable turn, but it needs a small
    // request-only system addendum and no project tools. It never rewrites the
    // chat's stored configuration.
    pub system_addendum: Option<String>,
}

/// The durable facts produced by a completed local-model turn. The three
/// artifacts retain the pre-reduction journal, model request, and raw output.
pub struct LocalTurnResult {
    pub user: Commit,
    pub assistant: Commit,
    pub message: Value,
    pub original_journal: ContextArtifact,
    pub original_request: ContextArtifact,
    pub output: ContextArtifact,
    pub estimated_input_tokens: u64,
}

/// The durable facts produced by an explicit context compaction. The summary
/// itself is in the journal; the raw history and provider output remain in
/// artifacts so a future turn can rehydrate evidence instead of trusting a
/// lossy string.
pub struct LocalCompactionResult {
    pub result: Value,
    pub context_epoch: u64,
    pub journal_revision: u64,
}

/// A local planning turn plus the separately journaled plan it produced. A
/// plan can be absent when the model asked questions instead of proposing
/// executable steps; the assistant turn still remains durable either way.
pub struct LocalPlanResult {
    pub turn: LocalTurnResult,
    pub plan: Option<Plan>,
}

/// The public-web transport is scoped to one durable chat and context epoch.
/// Keeping it here preserves AG1's short search/fetch cache and search hints
/// across tool rounds without sharing either between chats.
struct LocalWeb {
    workspace: String,
    session: String,
    epoch: u64,
    fetch: Arc<WebService<SystemResolver>>,
    search: Arc<SearchService<SystemResolver>>,
}

/// A prompt gets this mode from the desktop's explicit approval setting.
/// The tool definitions stay small, but execution still decides whether a
/// model may change the checked project root for this one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectWriteAccess {
    ReadOnly,
    AllowWrites,
}

/// A narrow bridge from one durable chat to its reviewed MCP connections. The
/// manager still owns the child process; this value only fixes the chat scope
/// and keeps the model from seeing a whole catalog until it asks to search it.
#[derive(Clone)]
pub struct McpToolSurface {
    manager: Arc<McpManager>,
    scope: CatalogScope,
}

impl McpToolSurface {
    pub fn new(manager: Arc<McpManager>, scope: CatalogScope) -> Self {
        Self { manager, scope }
    }

    fn request_definitions() -> Vec<Value> {
        vec![
            json!({
                "name":"mcpSearch",
                "description":"Search the small catalog of reviewed, connected MCP tools for this chat. Use this before mcpCall so the required arguments come from the live server catalog.",
                "parameters":{"type":"object","properties":{"query":{"type":"string","description":"Tool name or capability to look for"},"maxResults":{"type":"integer","description":"Optional result limit from 1 to 8"}},"required":["query"]},
            }),
            json!({
                "name":"mcpCall",
                "description":"Call one reviewed MCP tool returned by mcpSearch. The connection and name must match that search result exactly; arguments must satisfy its returned inputSchema.",
                "parameters":{"type":"object","properties":{"connection":{"type":"string","description":"Connection returned by mcpSearch"},"name":{"type":"string","description":"Exact MCP tool name returned by mcpSearch"},"arguments":{"type":"object","description":"Arguments matching the selected tool inputSchema"}},"required":["connection","name","arguments"]},
            }),
        ]
    }
}

/// The first native tool surface stays deliberately small and project-bound.
/// This is enough for a coding session without loading the full AG1 catalog
/// into every local-model request.
pub struct LocalProjectTools {
    // `workspace` is the working folder used for relative paths and direct
    // build commands.  A frozen folder snapshot, when present, additionally
    // lets the file tools reach the other folders the user explicitly granted
    // for this exact turn.
    workspace: Arc<Workspace>,
    folders: Option<FolderSnapshot>,
    write_access: ProjectWriteAccess,
    command_logs: Option<PathBuf>,
}

/// A tool call has already been committed as an assistant message when this
/// runner is created. Keeping that journal identity beside the file effect
/// lets a later review prove which turn requested a change.
struct LocalChangeContext<'a> {
    store: &'a Store,
    session: &'a str,
    operation: &'a str,
    timestamp_ms: u64,
}

impl LocalChangeContext<'_> {
    fn prepare(
        &self,
        workspace: &Workspace,
        call: &Value,
        path: &str,
        expected: Option<&ag1_core::FileRevision>,
        after: &[u8],
    ) -> Result<Option<PendingChange>, DomainError> {
        let call_id = call["id"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("native project tool call".into()))?;
        let tool = call["name"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("native project tool name".into()))?;
        PendingChange::prepare_local(
            self.store,
            workspace,
            self.session,
            workspace.id().as_str(),
            self.operation,
            call_id,
            tool,
            path,
            expected,
            after,
            self.timestamp_ms,
        )
    }
}

impl LocalProjectTools {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self::with_write_access(workspace, ProjectWriteAccess::ReadOnly)
    }

    pub fn with_write_access(workspace: Arc<Workspace>, write_access: ProjectWriteAccess) -> Self {
        Self {
            workspace,
            folders: None,
            write_access,
            command_logs: None,
        }
    }

    /// The desktop supplies a private state directory for short-lived command
    /// logs. This does not grant a shell: `run_project_command` still accepts
    /// only a fixed set of direct developer executables.
    pub fn with_command_runner(
        workspace: Arc<Workspace>,
        write_access: ProjectWriteAccess,
        command_logs: PathBuf,
    ) -> Self {
        Self {
            workspace,
            folders: None,
            write_access,
            command_logs: Some(command_logs),
        }
    }

    /// The host takes one immutable permission snapshot when it admits a
    /// prompt.  A later folder change cannot expand an already-running model
    /// turn, and a removed grant cannot be used by an old tool runner.
    pub fn with_folder_access(
        folders: FolderSnapshot,
        write_access: ProjectWriteAccess,
        command_logs: PathBuf,
    ) -> Self {
        let workspace = folders.primary();
        Self {
            workspace,
            folders: Some(folders),
            write_access,
            command_logs: Some(command_logs),
        }
    }

    pub fn definitions() -> Vec<Value> {
        Self::definitions_for(ProjectWriteAccess::ReadOnly)
    }

    pub fn definitions_for(write_access: ProjectWriteAccess) -> Vec<Value> {
        Self::definitions_for_command_runner(write_access, false)
    }

    pub fn definitions_for_command_runner(
        write_access: ProjectWriteAccess,
        has_command_runner: bool,
    ) -> Vec<Value> {
        let mut definitions = vec![
            json!({
                "name":"list_project_files",
                "description":"List regular files in the user-granted Chat folders. Relative paths are in the working folder; other folders use absolute paths. Generated dependency folders are excluded.",
                "parameters":{"type":"object","properties":{},"required":[]},
            }),
            json!({
                "name":"read_project_file",
                "description":"Read a UTF-8 text file in a user-granted Chat folder. Use a relative path in the working folder or an absolute path from the file listing.",
                "parameters":{"type":"object","properties":{"path":{"type":"string","description":"Relative working-folder path or absolute granted-file path"}},"required":["path"]},
            }),
            json!({
                "name":"search_project_files",
                "description":"Find literal text in UTF-8 files in the user-granted Chat folders. Results are bounded to relevant matching lines.",
                "parameters":{"type":"object","properties":{"query":{"type":"string","description":"Text to find in project files"}},"required":["query"]},
            }),
        ];
        if write_access == ProjectWriteAccess::AllowWrites {
            definitions.extend([
            json!({
                "name":"edit_project_file",
                "description":"Replace one exact piece of text in a user-granted writable UTF-8 file. Read the file first and use text that appears exactly once.",
                "parameters":{"type":"object","properties":{"path":{"type":"string","description":"Relative working-folder path or absolute granted-file path"},"oldText":{"type":"string","description":"Exact existing text, appearing once"},"newText":{"type":"string","description":"Replacement text; may be empty"}},"required":["path","oldText","newText"]},
            }),
            json!({
                "name":"write_project_file",
                "description":"Create or replace one bounded UTF-8 file in a user-granted writable folder. Use this for a new file or a full deliberate rewrite.",
                "parameters":{"type":"object","properties":{"path":{"type":"string","description":"Relative working-folder path or absolute granted-file path"},"content":{"type":"string","description":"Complete new UTF-8 file content"},"createParents":{"type":"boolean","description":"Create missing parent folders only when needed"}},"required":["path","content"]},
            }),
            ]);
            if has_command_runner {
                definitions.push(json!({
                    "name":"run_project_command",
                    "description":"Run one direct developer executable in the current project and return bounded combined output. This accepts cargo, dotnet, git, node, or python plus a separate argument list. Do not use shell syntax, pipes, redirects, command chaining, or interpreter eval flags.",
                    "parameters":{"type":"object","properties":{"program":{"type":"string","description":"One of: cargo, dotnet, git, node, python"},"args":{"type":"array","items":{"type":"string"},"description":"Separate command-line arguments; never a shell command string"},"timeoutMs":{"type":"integer","description":"Optional timeout from 1000 to 120000 milliseconds"}},"required":["program","args"]},
                }));
            }
        }
        definitions
    }

    /// Web research is intentionally a separate two-tool surface. It is useful
    /// in a general chat too, while project tools still require a granted folder.
    pub fn web_definitions() -> Vec<Value> {
        vec![
            json!({
                "name":"webSearch",
                "description":"Search the public web using DuckDuckGo. Returns bounded titles, URLs, and snippets with durable source evidence. Use it for current information, documentation, or error messages.",
                "parameters":{"type":"object","properties":{"query":{"type":"string","description":"Search query"},"maxResults":{"type":"number","description":"Maximum results to return, from 1 to 4"}},"required":["query"]},
            }),
            json!({
                "name":"webFetch",
                "description":"Fetch a public URL and return a compact, evidence-backed reading extract. Use it after webSearch when a source needs closer reading.",
                "parameters":{"type":"object","properties":{"url":{"type":"string","description":"Public http or https URL to fetch"}},"required":["url"]},
            }),
        ]
    }

    fn request_definitions(&self) -> Vec<Value> {
        Self::definitions_for_command_runner(self.write_access, self.command_logs.is_some())
    }

    fn execute(&self, call: &Value) -> Value {
        self.execute_with_changes(call, None)
    }

    fn execute_with_changes(&self, call: &Value, change: Option<&LocalChangeContext<'_>>) -> Value {
        let id = call["id"].as_str().unwrap_or_default();
        let name = call["name"].as_str().unwrap_or_default();
        let result = match name {
            "list_project_files" => self.list_files(),
            "read_project_file" => {
                let path = call["arguments"]["path"]
                    .as_str()
                    .ok_or(DomainError::InvalidInput("project file path".into()));
                path.and_then(|path| {
                    let (workspace, path) = self.resolve_path(path, false)?;
                    let snapshot =
                        workspace.snapshot(&path, MAX_READ.min(PROJECT_TOOL_MAX_TEXT))?;
                    String::from_utf8(snapshot.bytes)
                        .map_err(|_| DomainError::InvalidInput("project text file".into()))
                })
            }
            "search_project_files" => call["arguments"]["query"]
                .as_str()
                .ok_or(DomainError::InvalidInput("project search query".into()))
                .and_then(|query| self.search_files(query)),
            "edit_project_file" => self.edit(call, change),
            "write_project_file" => self.write(call, change),
            "run_project_command" => self.run_command(call),
            _ => Err(DomainError::InvalidInput("native project tool".into())),
        };
        match result {
            Ok(text) => json!({
                "role":"toolResult",
                "toolCallId":id,
                "toolName":name,
                "content":[{"type":"text","text":text}],
                "isError":false,
            }),
            Err(error) => json!({
                "role":"toolResult",
                "toolCallId":id,
                "toolName":name,
                "content":[{"type":"text","text":project_tool_error(&error)}],
                "isError":true,
            }),
        }
    }

    fn resolve_path(
        &self,
        path: &str,
        write: bool,
    ) -> Result<(Arc<Workspace>, String), DomainError> {
        if let Some(folders) = &self.folders {
            return folders.resolve_relative(path, write);
        }
        if std::path::Path::new(path).is_absolute() {
            return Err(DomainError::InvalidInput("project file path".into()));
        }
        Ok((self.workspace.clone(), path.into()))
    }

    fn list_files(&self) -> Result<String, DomainError> {
        let Some(folders) = &self.folders else {
            return self.workspace.scan(1_000).map(project_file_list);
        };
        let primary = folders.primary();
        let mut files = Vec::new();
        for root in folders.roots() {
            for path in root.scan(1_000usize.saturating_sub(files.len()))? {
                if root.root() == primary.root() {
                    files.push(path);
                } else {
                    files.push(root.root().join(path).to_string_lossy().replace('\\', "/"));
                }
                if files.len() == 1_000 {
                    return Ok(project_file_list(files));
                }
            }
        }
        files.sort();
        Ok(project_file_list(files))
    }

    fn search_files(&self, query: &str) -> Result<String, DomainError> {
        let Some(folders) = &self.folders else {
            return project_text_search(self.workspace.as_ref(), query);
        };
        let primary = folders.primary();
        let mut matches = Vec::new();
        for root in folders.roots() {
            let text = project_text_search(root.as_ref(), query)?;
            if text == "No matching project text found." {
                continue;
            }
            let label = if root.root() == primary.root() {
                "Working folder".to_owned()
            } else {
                root.root().display().to_string()
            };
            matches.push(format!("[{label}]\n{text}"));
        }
        if matches.is_empty() {
            Ok("No matching project text found.".into())
        } else {
            Ok(bounded_project_tool_text(matches.join("\n\n")))
        }
    }

    fn edit(
        &self,
        call: &Value,
        change: Option<&LocalChangeContext<'_>>,
    ) -> Result<String, DomainError> {
        self.require_write_access()?;
        let path = project_argument(call, "path")?;
        let old = project_argument(call, "oldText")?;
        let new = project_argument(call, "newText")?;
        if old.is_empty() {
            return Err(DomainError::InvalidInput("project edit old text".into()));
        }
        let (workspace, path) = self.resolve_path(path, true)?;
        let snapshot = workspace.snapshot(&path, MAX_READ.min(PROJECT_TOOL_MAX_TEXT))?;
        let text = std::str::from_utf8(&snapshot.bytes)
            .map_err(|_| DomainError::InvalidInput("project text file".into()))?;
        let matches = text.matches(old).count();
        if matches != 1 {
            return Err(DomainError::InvalidInput("project edit exact match".into()));
        }
        // Build the new text before touching disk. `preserve_text` retains the
        // file's existing BOM and newline convention so this one edit stays small.
        let replacement = text.replacen(old, new, 1);
        let bytes = preserve_text(&snapshot, &replacement)?;
        if bytes.len() > PROJECT_TOOL_MAX_TEXT {
            return Err(DomainError::LimitExceeded("project write bytes".into()));
        }
        let receipt = change
            .map(|context| {
                context.prepare(
                    workspace.as_ref(),
                    call,
                    &path,
                    Some(&snapshot.revision),
                    &bytes,
                )
            })
            .transpose()?
            .flatten();
        let published = workspace.replace(&path, &snapshot.revision, &bytes);
        let note = settle_project_change(receipt, change, &published);
        published?;
        Ok(project_write_result(&format!("Updated {path}."), note))
    }

    fn write(
        &self,
        call: &Value,
        change: Option<&LocalChangeContext<'_>>,
    ) -> Result<String, DomainError> {
        self.require_write_access()?;
        let path = project_argument(call, "path")?;
        let content = project_argument(call, "content")?;
        if content.len() > PROJECT_TOOL_MAX_TEXT {
            return Err(DomainError::LimitExceeded("project write bytes".into()));
        }
        let create_parents = call["arguments"]["createParents"]
            .as_bool()
            .unwrap_or(false);
        let (workspace, path) = self.resolve_path(path, true)?;
        match workspace.snapshot(&path, MAX_READ.min(PROJECT_TOOL_MAX_TEXT)) {
            Ok(snapshot) => {
                // Keep a deliberate whole-file rewrite from also changing its
                // newline style or UTF-8 BOM.
                let bytes = preserve_text(&snapshot, content)?;
                if bytes.len() > PROJECT_TOOL_MAX_TEXT {
                    return Err(DomainError::LimitExceeded("project write bytes".into()));
                }
                let receipt = change
                    .map(|context| {
                        context.prepare(
                            workspace.as_ref(),
                            call,
                            &path,
                            Some(&snapshot.revision),
                            &bytes,
                        )
                    })
                    .transpose()?
                    .flatten();
                let published = workspace.replace(&path, &snapshot.revision, &bytes);
                let note = settle_project_change(receipt, change, &published);
                published?;
                Ok(project_write_result(&format!("Replaced {path}."), note))
            }
            Err(DomainError::NotFound) => {
                let receipt = change
                    .map(|context| {
                        context.prepare(workspace.as_ref(), call, &path, None, content.as_bytes())
                    })
                    .transpose()?
                    .flatten();
                let published =
                    workspace.create_checked(&path, content.as_bytes(), create_parents, || Ok(()));
                let note = settle_project_change(receipt, change, &published);
                published?;
                Ok(project_write_result(&format!("Created {path}."), note))
            }
            Err(error) => Err(error),
        }
    }

    fn run_command(&self, call: &Value) -> Result<String, DomainError> {
        self.require_write_access()?;
        let logs = self
            .command_logs
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("Windows project command runner".into()))?;
        let program = call["arguments"]["program"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("project command program".into()))?;
        let args = project_command_arguments(program, &call["arguments"]["args"])?;
        // Validate the untrusted argument shape before consulting PATH. That
        // keeps an unavailable optional executable from hiding a rejected eval
        // flag or a malformed model tool call.
        let executable = project_command_executable(program)?;
        let timeout_ms = call["arguments"]["timeoutMs"].as_u64().unwrap_or(60_000);
        if !(1_000..=PROJECT_COMMAND_MAX_TIMEOUT_MS).contains(&timeout_ms) {
            return Err(DomainError::InvalidInput("project command timeout".into()));
        }

        fs::create_dir_all(logs).map_err(crate::io_error)?;
        let serial = PROJECT_COMMAND_SERIAL.fetch_add(1, Ordering::Relaxed) + 1;
        let spool = logs.join(format!("project-command-{serial}.log"));
        let supervisor = ProcessSupervisor::new(&executable)?;
        let spawn = supervisor.spawn(
            ProcessSpec {
                executable,
                argv: args.clone(),
                cwd: self.workspace.root().to_path_buf(),
                env: project_command_environment(),
                timeout_ms,
                grace_ms: 500,
                resident: false,
            },
            &spool,
        );
        let mut child = match spawn {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_file(&spool);
                return Err(error);
            }
        };
        let status = child.wait(Duration::from_millis(timeout_ms));
        let output = child
            .tail(PROJECT_COMMAND_OUTPUT_BYTES)
            .map(|view| {
                let mut text = String::from_utf8_lossy(&view.bytes).into_owned();
                if view.truncated {
                    text.push_str("\n[Earlier command output was truncated.]\n");
                }
                text
            })
            .unwrap_or_else(|_| "[Command output could not be read.]".into());
        drop(child);
        let _ = fs::remove_file(&spool);

        match status {
            Ok(status) => Ok(format!(
                "Program: {program}\nExit code: {}\n\n{output}",
                status.code().unwrap_or(-1)
            )),
            Err(DomainError::TimedOut) => Ok(format!(
                "Program: {program}\nResult: timed out after {timeout_ms} ms; AG1W stopped the owned process tree.\n\n{output}"
            )),
            Err(error) => Err(error),
        }
    }

    fn require_write_access(&self) -> Result<(), DomainError> {
        if self.write_access == ProjectWriteAccess::AllowWrites {
            Ok(())
        } else {
            Err(DomainError::InvalidInput("project write permission".into()))
        }
    }
}

fn settle_project_change(
    receipt: Option<PendingChange>,
    change: Option<&LocalChangeContext<'_>>,
    published: &Result<ag1_core::FileRevision, DomainError>,
) -> Option<&'static str> {
    let (Some(receipt), Some(change)) = (receipt, change) else {
        return None;
    };
    if receipt.settle(change.store, published).is_err() {
        // The file result remains authoritative. Asking the model to repeat a
        // successful write would be worse than reporting the missing review
        // record plainly to the user.
        return Some(
            " The file result is final, but AG1 could not finalize its review receipt. Do not retry this write automatically.",
        );
    }
    None
}

fn project_write_result(result: &str, note: Option<&str>) -> String {
    let mut text = result.to_owned();
    if let Some(note) = note {
        text.push_str(note);
    }
    text
}

fn project_argument<'a>(call: &'a Value, name: &str) -> Result<&'a str, DomainError> {
    call["arguments"][name]
        .as_str()
        .ok_or_else(|| DomainError::InvalidInput(format!("project {name}")))
}

fn project_command_executable(program: &str) -> Result<PathBuf, DomainError> {
    // Direct executable names stop the model from smuggling shell syntax through
    // this tool. A command can still modify the project, which is why this tool
    // is available only when the user explicitly allowed project changes.
    let program = program.to_ascii_lowercase();
    if !matches!(
        program.as_str(),
        "cargo" | "dotnet" | "git" | "node" | "python"
    ) {
        return Err(DomainError::InvalidInput("project command program".into()));
    }
    let executable = format!("{program}.exe");
    for folder in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = folder.join(&executable);
        if candidate.is_file() {
            return candidate.canonicalize().map_err(crate::io_error);
        }
    }
    Err(DomainError::Unavailable(format!(
        "AG1W could not find {program}.exe on PATH"
    )))
}

fn project_command_arguments(program: &str, value: &Value) -> Result<Vec<String>, DomainError> {
    let values = value
        .as_array()
        .ok_or_else(|| DomainError::InvalidInput("project command arguments".into()))?;
    if values.len() > PROJECT_COMMAND_MAX_ARGS {
        return Err(DomainError::LimitExceeded(
            "project command arguments".into(),
        ));
    }
    let mut arguments = Vec::with_capacity(values.len());
    for value in values {
        let value = value
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("project command arguments".into()))?;
        if value.len() > PROJECT_COMMAND_MAX_ARG_BYTES || value.contains('\0') {
            return Err(DomainError::InvalidInput(
                "project command arguments".into(),
            ));
        }
        arguments.push(value.to_owned());
    }
    // `dotnet -c Release` and `git -c key=value` are ordinary direct-tool
    // arguments. Only Node and Python have eval flags which turn this narrow
    // runner back into a general script interface.
    let program = program.to_ascii_lowercase();
    let eval_flag = match program.as_str() {
        "node" => arguments.iter().any(|argument| {
            matches!(argument.as_str(), "-e" | "--eval") || argument.starts_with("--eval=")
        }),
        "python" => arguments.iter().any(|argument| {
            argument == "-c" || argument.starts_with("-c") || argument == "--command"
        }),
        _ => false,
    };
    if eval_flag {
        return Err(DomainError::InvalidInput(
            "project command arguments".into(),
        ));
    }
    Ok(arguments)
}

fn project_command_environment() -> BTreeMap<String, String> {
    // CreateProcess receives only this explicit environment. The small allow
    // list is enough for ordinary developer tools without copying secrets or
    // every ambient variable from AG1W into an untrusted project command.
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
    environment
}

fn project_file_list(paths: Vec<String>) -> String {
    let mut text = String::new();
    for path in paths {
        let separator = if text.is_empty() { "" } else { "\n" };
        if text.len() + separator.len() + path.len() > PROJECT_TOOL_MAX_TEXT {
            // The model needs a useful starting point, not every generated or
            // deeply nested path in a large project. It can read a known file next.
            if text.is_empty() {
                return "[A project path is too long to include.]".into();
            }
            text.push_str("\n[File list truncated; ask for a narrower path.]");
            break;
        }
        text.push_str(separator);
        text.push_str(&path);
    }
    text
}

fn bounded_project_tool_text(mut text: String) -> String {
    if text.len() <= PROJECT_TOOL_MAX_TEXT {
        return text;
    }
    let suffix = "\n[Search results truncated; narrow the query or choose one folder.]";
    let mut end = PROJECT_TOOL_MAX_TEXT.saturating_sub(suffix.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(suffix);
    text
}

fn project_text_search(workspace: &Workspace, query: &str) -> Result<String, DomainError> {
    let query = query.trim();
    if query.is_empty() || query.len() > 256 {
        return Err(DomainError::InvalidInput("project search query".into()));
    }
    let needle = query.to_lowercase();
    let mut output = String::new();
    let mut matches = 0usize;
    for path in workspace
        .scan(1_000)?
        .into_iter()
        .take(PROJECT_SEARCH_MAX_FILES)
    {
        let snapshot = match workspace.snapshot(&path, MAX_READ.min(PROJECT_TOOL_MAX_TEXT)) {
            Ok(snapshot) => snapshot,
            // Search should remain helpful even when one dependency-generated
            // or binary-like file cannot be represented as compact UTF-8 text.
            Err(DomainError::LimitExceeded(_)) => continue,
            Err(error) => return Err(error),
        };
        let Ok(text) = String::from_utf8(snapshot.bytes) else {
            continue;
        };
        for (line_number, line) in text.lines().enumerate() {
            if !line.to_lowercase().contains(&needle) {
                continue;
            }
            let preview = bounded_line_preview(line);
            let entry = format!("{path}:{}:{preview}", line_number + 1);
            if !append_search_result(&mut output, &entry)
                || matches + 1 == PROJECT_SEARCH_MAX_MATCHES
            {
                return Ok(output);
            }
            matches += 1;
        }
    }
    if output.is_empty() {
        Ok("No matching text was found in the searched project files.".into())
    } else {
        Ok(output)
    }
}

fn bounded_line_preview(line: &str) -> String {
    if line.len() <= PROJECT_SEARCH_LINE_BYTES {
        return line.into();
    }
    let end = line
        .char_indices()
        .take_while(|(index, _)| *index < PROJECT_SEARCH_LINE_BYTES)
        .last()
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    format!("{}…", &line[..end])
}

fn append_search_result(output: &mut String, entry: &str) -> bool {
    let separator = if output.is_empty() { "" } else { "\n" };
    let trailer = "[Search results truncated; refine the query.]";
    if output.len() + separator.len() + entry.len() + trailer.len() + 1 > PROJECT_TOOL_MAX_TEXT {
        if output.is_empty() {
            output.push_str("[A matching project path is too long to include.]");
        } else {
            output.push_str("\n");
            output.push_str(trailer);
        }
        return false;
    }
    output.push_str(separator);
    output.push_str(entry);
    true
}

fn project_tool_error(error: &DomainError) -> &'static str {
    match error {
        DomainError::NotFound => "The project file was not found.",
        DomainError::PermissionDenied => {
            "That path is outside the project or crosses a Windows reparse point."
        }
        DomainError::LimitExceeded(_) => "That project file is too large for this read tool.",
        DomainError::Conflict => {
            "The project changed while the tool was reading it. Retry the tool call."
        }
        DomainError::InvalidInput(value) if value == "project search query" => {
            "Use a short, non-empty text search query."
        }
        DomainError::InvalidInput(value) if value == "project write permission" => {
            "This chat is set to ask before changes. Choose Allow routine changes or Bypass approvals, then ask again."
        }
        DomainError::InvalidInput(value) if value == "project edit old text" => {
            "Use non-empty oldText copied exactly from the file."
        }
        DomainError::InvalidInput(value) if value == "project edit exact match" => {
            "The oldText must appear exactly once. Read the file again and make the edit more specific."
        }
        DomainError::InvalidInput(value) if value == "project command program" => {
            "Use one direct developer program: cargo, dotnet, git, node, or python. Shell command strings are not supported."
        }
        DomainError::InvalidInput(value) if value == "project command arguments" => {
            "Use a short array of plain command-line arguments; shell and interpreter eval flags are not supported."
        }
        DomainError::InvalidInput(value) if value == "project command timeout" => {
            "Use a command timeout between 1000 and 120000 milliseconds."
        }
        DomainError::InvalidInput(_) => "Use a relative UTF-8 project file path.",
        _ => "The project tool could not complete safely.",
    }
}

fn unavailable_local_tool_result(call: &Value) -> Value {
    json!({
        "role":"toolResult",
        "toolCallId":call["id"],
        "toolName":call["name"],
        "content":[{"type":"text","text":"Attach a project before using file or command tools."}],
        "isError":true,
    })
}

fn repeated_tool_error_result(call: &Value) -> Value {
    json!({
        "role":"toolResult",
        "toolCallId":call["id"],
        "toolName":call["name"],
        "content":[{"type":"text","text":"This exact tool call already failed during this answer, so AG1W did not run it again. Use materially different arguments or explain the limitation from the recorded error."}],
        "isError":true,
    })
}

fn repeats_failed_call(failed_calls: &[(String, Value)], call: &Value) -> bool {
    let name = call["name"].as_str().unwrap_or_default();
    failed_calls.iter().any(|(previous_name, previous_args)| {
        previous_name == name && previous_args == &call["arguments"]
    })
}

fn web_tool_result(call: &Value, result: Value) -> Value {
    let name = call["name"].as_str().unwrap_or_default();
    let content = result["content"].as_array().cloned().unwrap_or_else(|| {
        vec![json!({"type":"text","text":"The web tool returned no readable result."})]
    });
    let presentation = web_presentation::render(name, &call["arguments"], &result, false)
        .ok()
        .and_then(|value| serde_json::to_value(value).ok());
    json!({
        "role":"toolResult",
        "toolCallId":call["id"],
        "toolName":name,
        "content":content,
        "details":result["details"],
        "presentation":presentation,
        "isError":result["isError"].as_bool().unwrap_or(false),
    })
}

fn web_tool_error_result(call: &Value, error: DomainError) -> Value {
    let message = match error {
        DomainError::PermissionDenied => "That URL is outside AG1W's public-web safety boundary.",
        DomainError::Cancelled => "Web research was stopped.",
        DomainError::TimedOut => "Web research timed out.",
        DomainError::QueueFull => "Web research is busy. Retry shortly.",
        DomainError::InvalidInput(reason)
        | DomainError::LimitExceeded(reason)
        | DomainError::Unavailable(reason) => {
            return json!({
                "role":"toolResult",
                "toolCallId":call["id"],
                "toolName":call["name"],
                "content":[{"type":"text","text":reason}],
                "details":{"error":reason,"status":"error"},
                "isError":true,
            });
        }
        _ => "Web research could not complete safely.",
    };
    json!({
        "role":"toolResult",
        "toolCallId":call["id"],
        "toolName":call["name"],
        "content":[{"type":"text","text":message}],
        "details":{"error":message,"status":"error"},
        "isError":true,
    })
}

fn mcp_tool_result(call: &Value, result: Value) -> Value {
    let content = result["result"]["content"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| {
            vec![json!({
                "type":"text",
                "text":bounded_mcp_text(&result),
            })]
        });
    json!({
        "role":"toolResult",
        "toolCallId":call["id"],
        "toolName":call["name"],
        "content":content,
        "details":{"mcp":{"connection":call["arguments"]["connection"],"name":call["arguments"]["name"]}},
        "isError":result["result"]["isError"].as_bool().unwrap_or(false),
    })
}

fn mcp_tool_error_result(call: &Value, error: DomainError) -> Value {
    let text = match error {
        DomainError::Cancelled => "The MCP call was stopped and its connection was retired.".into(),
        DomainError::TimedOut => "The MCP call timed out and its connection was retired.".into(),
        DomainError::QueueFull => "That MCP connection is already handling a tool call.".into(),
        DomainError::NotFound => "That MCP connection or tool is no longer available. Search the MCP catalog again.".into(),
        DomainError::PermissionDenied => "That MCP tool is not enabled for this chat or its reviewed connection is no longer ready.".into(),
        DomainError::Conflict => "The MCP tool catalog changed. Search it again before calling a tool.".into(),
        DomainError::InvalidInput(reason)
        | DomainError::LimitExceeded(reason)
        | DomainError::Unavailable(reason) => reason,
        _ => "The MCP tool could not complete safely.".into(),
    };
    json!({
        "role":"toolResult",
        "toolCallId":call["id"],
        "toolName":call["name"],
        "content":[{"type":"text","text":text}],
        "details":{"mcp":{"status":"error"}},
        "isError":true,
    })
}

fn bounded_mcp_text(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| {
        "MCP returned a result that AG1W could not encode for this chat.".into()
    });
    if text.len() <= PROJECT_TOOL_MAX_TEXT {
        return text;
    }
    let end = text
        .char_indices()
        .take_while(|(index, _)| *index < PROJECT_TOOL_MAX_TEXT.saturating_sub(64))
        .last()
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    format!(
        "{}\n[MCP result truncated in chat; reconnect and inspect its catalog for details.]",
        &text[..end]
    )
}

// Gregorian civil year from nonnegative Unix milliseconds. Search planning
// needs the year, and this avoids a second time library in the native host.
fn utc_year(milliseconds: u64) -> u32 {
    let days = milliseconds / 86_400_000 + 719468;
    let era = days / 146097;
    let day = days % 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let day_of_year = day - (365 * year + year / 4 - year / 100);
    let month = (5 * day_of_year + 2) / 153;
    (year + era * 400 + u64::from(month >= 10)) as u32
}

/// A small Windows owner around the real AG1 session state machine.
///
/// The mutex serializes journal transitions. SQLite is still the durable
/// authority: a state change is visible here only after its journal receipt is
/// acknowledged by `SessionRuntime`.
pub struct NativeSession {
    state: Mutex<SessionRuntime>,
    store: Arc<Store>,
    session: SessionId,
    origin: String,
    // The journal owns this value. The desktop uses it to reopen the same
    // project folder rather than treating every durable chat as general chat.
    cwd: String,
    web: Mutex<Option<LocalWeb>>,
}

impl NativeSession {
    /// Create an empty journal with a cursor key generated by Windows CNG.
    /// The key is deliberately host-only; it is never placed in a prompt or UI
    /// payload.
    pub fn create(
        store: Arc<Store>,
        session: SessionId,
        origin: &str,
        cwd: &str,
        timestamp: &str,
        epoch: u64,
        configuration: Configuration,
    ) -> Result<Self, DomainError> {
        let mut cursor_key = [0u8; 32];
        system_random(&mut cursor_key)?;

        let header = serde_json::to_vec(&json!({
            "type": "session",
            "version": 3,
            "id": session.as_str(),
            "cwd": cwd,
            "timestamp": timestamp,
            // The caller may select another catalog model before this chat is
            // reopened. Keep the resolved local-model settings with the
            // journal so a resumed conversation cannot silently switch model.
            "ag1wRuntime": {
                "model": configuration.model.clone(),
                "systemPrompt": configuration.system_prompt.clone(),
            },
            // Context artifacts are owned by this session's origin. Persist
            // it so a restart resumes the same owner instead of guessing.
            "ag1wOrigin": origin,
        }))
        .map_err(|_| DomainError::InvalidInput("Windows session header".into()))?;

        store.bootstrap_session(SessionBootstrap {
            command: &format!("bootstrap-{}", session.as_str()),
            session: session.as_str(),
            origin,
            cursor_key: &cursor_key,
            seed: SessionSeed::Fresh {
                header: &header,
                records: &[],
            },
        })?;

        Self::resume(store, session, origin, epoch, configuration)
    }

    /// Resume only an already-created session. A requested missing session is
    /// an error rather than an implicit empty conversation.
    pub fn resume(
        store: Arc<Store>,
        session: SessionId,
        origin: &str,
        epoch: u64,
        mut configuration: Configuration,
    ) -> Result<Self, DomainError> {
        let journal = store.journal_snapshot(&session)?;
        // Context artifacts retain their own persisted origin. File-change
        // snapshots deliberately use the session ID, matching AG1's existing
        // change-review reader, so make that idempotent origin available on
        // both a new session and a resumed older one.
        store.ensure_output_origin(session.as_str(), session.as_str())?;
        let header: Value = serde_json::from_slice(&journal.header)
            .map_err(|_| DomainError::InvalidInput("Windows session header".into()))?;
        let cwd = header["cwd"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("Windows session directory".into()))?
            .to_owned();

        // Older Windows sessions did not retain these fields, so they continue
        // to use the caller's configuration. New sessions always resume the
        // exact local model and system prompt they were created with.
        if let Some(saved) = header.get("ag1wRuntime") {
            let model = saved["model"]
                .as_object()
                .ok_or_else(|| DomainError::InvalidInput("Windows session model".into()))?;
            let prompt = saved["systemPrompt"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("Windows session prompt".into()))?;
            configuration.model = Value::Object(model.clone());
            configuration.system_prompt = prompt.into();
        }
        let state = SessionRuntime::resume(session.clone(), epoch, &journal, configuration)?;
        Ok(Self {
            state: Mutex::new(state),
            store,
            session,
            origin: origin.into(),
            cwd,
            web: Mutex::new(None),
        })
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session
    }

    /// This is a journal header value, not the process-wide current directory.
    /// The Windows host revalidates it as a Workspace before touching files.
    pub fn working_directory(&self) -> &str {
        &self.cwd
    }

    pub fn journal_revision(&self) -> Result<u64, DomainError> {
        Ok(self.state()?.journal_revision())
    }

    pub fn records(&self) -> Result<Arc<Vec<Value>>, DomainError> {
        Ok(self.state()?.branch_snapshot())
    }

    /// Return every durable record in this session, including entries outside
    /// the currently selected branch. The desktop uses this only for its
    /// history inspector; provider context still comes from `records()`.
    pub fn history_records(&self) -> Result<Vec<Value>, DomainError> {
        let journal = self.store.journal_snapshot(&self.session)?;
        journal
            .entries
            .into_iter()
            .map(|entry| {
                serde_json::from_slice(&entry.normalized)
                    .map_err(|_| DomainError::InvalidInput("Windows journal record".into()))
            })
            .collect()
    }

    pub fn configuration(&self) -> Result<Configuration, DomainError> {
        Ok(self.state()?.configuration().clone())
    }

    /// Create a separate durable session whose branch ends immediately before
    /// one existing user message. The new session gets a fresh artifact origin
    /// while the storage layer copies only the parent's authorized evidence
    /// closure, so a fork neither shares mutable runtime state nor loses the
    /// evidence needed to explain its inherited context.
    pub fn fork_before_user_message(
        &self,
        child: SessionId,
        child_origin: &str,
        timestamp: &str,
        entry: &str,
    ) -> Result<Self, DomainError> {
        let records = self.history_records()?;
        let selected = records
            .iter()
            .find(|record| record["id"].as_str() == Some(entry))
            .ok_or_else(|| DomainError::InvalidInput("fork journal entry".into()))?;
        if selected["type"] != "message" || selected["message"]["role"] != "user" {
            return Err(DomainError::InvalidInput("fork user message".into()));
        }
        let leaf = selected["parentId"].as_str().map(str::to_owned);
        let (revision, configuration) = {
            let state = self.state()?;
            (state.journal_revision(), state.configuration().clone())
        };
        let mut cursor_key = [0u8; 32];
        system_random(&mut cursor_key)?;
        let command = format!("fork-{}", child.as_str());
        self.store.bootstrap_session(SessionBootstrap {
            command: &command,
            session: child.as_str(),
            origin: child_origin,
            cursor_key: &cursor_key,
            seed: SessionSeed::RuntimeFork {
                parent: self.session.as_str(),
                expected_revision: revision,
                leaf: leaf.as_deref(),
                timestamp,
            },
        })?;
        Self::resume(self.store.clone(), child, child_origin, 1, configuration)
    }

    /// Select an existing point in this session's stored tree. This changes
    /// only the active branch pointer; records after the selected point remain
    /// durable and available to the history inspector.
    pub fn select_history_entry(&self, command: &str, target: &str) -> Result<Commit, DomainError> {
        let mut state = self.state()?;
        let token = state.reserve_compaction()?;
        let capture = match state.capture_compaction(&token) {
            Ok(capture) => capture,
            Err(error) => {
                let _ = state.abandon_compaction(&token);
                return Err(error);
            }
        };
        let journal = match self.store.journal_snapshot(&self.session) {
            Ok(journal) => journal,
            Err(error) => {
                let _ = state.abandon_compaction(&token);
                return Err(error);
            }
        };
        let action = match state.navigate_tree(&capture, &journal, target, command, Vec::new()) {
            Ok(action) => action,
            Err(error) => {
                let _ = state.abandon_compaction(&token);
                return Err(error);
            }
        };
        let receipt = action.execute(self.store.as_ref());
        state.acknowledge(receipt)?;
        Ok(Commit {
            revision: state.journal_revision(),
            sequence: state.committed_sequence(),
        })
    }

    /// Move to an earlier branch point and retain a model-written account of
    /// the abandoned work. This deliberately uses the same isolated summary
    /// request as AG1's Linux session path: a summary is evidence, not a UI
    /// convenience string made up by the desktop host.
    pub async fn select_history_entry_with_summary(
        &self,
        command: &str,
        target: &str,
        timestamp: &str,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        mut observe: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<Commit, DomainError> {
        OperationId::new(command)?;
        OperationId::new(target)?;

        // Reserving compaction fences new turns. We release the session mutex
        // before the slow provider call, but the capture below prevents a late
        // summary from overwriting a newer journal revision.
        let (token, capture) = {
            let mut state = self.state()?;
            let token = state.reserve_compaction()?;
            let capture = match state.capture_compaction(&token) {
                Ok(capture) => capture,
                Err(error) => {
                    let _ = state.abandon_compaction(&token);
                    return Err(error);
                }
            };
            (token, capture)
        };

        let result = async {
            let journal = self.store.journal_snapshot(&self.session)?;
            let navigation = tree::prepare(&journal, target)?;
            let summary = self
                .summarize_abandoned_branch(
                    &capture,
                    &navigation.entries,
                    command,
                    timestamp,
                    scheduler,
                    http,
                    cancel,
                    &mut observe,
                )
                .await?;
            let trajectory = self.retain_branch_trajectory(
                command,
                timestamp,
                &journal,
                target,
                &navigation.entries,
            )?;

            let record = json!({
                "type":"branch_summary",
                "id":format!("{command}-summary"),
                "timestamp":timestamp,
                "fromId":journal.leaf,
                "summary":summary["summary"],
                "fromHook":false,
                "details":summary.get("details").cloned().unwrap_or(Value::Null),
                "nativeTrajectory":trajectory,
            });

            // The branch action appends the summary and changes the selected
            // leaf in a single checked write. If anything changed while the
            // model was working, the runtime returns Conflict instead.
            let mut state = self.state()?;
            let action = state.navigate_tree(&capture, &journal, target, command, vec![record])?;
            let receipt = action.execute(self.store.as_ref());
            state.acknowledge(receipt)?;
            Ok(Commit {
                revision: state.journal_revision(),
                sequence: state.committed_sequence(),
            })
        }
        .await;

        if result.is_err() {
            // A failed or cancelled model call must not leave this chat fenced
            // forever. The journal itself has not moved until acknowledge.
            if let Ok(mut state) = self.state() {
                let _ = state.abandon_compaction(&token);
            }
        }
        result
    }

    async fn summarize_abandoned_branch(
        &self,
        capture: &CompactionCapture,
        entries: &[Value],
        command: &str,
        timestamp: &str,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        observe: &mut impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<Value, DomainError> {
        let mut configuration = capture.configuration().clone();
        let reserve_tokens = 2_048;
        let budget = configuration.context_limit().saturating_sub(reserve_tokens);
        let prepared = tree::summary(entries, budget.min(i64::MAX as u64) as i64)?;
        if prepared.messages.is_empty() {
            return Ok(prepared.finish("No abandoned branch content needed summarizing."));
        }

        let isolated = prepared.request(
            None,
            false,
            self.session.as_str(),
            configuration.model["maxTokens"].as_u64().unwrap_or(0),
        )?;
        let messages = isolated.context["messages"]
            .as_array()
            .cloned()
            .ok_or_else(|| DomainError::InvalidInput("branch summary messages".into()))?;
        configuration.tools.clear();
        configuration.thinking = "off".into();
        configuration.system_prompt = isolated.context["systemPrompt"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("branch summary prompt".into()))?
            .into();

        // Do the admission check before talking to the provider. Isolated
        // summaries must fit inside the same selected working context as a
        // normal chat turn; they never get a hidden larger window.
        let estimated = configuration.fixed_tokens()
            + messages
                .iter()
                .map(ag1_core::context::estimate_tokens)
                .sum::<u64>();
        if estimated.saturating_mul(11).div_ceil(10) > configuration.context_limit() {
            return Err(DomainError::LimitExceeded(
                "branch summary context admission margin".into(),
            ));
        }

        let endpoint = Self::local_endpoint(&configuration)?;
        let family: ApiFamily = serde_json::from_value(configuration.model["api"].clone())
            .map_err(|_| DomainError::InvalidInput("local provider family".into()))?;
        let context = json!({
            "systemPrompt":configuration.system_prompt,
            "messages":messages,
            "tools":[],
        });
        let options = session_options(&configuration.model, &configuration.thinking, None)?;
        let credential = SecretBytes::new(b"ag1-local".to_vec());
        let operation = format!("{command}-summary");
        let request = generation_http_request(
            &RequestContext {
                model: &configuration.model,
                context: &context,
                options: &options,
                anthropic_oauth: false,
                google_project: None,
                request_id: &operation,
            },
            HttpCredential::Bearer(&credential),
            32 * 1024 * 1024,
        )?;
        let generation = GenerationRequest {
            request_id: operation.clone(),
            tool_names: Vec::new(),
            family,
            request,
            limits: GenerationLimits {
                max_output_bytes: 32 * 1024 * 1024,
                prefill_ms: 120_000,
                idle_ms: 120_000,
                overall_ms: 120_000,
            },
        };

        if let Err(error) = scheduler.register(&endpoint, EndpointKind::Local) {
            return Err(error);
        }
        let ticket = scheduler.submit(&endpoint, OperationId::new(operation.clone())?)?;
        let permit = loop {
            if cancel.cancelled() {
                scheduler.cancel(&ticket)?;
                return Err(DomainError::Cancelled);
            }
            match scheduler.start(&ticket)? {
                Some(permit) => break permit,
                None => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        };

        let outcome =
            request_generation(http, generation, permit, cancel, |event| observe(event)).await;
        let (raw_snapshot, raw_error) = match &outcome {
            Ok(snapshot) => (Some(snapshot), None),
            Err(failure) => (failure.partial.as_ref(), Some(&failure.error)),
        };
        let output = self.retain_provider_output(&operation, timestamp, raw_snapshot, raw_error)?;
        let snapshot = outcome.map_err(|failure| failure.error)?;
        if matches!(snapshot.stop_reason, Some(StopReason::Error)) {
            return Err(DomainError::Unavailable(
                snapshot
                    .error
                    .unwrap_or_else(|| "local branch summary failed".into()),
            ));
        }
        if matches!(snapshot.stop_reason, Some(StopReason::Aborted)) {
            return Err(DomainError::Cancelled);
        }
        let text = snapshot
            .content
            .iter()
            .filter(|block| block.kind == BlockKind::Text)
            .map(|block| block.text.as_str())
            .collect::<String>();
        if text.trim().is_empty() {
            return Err(DomainError::Unavailable(
                "local branch summary returned no text".into(),
            ));
        }
        let mut summary = prepared.finish(&text);
        summary["details"]["summaryOutput"] = serde_json::to_value(output)
            .map_err(|_| DomainError::InvalidInput("branch summary output artifact".into()))?;
        Ok(summary)
    }

    fn retain_branch_trajectory(
        &self,
        command: &str,
        timestamp: &str,
        journal: &ag1_core::storage::StoredJournal,
        target: &str,
        entries: &[Value],
    ) -> Result<ContextArtifact, DomainError> {
        let raw_entries = journal
            .entries
            .iter()
            .map(|entry| {
                std::str::from_utf8(&entry.raw)
                    .map(str::to_owned)
                    .map_err(|_| DomainError::InvalidInput("Windows journal UTF-8".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut artifacts = ContextStore::new(
            self.store.as_ref(),
            self.session.clone(),
            &self.origin,
            &format!("{command}-trajectory"),
            timestamp,
            Features::default(),
        )?;
        artifacts.put_evidence(
            &json!({
                "version":1,
                "kind":"branch-trajectory",
                "journalRevision":journal.revision,
                "oldLeafId":journal.leaf,
                "rawEntries":raw_entries,
                "targetId":target,
                "entries":entries,
            })
            .to_string(),
            "runtime-branch-trajectory",
            &json!({"type":"session","name":"tree-navigation"}),
            &[],
            None,
        )
    }

    /// Append one already-validated AG1 history message and wait for its SQLite
    /// receipt before updating the live runtime state.
    pub fn append_message(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        message: Value,
    ) -> Result<Commit, DomainError> {
        let mut state = self.state()?;
        let action = state.append_message(command, id, timestamp, message)?;
        Self::commit(&mut state, action, self.store.as_ref())
    }

    /// Persist a selected context limit through the same journal path as AG1's
    /// existing client. A later resume reads this record back into the compiler.
    pub fn select_context(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        tokens: u64,
    ) -> Result<Commit, DomainError> {
        let mut state = self.state()?;
        let action = state.select_context(command, id, timestamp, tokens)?;
        Self::commit(&mut state, action, self.store.as_ref())
    }

    /// Apply AG1's validated task reducer, then journal the next task state as
    /// a normal durable record. The context compiler reads this record on the
    /// next turn, so a desktop task is not merely a UI note.
    pub fn update_task(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        args: &Value,
    ) -> Result<Value, DomainError> {
        let mut state = self.state()?;
        let plan = ag1_core::tools::task_state::prepare(state.branch(), args, timestamp, id)?;
        if let Some(task) = plan.next {
            let action = state.append_task_state(command, id, timestamp, task)?;
            Self::commit(&mut state, action, self.store.as_ref())?;
        }
        Ok(plan.result)
    }

    /// Return the latest durable plan revision for each plan thread. Plans are
    /// workflow state in the journal, not UI objects reconstructed from text.
    pub fn plan_snapshot(&self, limit: usize) -> Result<Vec<Plan>, DomainError> {
        if !(1..=128).contains(&limit) {
            return Err(DomainError::InvalidInput("plan history limit".into()));
        }
        Ok(planning::recent_plans(self.state()?.branch(), limit))
    }

    /// Run one read-only planning turn, then convert its structured markdown
    /// into AG1's validated plan record. The plan is stored separately from
    /// the assistant message so later accept/reject/step updates are checked
    /// workflow transitions rather than text edits.
    pub async fn draft_plan_local(
        &self,
        input: LocalTurnInput,
        goal: &str,
        previous: Option<PlanRef>,
        feedback: Option<&str>,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        observe: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<LocalPlanResult, DomainError> {
        if goal.trim().is_empty() || goal.len() > 1024 * 1024 {
            return Err(DomainError::InvalidInput("planning goal".into()));
        }
        if input.system_addendum.is_none() {
            return Err(DomainError::InvalidInput("planning addendum".into()));
        }
        let prior = if let Some(reference) = previous.as_ref() {
            Some(
                planning::resolve(self.state()?.branch(), reference)
                    .ok_or(DomainError::NotFound)?,
            )
        } else {
            None
        };

        let command = input.command.clone();
        let timestamp = input.timestamp.clone();
        let turn = self
            .run_local_turn(input, scheduler, http, cancel, observe)
            .await?;
        let text = Self::assistant_text(&turn.message);
        // The turn already used `input.command` for its user record. Publishing
        // the plan is a second journal transaction, so it needs AG1's stable
        // derived command instead of replaying the turn command.
        let plan_command = format!("plan-{}", ag1_storage::digest(command.as_bytes()));
        let plan_id = plan_command.clone();
        let plan =
            Plan::from_markdown(&plan_id, goal, &text, prior.as_ref(), feedback, &timestamp)?.or(
                Plan::fallback(&plan_id, goal, prior.as_ref(), feedback, &text, &timestamp)?,
            );

        if let Some(plan) = plan.as_ref() {
            let mut state = self.state()?;
            let mut selection = planning::selection(state.branch()).unwrap_or(Selection {
                version: 1,
                planning: None,
                accepted: None,
                executing: None,
                updated_at: timestamp.clone(),
            });
            selection.planning = Some(plan.reference());
            selection.accepted = None;
            selection.executing = None;
            selection.updated_at = timestamp.clone();
            let id = format!("{plan_command}-record");
            let action = state.update_plan(
                &plan_command,
                &id,
                &timestamp,
                Some(plan.clone()),
                selection,
            )?;
            Self::commit(&mut state, action, self.store.as_ref())?;
        }
        Ok(LocalPlanResult { turn, plan })
    }

    /// Apply one non-generative plan transition using AG1's same validated
    /// state machine. It cannot grant tool permissions or run a plan.
    pub fn change_plan(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        action: &str,
        args: &Value,
    ) -> Result<Plan, DomainError> {
        let plan_id = args["planId"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| DomainError::InvalidInput("plan identifier".into()))?;
        let revision = args["revision"]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or_else(|| DomainError::InvalidInput("plan revision".into()))?;
        let reference = PlanRef {
            plan_id: plan_id.into(),
            revision,
        };
        let mut state = self.state()?;
        let plan = planning::resolve(state.branch(), &reference).ok_or(DomainError::NotFound)?;
        let mut selection = planning::selection(state.branch()).unwrap_or(Selection {
            version: 1,
            planning: None,
            accepted: None,
            executing: None,
            updated_at: timestamp.into(),
        });
        let updated = match action {
            "accept" => {
                let updated = plan.transition(Status::Accepted, timestamp)?;
                selection.planning = None;
                selection.executing = None;
                selection.accepted = Some(reference);
                updated
            }
            "reject" | "clear" => {
                let next = if action == "reject" {
                    Status::Rejected
                } else {
                    Status::Discarded
                };
                let mut updated = plan.transition(next, timestamp)?;
                if action == "reject" {
                    updated.rejected_reason = args["reason"]
                        .as_str()
                        .filter(|reason| !reason.trim().is_empty())
                        .map(|reason| reason.trim().into());
                }
                for selected in [
                    &mut selection.planning,
                    &mut selection.accepted,
                    &mut selection.executing,
                ] {
                    if selected.as_ref() == Some(&reference) {
                        *selected = None;
                    }
                }
                updated
            }
            "complete_step" | "reopen_step" => {
                let step = args["stepIndex"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| DomainError::InvalidInput("plan step".into()))?;
                let updated = plan.mark_step(step, action == "complete_step", timestamp)?;
                selection.planning = None;
                selection.executing = None;
                selection.accepted = Some(updated.reference());
                updated
            }
            "answer" => {
                let question = args["questionIndex"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| DomainError::InvalidInput("plan question".into()))?;
                let answer = args["answer"]
                    .as_str()
                    .ok_or_else(|| DomainError::InvalidInput("plan answer".into()))?;
                plan.answer(question, answer, timestamp)?
            }
            "execute" => {
                return Err(DomainError::Unavailable(
                    "native plan execution needs an explicit follow-up request".into(),
                ));
            }
            _ => return Err(DomainError::InvalidInput("plan action".into())),
        };
        selection.updated_at = timestamp.into();
        let action = state.update_plan(command, id, timestamp, Some(updated.clone()), selection)?;
        Self::commit(&mut state, action, self.store.as_ref())?;
        Ok(updated)
    }

    /// Compact the active durable branch through the selected local model.
    ///
    /// This keeps AG1's normal contract: capture and fence the journal first,
    /// preserve the raw input as an artifact, ask the model for a summary, and
    /// append that summary only if the original capture is still current.
    /// Nothing in this method mutates an in-memory chat transcript.
    pub async fn compact_local(
        &self,
        command: &str,
        timestamp: &str,
        focus: Option<&str>,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        mut observe: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<Option<LocalCompactionResult>, DomainError> {
        OperationId::new(command)?;

        // The reservation makes a slow summary safe. New turns cannot slip
        // between this capture and the checked append at the end.
        let (token, capture) = {
            let mut state = self.state()?;
            let token = state.reserve_compaction()?;
            let capture = match state.capture_compaction(&token) {
                Ok(capture) => capture,
                Err(error) => {
                    let _ = state.abandon_compaction(&token);
                    return Err(error);
                }
            };
            (token, capture)
        };

        let result = async {
            let branch = capture.branch().to_vec();
            let messages = compaction::context_messages(&branch);
            let revision = capture.journal_revision();
            let settings = Self::local_compaction_settings(capture.configuration().context_limit());

            // Keep the raw pre-summary request and the exact cut input. The
            // compaction text is an index; these artifacts remain the source
            // when a later action needs primary evidence.
            let mut artifacts = ContextStore::new(
                self.store.as_ref(),
                self.session.clone(),
                &self.origin,
                command,
                timestamp,
                Features::default(),
            )?;
            let original = artifacts.project(&messages, revision, |messages, _| {
                Ok(Compilation {
                    messages: messages.to_vec(),
                    stats: json!({}),
                })
            })?;
            let Some(preparation) = compaction::prepare_full(&branch, &settings)? else {
                self.state()?.abandon_compaction(&token)?;
                return Ok(None);
            };
            let trajectory = artifacts.put_evidence(
                &json!({
                    "version": 1,
                    "kind": "compaction-trajectory",
                    "createdAt": timestamp,
                    "tokensBefore": preparation["tokensBefore"],
                    "firstKeptEntryId": preparation["firstKeptEntryId"],
                    "isSplitTurn": preparation["isSplitTurn"],
                    "previousSummaryIncluded": preparation["previousSummary"].as_str().is_some_and(|text| !text.is_empty()),
                    "messagesToSummarize": preparation["messagesToSummarize"],
                    "turnPrefixMessages": preparation["turnPrefixMessages"],
                })
                .to_string(),
                "compaction-trajectory",
                &json!({"type":"session","name":"semantic-compaction"}),
                &[],
                None,
            )?;

            let split = preparation["isSplitTurn"] == true
                && preparation["turnPrefixMessages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty());
            let mut summary_outputs = Vec::new();
            let history = if split
                && preparation["messagesToSummarize"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            {
                "No prior history.".to_owned()
            } else {
                let (text, output) = self.summarize_compaction_part(
                    &capture,
                    &preparation,
                    focus,
                    false,
                    command,
                    timestamp,
                    scheduler,
                    http,
                    cancel,
                    &mut observe,
                )
                .await?;
                summary_outputs.push(output);
                text
            };
            let prefix = if split {
                let (text, output) = self.summarize_compaction_part(
                        &capture,
                        &preparation,
                        focus,
                        true,
                        command,
                        timestamp,
                        scheduler,
                        http,
                        cancel,
                        &mut observe,
                    )
                    .await?;
                summary_outputs.push(output);
                Some(text)
            } else {
                None
            };

            let mut summary = compaction::finish_summary(
                &preparation,
                &history,
                prefix.as_deref(),
                None,
            )?;
            compaction::attach_provenance(&mut summary, &trajectory)?;
            summary["details"]["originalJournal"] = json!(original.original_journal);
            summary["details"]["originalRequest"] = json!(original.original_request);
            summary["details"]["summaryOutputs"] = json!(summary_outputs);

            // `append_compaction` rechecks the capture. If any new journal
            // record appeared while the summary was running, this fails rather
            // than applying a summary to the wrong branch.
            let id = format!("{command}-summary");
            let mut state = self.state()?;
            let action = state.append_compaction(&capture, command, &id, timestamp, summary.clone())?;
            let commit = Self::commit(&mut state, action, self.store.as_ref())?;
            let context_epoch = state.stamp().epoch();
            Ok(Some(LocalCompactionResult {
                result: summary,
                context_epoch,
                journal_revision: commit.revision,
            }))
        }
        .await;

        if result.is_err() {
            // A cancelled or failed summary did not write a journal entry, so
            // release its fence and leave the conversation untouched.
            if let Ok(mut state) = self.state() {
                let _ = state.abandon_compaction(&token);
            }
        }
        result
    }

    /// Build the real context-virtualized request projection. This retains the
    /// immutable journal and request inputs before any compiler reduction so a
    /// later UI can inspect or reproduce the exact context decision.
    pub fn compile_context(
        &self,
        operation: &str,
        timestamp: &str,
    ) -> Result<CompiledContext, DomainError> {
        let (messages, options, revision) = {
            let state = self.state()?;
            if state.pending_commit() || !state.persistence_healthy() {
                return Err(DomainError::DurabilityUnknown);
            }

            let configuration = state.configuration();
            let maximum = configuration.model["contextWindow"]
                .as_u64()
                .ok_or_else(|| DomainError::InvalidInput("model context window".into()))?;
            let mut options = configuration.compiler.clone();
            options.context_window = if options.context_window == 0 {
                maximum
            } else {
                options.context_window.min(maximum)
            };
            options.fixed_tokens = Some(configuration.fixed_tokens());
            (
                state.messages().values().cloned().collect::<Vec<_>>(),
                options,
                state.journal_revision(),
            )
        };

        let mut context = ContextStore::new(
            self.store.as_ref(),
            self.session.clone(),
            &self.origin,
            operation,
            timestamp,
            Features::default(),
        )?;
        let compiled = context.compile(&messages, &options, revision)?;
        Ok(CompiledContext {
            projection: compiled.projection,
            original_journal: compiled.original_journal,
            original_request: compiled.original_request,
            revision: compiled.revision,
        })
    }

    /// Run one durable user/assistant exchange without project tools.
    pub async fn run_local_turn(
        &self,
        input: LocalTurnInput,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        observe: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<LocalTurnResult, DomainError> {
        self.run_local_turn_with_tools(input, scheduler, http, cancel, None, None, observe)
            .await
    }

    /// Run a durable local turn, executing only the explicitly supplied
    /// project tools. The caller cannot smuggle arbitrary tool definitions
    /// through a model response because this method owns the allow-list.
    pub async fn run_local_turn_with_tools(
        &self,
        input: LocalTurnInput,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        tools: Option<&LocalProjectTools>,
        mcp: Option<McpToolSurface>,
        mut observe: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<LocalTurnResult, DomainError> {
        Self::validate_local_turn(&input)?;
        // Reject an unsupported configuration before journaling a user turn
        // which this Windows runner cannot actually send to a model.
        Self::local_endpoint(&self.configuration()?)?;

        let user = self.append_message(
            &input.command,
            &input.user_id,
            &input.timestamp,
            input.message,
        )?;

        // Each completed tool round is committed before the next model request.
        // That is what lets AG1's context compiler retain a real evidence trail
        // instead of an in-memory transcript of uncommitted tool output.
        let mut round = 0u8;
        let mut tool_calls = 0usize;
        // Keep failed calls for this one user turn. A model sometimes retries
        // the exact same request after an error, which only wastes time and
        // can make a real provider block the chat for longer.
        let mut failed_calls: Vec<(String, Value)> = Vec::new();
        // Web research is available in ordinary chats, even before a project
        // is attached. Project tools are added only from the prompt's frozen
        // folder snapshot. Plan drafts deliberately receive neither surface.
        let request_tools = if input.system_addendum.is_some() {
            None
        } else {
            let mut definitions = LocalProjectTools::web_definitions();
            if let Some(tools) = tools {
                definitions.extend(tools.request_definitions());
            }
            if let Some(mcp) = &mcp
                && mcp.manager.has_callable_tools(&mcp.scope)?
            {
                definitions.extend(McpToolSurface::request_definitions());
            }
            Some(definitions)
        };
        loop {
            // Preparation captures one immutable journal/configuration view. The
            // context compiler can now run without holding the session mutex.
            let mut preparation = self.state()?.prepare_request()?;
            if let Some(addendum) = input.system_addendum.as_deref() {
                // Plan mode keeps the same conversation and model, but this
                // one request is read-only. Its tools and instructions are
                // changed on the captured preparation only, never globally.
                let system = format!(
                    "{}\n\n{addendum}",
                    preparation.configuration().system_prompt
                );
                if let Err(error) = preparation
                    .set_tools(Vec::new())
                    .and_then(|_| preparation.set_system_prompt(system))
                {
                    self.discard_preparation(preparation, error.clone());
                    return Err(error);
                }
            }
            let assistant_id = if round == 0 {
                input.assistant_id.clone()
            } else {
                format!("{}-{round}", input.assistant_id)
            };
            let operation = format!("{assistant_id}.local");
            let compiled = self.compile_preparation(&preparation, &operation, &input.timestamp);
            let compiled = match compiled {
                Ok(compiled) => compiled,
                Err(error) => {
                    let _ = self.state()?.admit_request(preparation, Err(error.clone()));
                    return Err(error);
                }
            };

            let (endpoint, generation) = match Self::build_local_generation(
                &preparation,
                compiled.projection.clone(),
                &operation,
                request_tools.as_deref(),
            ) {
                Ok(value) => value,
                Err(error) => {
                    let _ = self.state()?.admit_request(preparation, Err(error.clone()));
                    return Err(error);
                }
            };

            // A local endpoint gets exactly one slot. We acquire it before session
            // admission, so a queued request cannot become a stale model request.
            if let Err(error) = scheduler.register(&endpoint, EndpointKind::Local) {
                self.discard_preparation(preparation, error.clone());
                return Err(error);
            }
            let ticket = match scheduler.submit(&endpoint, OperationId::new(operation.clone())?) {
                Ok(ticket) => ticket,
                Err(error) => {
                    self.discard_preparation(preparation, error.clone());
                    return Err(error);
                }
            };
            let permit = loop {
                if cancel.cancelled() {
                    match scheduler.cancel(&ticket) {
                        Ok(()) => {
                            self.discard_preparation(preparation, DomainError::Cancelled);
                            return Err(DomainError::Cancelled);
                        }
                        Err(error) => {
                            self.discard_preparation(preparation, error.clone());
                            return Err(error);
                        }
                    }
                }
                match scheduler.start(&ticket) {
                    Ok(Some(permit)) => break permit,
                    Ok(None) => {}
                    Err(error) => {
                        self.discard_preparation(preparation, error.clone());
                        return Err(error);
                    }
                }
                // async/await yields the worker while another local generation owns
                // the model. It avoids blocking the desktop event loop.
                tokio::time::sleep(Duration::from_millis(10)).await;
            };

            let admitted = self
                .state()?
                .admit_request(preparation, Ok(compiled.projection.clone()));
            let generation_state = match admitted {
                Ok(generation_state) => generation_state,
                Err(error) => {
                    permit.terminal()?;
                    return Err(error);
                }
            };

            let outcome =
                request_generation(http, generation, permit, cancel, |event| observe(event)).await;
            let (raw_snapshot, raw_error) = match &outcome {
                Ok(snapshot) => (Some(snapshot), None),
                Err(failure) => (failure.partial.as_ref(), Some(&failure.error)),
            };

            // Preserve what the provider actually returned before translating a
            // failure or partial stream into an assistant journal message.
            let output =
                self.retain_provider_output(&operation, &input.timestamp, raw_snapshot, raw_error);
            let output = match output {
                Ok(output) => output,
                Err(error) => {
                    let mut state = self.state()?;
                    state.fail_persistence();
                    state.generation_terminal(&generation_state)?;
                    return Err(error);
                }
            };

            let (snapshot, provider_error) = match outcome {
                Ok(snapshot) => {
                    let error = match snapshot.stop_reason {
                        Some(StopReason::Error) => Some(DomainError::Unavailable(
                            snapshot
                                .error
                                .clone()
                                .unwrap_or_else(|| "local provider generation failed".into()),
                        )),
                        Some(StopReason::Aborted) => Some(DomainError::Cancelled),
                        _ => None,
                    };
                    (snapshot, error)
                }
                Err(failure) => {
                    let Some(mut partial) = failure.partial else {
                        self.state()?.generation_terminal(&generation_state)?;
                        return Err(failure.error);
                    };
                    partial.stop_reason = Some(if failure.error == DomainError::Cancelled {
                        StopReason::Aborted
                    } else {
                        StopReason::Error
                    });
                    partial.error = Some(format!("{:?}", failure.error));
                    (partial, Some(failure.error))
                }
            };

            let mut message = match assistant_message(
                &snapshot,
                &generation_state.configuration().model,
                input.timestamp_ms,
            ) {
                Ok(message) => message,
                Err(error) => {
                    let mut state = self.state()?;
                    state.fail_persistence();
                    state.generation_terminal(&generation_state)?;
                    return Err(error);
                }
            };
            message["nativeOutputArtifact"] = serde_json::to_value(&output)
                .map_err(|_| DomainError::InvalidInput("provider output artifact".into()))?;

            // The transport is settled, but the session boundary stays closed until
            // the assistant record is durably acknowledged.
            let assistant = {
                let mut state = self.state()?;
                let action = state.append_message(
                    &assistant_id,
                    &assistant_id,
                    &input.timestamp,
                    message.clone(),
                );
                state.generation_terminal(&generation_state)?;
                let action = match action {
                    Ok(action) => action,
                    Err(error) => {
                        state.fail_persistence();
                        return Err(error);
                    }
                };
                Self::commit(&mut state, action, self.store.as_ref())?
            };

            if let Some(error) = provider_error {
                return Err(error);
            }

            let calls = Self::project_tool_calls(&message)?;
            if calls.is_empty() {
                return Ok(LocalTurnResult {
                    user,
                    assistant,
                    message,
                    original_journal: compiled.original_journal,
                    original_request: compiled.original_request,
                    output,
                    estimated_input_tokens: generation_state.estimated_input_tokens(),
                });
            }
            if round >= 7 {
                return Err(DomainError::LimitExceeded(
                    "local project tool rounds".into(),
                ));
            }
            if calls.len() > MAX_LOCAL_TOOL_CALLS.saturating_sub(tool_calls) {
                return Err(DomainError::LimitExceeded(
                    "local project tool calls".into(),
                ));
            }
            let change = LocalChangeContext {
                store: self.store.as_ref(),
                session: self.session.as_str(),
                operation: &assistant_id,
                timestamp_ms: input.timestamp_ms,
            };
            for (index, call) in calls.iter().enumerate() {
                let name = call["name"].as_str().unwrap_or_default();
                let repeated_failure = repeats_failed_call(&failed_calls, call);
                let result = if repeated_failure {
                    repeated_tool_error_result(call)
                } else {
                    match call["name"].as_str() {
                        Some("webSearch") | Some("webFetch") => {
                            self.execute_web_tool(
                                call,
                                &format!("{assistant_id}-web-{round}-{index}"),
                                &input.timestamp,
                                input.timestamp_ms,
                                cancel,
                            )
                            .await
                        }
                        Some("mcpSearch") => self.execute_mcp_search(mcp.as_ref(), call),
                        Some("mcpCall") => self.execute_mcp_call(mcp.as_ref(), call, cancel).await,
                        _ => tools
                            .map(|tools| tools.execute_with_changes(call, Some(&change)))
                            .unwrap_or_else(|| unavailable_local_tool_result(call)),
                    }
                };
                if result["isError"] == true {
                    failed_calls.push((name.to_owned(), call["arguments"].clone()));
                }
                let id = format!("{}-tool-{round}-{index}", input.assistant_id);
                self.append_message(&id, &id, &input.timestamp, result)?;
            }
            tool_calls += calls.len();
            round += 1;
        }
    }

    fn execute_mcp_search(&self, mcp: Option<&McpToolSurface>, call: &Value) -> Value {
        let Some(mcp) = mcp else {
            return mcp_tool_error_result(
                call,
                DomainError::Unavailable(
                    "No reviewed MCP tools are connected for this chat.".into(),
                ),
            );
        };
        let query = call["arguments"]["query"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("MCP tool search query".into()));
        let limit = call["arguments"]["maxResults"]
            .as_u64()
            .map(|value| value as usize)
            .unwrap_or(4);
        match query.and_then(|query| mcp.manager.search_tools(&mcp.scope, query, limit)) {
            Ok(result) => json!({
                "role":"toolResult",
                "toolCallId":call["id"],
                "toolName":call["name"],
                "content":[{"type":"text","text":bounded_mcp_text(&result)}],
                "details":{"mcp":{"status":"search"}},
                "isError":false,
            }),
            Err(error) => mcp_tool_error_result(call, error),
        }
    }

    async fn execute_mcp_call(
        &self,
        mcp: Option<&McpToolSurface>,
        call: &Value,
        cancel: &mut CancelToken,
    ) -> Value {
        let Some(mcp) = mcp else {
            return mcp_tool_error_result(
                call,
                DomainError::Unavailable(
                    "No reviewed MCP tools are connected for this chat.".into(),
                ),
            );
        };
        let connection = match call["arguments"]["connection"].as_str() {
            Some(value) => value.to_owned(),
            None => {
                return mcp_tool_error_result(
                    call,
                    DomainError::InvalidInput("MCP connection".into()),
                );
            }
        };
        let name = match call["arguments"]["name"].as_str() {
            Some(value) => value.to_owned(),
            None => {
                return mcp_tool_error_result(
                    call,
                    DomainError::InvalidInput("MCP tool name".into()),
                );
            }
        };
        let arguments = match call["arguments"].get("arguments") {
            Some(Value::Object(value)) => Value::Object(value.clone()),
            _ => {
                return mcp_tool_error_result(
                    call,
                    DomainError::InvalidInput("MCP tool arguments".into()),
                );
            }
        };
        if cancel.cancelled() {
            return mcp_tool_error_result(call, DomainError::Cancelled);
        }

        // The MCP stdio process is deliberately blocking. Move that wait off
        // the async desktop worker, but keep Stop responsive by retiring the
        // owned connection if the chat cancellation token fires.
        let manager = mcp.manager.clone();
        let scope = mcp.scope.clone();
        let call_manager = manager.clone();
        let call_connection = connection.clone();
        let work = tokio::task::spawn_blocking(move || {
            call_manager.execute_tool(&call_connection, &scope, &name, arguments)
        });
        tokio::pin!(work);
        let outcome = tokio::select! {
            result = &mut work => result.map_err(|_| DomainError::Unavailable("MCP worker task".into())).and_then(|result| result),
            _ = cancel.wait() => {
                let _ = manager.disconnect(&connection);
                Err(DomainError::Cancelled)
            }
        };
        match outcome {
            Ok(result) => mcp_tool_result(call, result),
            Err(error) => mcp_tool_error_result(call, error),
        }
    }

    /// Run AG1's existing web services behind a fresh, turn-bound authority
    /// fence. The model sees only the two small web schemas; raw responses are
    /// retained as evidence and do not become unbounded chat context.
    async fn execute_web_tool(
        &self,
        call: &Value,
        operation: &str,
        timestamp: &str,
        timestamp_ms: u64,
        cancel: &mut CancelToken,
    ) -> Value {
        let result = async {
            let name = call["name"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("web tool name".into()))?;
            let call_id = call["id"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("web tool call".into()))?;
            let args = &call["arguments"];
            let (scope, profile) = {
                let state = self.state()?;
                let configuration = state.configuration();
                (
                    Scope {
                        workspace: self.cwd.clone(),
                        session: self.session.as_str().into(),
                        epoch: state.stamp().epoch(),
                        revision: state.journal_revision(),
                    },
                    research::profile(configuration.context_limit() as f64, 0.0),
                )
            };
            let (service, search) = self.web_services(&scope)?;
            let mut registry = Registry::builtins()?;
            let available = BTreeSet::from([name.to_owned()]);
            registry.activate(&[name.to_owned()], &available)?;
            let prepared = registry.prepare(operation, call_id, name, args, true)?;
            let mut authorizer = Authorizer::new(operation)?;
            let policy = Self::web_tool_policy();
            let target = if name == "webSearch" {
                search.target_revision().to_owned()
            } else {
                service.target_revision().to_owned()
            };
            let authorization = match authorizer.preflight(
                prepared,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &target,
                    now: timestamp_ms,
                    ttl: 60_000,
                    untrusted_extensions: false,
                },
            )? {
                Preflight::Ready(authorization) => authorization,
                Preflight::Rejected(_) => return Err(DomainError::PermissionDenied),
                // The policy currently marks public research as ready. Keep a
                // hard failure here if that contract changes rather than
                // manufacturing an approval from inside a model turn.
                Preflight::Approval(_) => return Err(DomainError::PermissionDenied),
            };
            let mut artifacts = ContextStore::new(
                self.store.as_ref(),
                self.session.clone(),
                &self.origin,
                operation,
                timestamp,
                Features::default(),
            )?;
            let execution = WebExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || timestamp_ms,
                deadline: tokio::time::Instant::now() + WEB_TOOL_TIMEOUT,
                cancel,
                profile,
            };
            let outcome = if name == "webSearch" {
                search
                    .execute(
                        authorization,
                        execution,
                        utc_year(timestamp_ms),
                        &mut artifacts,
                    )
                    .await?
            } else if name == "webFetch" {
                service
                    .execute(authorization, execution, &mut artifacts)
                    .await?
            } else {
                return Err(DomainError::InvalidInput("web tool name".into()));
            };
            Ok::<Value, DomainError>(web_tool_result(call, outcome.result))
        }
        .await;
        result.unwrap_or_else(|error| web_tool_error_result(call, error))
    }

    fn web_tool_policy() -> Policy {
        // The shared policy layer uses a portable POSIX-shaped path only as a
        // captured policy namespace. It is never used to open files here.
        // Passing a Windows drive path made every native web call fail before
        // it reached the search service.
        Policy {
            // Research is read-only, but Autonomous makes that explicit for
            // the policy engine instead of silently bypassing it.
            mode: ToolMode::Autonomous,
            cwd: "/ag1w/public-web".into(),
            planning_read_only_tools: None,
            trusted_tools: vec![],
            extension_tools: vec![],
        }
    }

    fn web_services(
        &self,
        scope: &Scope,
    ) -> Result<
        (
            Arc<WebService<SystemResolver>>,
            Arc<SearchService<SystemResolver>>,
        ),
        DomainError,
    > {
        let mut state = self
            .web
            .lock()
            .map_err(|_| DomainError::Unavailable("web service state".into()))?;
        let stale = state.as_ref().is_none_or(|web| {
            web.workspace != scope.workspace
                || web.session != scope.session
                || web.epoch != scope.epoch
        });
        if stale {
            let incarnation = format!("ag1w-web-{}", scope.epoch);
            let fetch = Arc::new(WebService::new(
                WebHttp::new(WebPolicy::default()),
                scope.clone(),
                &incarnation,
                BTreeMap::new(),
            )?);
            let search = Arc::new(SearchService::new(
                fetch.clone(),
                SearchConfig::default(),
                &format!("ag1w-search-{}", scope.epoch),
            )?);
            *state = Some(LocalWeb {
                workspace: scope.workspace.clone(),
                session: scope.session.clone(),
                epoch: scope.epoch,
                fetch,
                search,
            });
        }
        let web = state.as_ref().expect("web state initialized");
        Ok((web.fetch.clone(), web.search.clone()))
    }

    fn validate_local_turn(input: &LocalTurnInput) -> Result<(), DomainError> {
        OperationId::new(&input.command)?;
        OperationId::new(&input.user_id)?;
        OperationId::new(&input.assistant_id)?;
        // Later tool records add a short suffix to the supplied assistant ID.
        // Keep every derived journal operation inside AG1's identifier bound.
        if input.assistant_id.len() > 119 {
            return Err(DomainError::LimitExceeded(
                "local assistant identifier".into(),
            ));
        }
        if input.user_id == input.assistant_id
            || input.timestamp.is_empty()
            || input.timestamp.len() > 64
            || input
                .system_addendum
                .as_ref()
                .is_some_and(|value| value.len() > 2 * 1024 * 1024)
            || input.message["role"] != "user"
        {
            return Err(DomainError::InvalidInput("local turn input".into()));
        }
        Ok(())
    }

    fn assistant_text(message: &Value) -> String {
        message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<String>()
    }

    fn project_tool_calls(message: &Value) -> Result<Vec<Value>, DomainError> {
        let calls = message["content"]
            .as_array()
            .ok_or_else(|| DomainError::InvalidInput("local assistant content".into()))?
            .iter()
            .filter(|block| block["type"] == "toolCall")
            .cloned()
            .collect::<Vec<_>>();
        for call in &calls {
            let id = call["id"].as_str().unwrap_or_default();
            let name = call["name"].as_str().unwrap_or_default();
            if id.is_empty() || id.len() > 128 || name.is_empty() || name.len() > 128 {
                return Err(DomainError::InvalidInput("local project tool call".into()));
            }
        }
        if calls.len() > 8 {
            return Err(DomainError::LimitExceeded(
                "local project tool calls".into(),
            ));
        }
        Ok(calls)
    }

    fn compile_preparation(
        &self,
        preparation: &PreparationAction,
        operation: &str,
        timestamp: &str,
    ) -> Result<CompiledContext, DomainError> {
        let mut context = ContextStore::new(
            self.store.as_ref(),
            self.session.clone(),
            &self.origin,
            operation,
            timestamp,
            Features::default(),
        )?;
        let messages = preparation.messages().values().cloned().collect::<Vec<_>>();
        // Tasks are durable custom records, not chat messages. Build AG1's
        // request-local projection from the captured branch so a task set in
        // the details drawer reaches the model without changing chat history.
        let working_state = runtime_context::working_state_with_control_tool(
            preparation.branch(),
            &messages,
            &context,
            |_| None,
            None,
        );
        let request_messages =
            runtime_context::append_working_state(&messages, working_state.as_deref())?;
        let mut options = preparation.compile_options();
        if let Some(working_state) = &working_state {
            let task_tokens = ag1_core::context::estimate_tokens(&json!({
                "role":"custom",
                "content":working_state,
            }));
            options.fixed_tokens = Some(
                options
                    .fixed_tokens
                    .unwrap_or_else(|| preparation.configuration().fixed_tokens())
                    .saturating_add(task_tokens),
            );
        }
        let mut compiled =
            context.compile(&request_messages, &options, preparation.journal_revision())?;
        // Context compilation preserves AG1's durable/custom message shape.
        // Convert its request-only tail before serializing the llama.cpp call.
        compiled.projection.messages =
            runtime_messages::to_provider(&compiled.projection.messages)?;
        Ok(CompiledContext {
            projection: compiled.projection,
            original_journal: compiled.original_journal,
            original_request: compiled.original_request,
            revision: compiled.revision,
        })
    }

    fn build_local_generation(
        preparation: &PreparationAction,
        projection: Compilation,
        operation: &str,
        request_tools: Option<&[Value]>,
    ) -> Result<(String, GenerationRequest), DomainError> {
        let configuration = preparation.configuration();
        let tools = request_tools.unwrap_or(&configuration.tools);
        let endpoint = Self::local_endpoint(configuration)?;
        let family: ApiFamily = serde_json::from_value(configuration.model["api"].clone())
            .map_err(|_| DomainError::InvalidInput("local provider family".into()))?;
        let context = json!({
            "systemPrompt": configuration.system_prompt,
            "messages": projection.messages,
            "tools": tools,
        });
        let options = session_options(&configuration.model, &configuration.thinking, None)?;
        // llama.cpp accepts this local bearer placeholder when no API key is
        // configured. It is not a user secret and never leaves loopback.
        let local_credential = SecretBytes::new(b"ag1-local".to_vec());
        let request = generation_http_request(
            &RequestContext {
                model: &configuration.model,
                context: &context,
                options: &options,
                anthropic_oauth: false,
                google_project: None,
                request_id: operation,
            },
            HttpCredential::Bearer(&local_credential),
            32 * 1024 * 1024,
        )?;
        Ok((
            endpoint,
            GenerationRequest {
                request_id: operation.into(),
                tool_names: tools
                    .iter()
                    .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                    .collect(),
                family,
                request,
                limits: GenerationLimits {
                    max_output_bytes: 32 * 1024 * 1024,
                    prefill_ms: 120_000,
                    idle_ms: 120_000,
                    overall_ms: 0,
                },
            },
        ))
    }

    fn local_compaction_settings(context_limit: u64) -> Settings {
        // The upstream defaults target large working windows. Scale them down
        // for a smaller model so the compaction request itself still fits in
        // the selected context instead of failing after the journal is fenced.
        let reserve_tokens = 40_000.min((context_limit / 2).max(2_048));
        let keep_recent_tokens = 20_000.min((context_limit / 4).max(1_024));
        Settings {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
        }
    }

    async fn summarize_compaction_part(
        &self,
        capture: &CompactionCapture,
        preparation: &Value,
        focus: Option<&str>,
        split_prefix: bool,
        command: &str,
        timestamp: &str,
        scheduler: &Scheduler,
        http: &ProviderHttp,
        cancel: &mut CancelToken,
        observe: &mut impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
    ) -> Result<(String, ContextArtifact), DomainError> {
        let key = if split_prefix {
            "turnPrefixMessages"
        } else {
            "messagesToSummarize"
        };
        let messages = preparation[key]
            .as_array()
            .ok_or_else(|| DomainError::InvalidInput("compaction summary messages".into()))?
            .clone();
        if messages.is_empty() {
            return Err(DomainError::InvalidInput(
                "compaction summary has no messages".into(),
            ));
        }

        // Use AG1's isolated summary prompt. The normal system prompt, tools,
        // and reasoning mode would make a summary less predictable and waste
        // part of the selected model's context window.
        let isolated = summary_request::request(SummaryRequest {
            messages: &messages,
            reserve_tokens: Self::local_compaction_settings(
                capture.configuration().context_limit(),
            )
            .reserve_tokens,
            previous: preparation["previousSummary"].as_str(),
            focus,
            memory: Some(&preparation["workingMemory"]),
            split_prefix,
            root_session: Some(self.session.as_str()),
            model_max: capture.configuration().model["maxTokens"]
                .as_u64()
                .unwrap_or(0),
        })?;
        let messages = isolated.context["messages"]
            .as_array()
            .cloned()
            .ok_or_else(|| DomainError::InvalidInput("compaction summary request".into()))?;
        let system_prompt = isolated.context["systemPrompt"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("compaction summary prompt".into()))?;
        let mut configuration = capture.configuration().clone();
        configuration.tools.clear();
        configuration.thinking = "off".into();
        configuration.system_prompt = system_prompt.into();

        // Refuse a summary request which would overflow the same model window
        // used for normal turns. A compacted chat must not rely on a hidden
        // larger context than the user selected.
        let estimated = configuration.fixed_tokens()
            + messages
                .iter()
                .map(ag1_core::context::estimate_tokens)
                .sum::<u64>();
        if estimated.saturating_mul(11).div_ceil(10) > configuration.context_limit() {
            return Err(DomainError::LimitExceeded(
                "compaction summary context admission margin".into(),
            ));
        }

        self.state()?.check_compaction_capture(capture)?;
        let endpoint = Self::local_endpoint(&configuration)?;
        let family: ApiFamily = serde_json::from_value(configuration.model["api"].clone())
            .map_err(|_| DomainError::InvalidInput("local provider family".into()))?;
        let context = json!({
            "systemPrompt": configuration.system_prompt,
            "messages": messages,
            "tools": [],
        });
        let overrides = json!({
            "maxTokens": isolated.max_tokens,
            "sessionId": isolated.session_id,
            "cacheRetention": "none",
        });
        let options = session_options(
            &configuration.model,
            &configuration.thinking,
            Some(&overrides),
        )?;
        let credential = SecretBytes::new(b"ag1-local".to_vec());
        let operation = format!(
            "{command}-summary-{}",
            if split_prefix { "prefix" } else { "history" }
        );
        let request = generation_http_request(
            &RequestContext {
                model: &configuration.model,
                context: &context,
                options: &options,
                anthropic_oauth: false,
                google_project: None,
                request_id: &operation,
            },
            HttpCredential::Bearer(&credential),
            32 * 1024 * 1024,
        )?;
        let generation = GenerationRequest {
            request_id: operation.clone(),
            tool_names: Vec::new(),
            family,
            request,
            limits: GenerationLimits {
                max_output_bytes: 32 * 1024 * 1024,
                prefill_ms: 120_000,
                idle_ms: 120_000,
                overall_ms: 0,
            },
        };

        scheduler.register(&endpoint, EndpointKind::Local)?;
        let ticket = scheduler.submit(&endpoint, OperationId::new(operation.clone())?)?;
        let permit = loop {
            if cancel.cancelled() {
                scheduler.cancel(&ticket)?;
                return Err(DomainError::Cancelled);
            }
            if let Some(permit) = scheduler.start(&ticket)? {
                break permit;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        let outcome =
            request_generation(http, generation, permit, cancel, |event| observe(event)).await;
        let (raw_snapshot, raw_error) = match &outcome {
            Ok(snapshot) => (Some(snapshot), None),
            Err(failure) => (failure.partial.as_ref(), Some(&failure.error)),
        };
        let output = self.retain_provider_output(&operation, timestamp, raw_snapshot, raw_error)?;
        let snapshot = outcome.map_err(|failure| failure.error)?;
        if matches!(snapshot.stop_reason, Some(StopReason::Error)) {
            return Err(DomainError::Unavailable(
                snapshot
                    .error
                    .unwrap_or_else(|| "local compaction summary failed".into()),
            ));
        }
        if matches!(snapshot.stop_reason, Some(StopReason::Aborted)) {
            return Err(DomainError::Cancelled);
        }
        let text = snapshot
            .content
            .iter()
            .filter(|block| block.kind == BlockKind::Text)
            .map(|block| block.text.as_str())
            .collect::<String>();
        if text.trim().is_empty() {
            return Err(DomainError::Unavailable(
                "local compaction summary returned no text".into(),
            ));
        }
        Ok((text, output))
    }

    fn local_endpoint(configuration: &Configuration) -> Result<String, DomainError> {
        let family: ApiFamily = serde_json::from_value(configuration.model["api"].clone())
            .map_err(|_| DomainError::InvalidInput("local provider family".into()))?;
        if family != ApiFamily::Completions
            || configuration.model["provider"].as_str() != Some("llamacpp")
        {
            return Err(DomainError::Unavailable(
                "Windows native sessions currently support local llama.cpp models only".into(),
            ));
        }
        let base_url = configuration.model["baseUrl"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("local model URL".into()))?;
        let url = reqwest::Url::parse(base_url)
            .map_err(|_| DomainError::InvalidInput("local model URL".into()))?;
        let host = url.host_str().unwrap_or_default();
        if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
            return Err(DomainError::PermissionDenied);
        }
        endpoint_origin(base_url)
    }

    fn retain_provider_output(
        &self,
        operation: &str,
        timestamp: &str,
        snapshot: Option<&ag1_core::provider::ProviderSnapshot>,
        error: Option<&DomainError>,
    ) -> Result<ContextArtifact, DomainError> {
        let envelope = json!({
            "version": 1,
            "kind": "runtime-provider-output",
            "operation": operation,
            "snapshot": snapshot,
            "error": error.map(|value| format!("{value:?}")),
        });
        let mut context = ContextStore::new(
            self.store.as_ref(),
            self.session.clone(),
            &self.origin,
            operation,
            timestamp,
            Features::default(),
        )?;
        context.put_evidence(
            &envelope.to_string(),
            "runtime-provider-output",
            &json!({"type":"session","name":"provider-output","callId":operation}),
            &[],
            None,
        )
    }

    fn discard_preparation(&self, preparation: PreparationAction, error: DomainError) {
        if let Ok(mut state) = self.state() {
            let _ = state.admit_request(preparation, Err(error));
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, SessionRuntime>, DomainError> {
        self.state
            .lock()
            .map_err(|_| DomainError::Unavailable("Windows session owner poisoned".into()))
    }

    fn commit(
        state: &mut SessionRuntime,
        action: ag1_core::runtime::session::AppendAction,
        store: &Store,
    ) -> Result<Commit, DomainError> {
        // `execute` owns the exact prepared bytes. Acknowledging anything else
        // would allow memory and storage to diverge after a crash or conflict.
        let receipt = action.execute(store);
        state.acknowledge(receipt)?;
        Ok(Commit {
            revision: state.journal_revision(),
            sequence: state.committed_sequence(),
        })
    }
}

fn system_random(bytes: &mut [u8]) -> Result<(), DomainError> {
    // Windows CNG is the OS CSPRNG. We need it for durable cursor keys; a clock
    // or process ID would make cursors predictable across fresh sessions.
    let result = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes
                .len()
                .try_into()
                .map_err(|_| DomainError::LimitExceeded("random key length".into()))?,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(DomainError::Unavailable(
            "Windows system random generator".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        change_receipts::{self, ChangeReceipt},
        folder_access::{Change as FolderChange, FolderAccess},
        workspace::WorkspaceService,
    };
    use ag1_core::{context::compiler::CompileOptions, runtime::session::Configuration};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };
    use tempfile::tempdir;

    fn config() -> Configuration {
        Configuration {
            model: json!({
                "provider": "llamacpp",
                "id": "test-model",
                "contextWindow": 4096,
            }),
            thinking: "off".into(),
            tools: vec![],
            system_prompt: "You are a local test model.".into(),
            prompt: None,
            compiler: CompileOptions::default(),
        }
    }

    #[test]
    fn project_tools_are_small_and_stay_inside_the_workspace() {
        let project = tempdir().unwrap();
        std::fs::write(project.path().join("notes.txt"), "first line\r\n").unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(project.path())
            .unwrap();
        let tools = LocalProjectTools::new(workspace);

        let definitions = LocalProjectTools::definitions_for(ProjectWriteAccess::AllowWrites);
        assert_eq!(definitions.len(), 5);
        assert_eq!(definitions[0]["name"], "list_project_files");
        assert_eq!(definitions[1]["name"], "read_project_file");
        assert_eq!(definitions[2]["name"], "search_project_files");
        assert_eq!(definitions[3]["name"], "edit_project_file");
        assert_eq!(definitions[4]["name"], "write_project_file");

        let web = LocalProjectTools::web_definitions();
        assert_eq!(web.len(), 2);
        assert_eq!(web[0]["name"], "webSearch");
        assert_eq!(web[1]["name"], "webFetch");
        assert_eq!(web[0]["parameters"]["required"], json!(["query"]));

        let listed = tools.execute(&json!({
            "id":"list-project",
            "name":"list_project_files",
            "arguments":{}
        }));
        assert_eq!(listed["isError"], false);
        assert_eq!(listed["content"][0]["text"], "notes.txt");

        let read = tools.execute(&json!({
            "id":"read-notes",
            "name":"read_project_file",
            "arguments":{"path":"notes.txt"}
        }));
        assert_eq!(read["isError"], false);
        assert_eq!(read["content"][0]["text"], "first line\r\n");

        let search = tools.execute(&json!({
            "id":"search-notes",
            "name":"search_project_files",
            "arguments":{"query":"FIRST"}
        }));
        assert_eq!(search["isError"], false);
        assert_eq!(search["content"][0]["text"], "notes.txt:1:first line");

        let invalid_search = tools.execute(&json!({
            "id":"empty-search",
            "name":"search_project_files",
            "arguments":{"query":"  "}
        }));
        assert_eq!(invalid_search["isError"], true);
        assert_eq!(
            invalid_search["content"][0]["text"],
            "Use a short, non-empty text search query."
        );

        let escaped = tools.execute(&json!({
            "id":"outside-project",
            "name":"read_project_file",
            "arguments":{"path":"../outside.txt"}
        }));
        assert_eq!(escaped["isError"], true);
        assert_eq!(
            escaped["content"][0]["text"],
            "Use a relative UTF-8 project file path."
        );

        let denied = tools.execute(&json!({
            "id":"denied-edit",
            "name":"edit_project_file",
            "arguments":{"path":"notes.txt","oldText":"first","newText":"second"}
        }));
        assert_eq!(denied["isError"], true);
        assert_eq!(
            denied["content"][0]["text"],
            "This chat is set to ask before changes. Choose Allow routine changes or Bypass approvals, then ask again."
        );
    }

    #[test]
    fn repeated_failed_tool_calls_are_detected_without_matching_new_arguments() {
        let first = json!({
            "id":"first-search",
            "name":"webSearch",
            "arguments":{"query":"AG1W release notes"}
        });
        let same_request = json!({
            "id":"second-search",
            "name":"webSearch",
            "arguments":{"query":"AG1W release notes"}
        });
        let different_request = json!({
            "id":"third-search",
            "name":"webSearch",
            "arguments":{"query":"AG1W native model setup"}
        });
        let failed = vec![("webSearch".to_owned(), first["arguments"].clone())];

        assert!(repeats_failed_call(&failed, &same_request));
        assert!(!repeats_failed_call(&failed, &different_request));

        let result = repeated_tool_error_result(&same_request);
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("did not run it again")
        );
    }

    #[test]
    fn native_web_policy_uses_a_portable_captured_workspace() {
        let decision = ag1_core::tools::policy::evaluate(
            "webSearch",
            &json!({"query":"Qwen release notes"}),
            &NativeSession::web_tool_policy(),
            false,
        )
        .unwrap();
        assert!(decision.allow);
        assert!(decision.reason.is_none());
    }

    #[test]
    fn project_tools_use_only_the_folders_granted_to_this_chat() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        let extra = root.path().join("extra");
        let state = root.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(project.join("main.txt"), "working folder").unwrap();
        std::fs::write(extra.join("notes.txt"), "granted folder").unwrap();

        let workspace = WorkspaceService::new().unwrap().register(&project).unwrap();
        let access = FolderAccess::open(workspace, &state, "folder-tool-chat").unwrap();
        let revision = access.snapshot().unwrap().selection.revision;
        access
            .change(FolderChange {
                revision,
                action: "add".into(),
                path: extra.clone(),
                mode: None,
            })
            .unwrap();
        let folders = access.snapshot().unwrap();
        let extra_path = folders.selection.folders[1]
            .path
            .join("notes.txt")
            .to_string_lossy()
            .into_owned();
        let tools = LocalProjectTools::with_folder_access(
            folders,
            ProjectWriteAccess::AllowWrites,
            state.join("logs"),
        );

        let listed = tools.execute(&json!({
            "id":"folder-list", "name":"list_project_files", "arguments":{}
        }));
        let listing = listed["content"][0]["text"].as_str().unwrap();
        assert!(listing.contains("main.txt"));
        assert!(
            listing
                .lines()
                .any(|path| path.replace('\\', "/").ends_with("/extra/notes.txt")),
            "listing was: {listing}"
        );

        let read = tools.execute(&json!({
            "id":"folder-read", "name":"read_project_file", "arguments":{"path":extra_path}
        }));
        assert_eq!(read["isError"], false);
        assert_eq!(read["content"][0]["text"], "granted folder");

        let denied = tools.execute(&json!({
            "id":"folder-write", "name":"write_project_file", "arguments":{"path":extra_path,"content":"changed"}
        }));
        assert_eq!(denied["isError"], true);
        assert_eq!(
            std::fs::read_to_string(extra.join("notes.txt")).unwrap(),
            "granted folder"
        );

        let escaped = tools.execute(&json!({
            "id":"folder-escape", "name":"read_project_file", "arguments":{"path":root.path().join("outside.txt").to_string_lossy()}
        }));
        assert_eq!(escaped["isError"], true);
    }

    #[test]
    fn combined_folder_search_output_stays_within_one_tool_result_budget() {
        let text = format!("{}😀", "match\n".repeat(PROJECT_TOOL_MAX_TEXT));
        let bounded = bounded_project_tool_text(text);
        assert!(bounded.len() <= PROJECT_TOOL_MAX_TEXT);
        assert!(bounded.ends_with("narrow the query or choose one folder.]"));
    }

    #[test]
    fn project_write_tools_keep_edits_exact_and_preserve_file_text_style() {
        let project = tempdir().unwrap();
        std::fs::write(project.path().join("notes.txt"), "\u{feff}first line\r\n").unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(project.path())
            .unwrap();
        let tools = LocalProjectTools::with_write_access(
            workspace.clone(),
            ProjectWriteAccess::AllowWrites,
        );

        let updated = tools.execute(&json!({
            "id":"change-notes",
            "name":"edit_project_file",
            "arguments":{"path":"notes.txt","oldText":"first","newText":"second"}
        }));
        assert_eq!(updated["isError"], false);
        assert_eq!(
            std::fs::read_to_string(project.path().join("notes.txt")).unwrap(),
            "\u{feff}second line\r\n"
        );

        let ambiguous = tools.execute(&json!({
            "id":"ambiguous-edit",
            "name":"edit_project_file",
            "arguments":{"path":"notes.txt","oldText":"e","newText":"_"}
        }));
        assert_eq!(ambiguous["isError"], true);
        assert_eq!(
            ambiguous["content"][0]["text"],
            "The oldText must appear exactly once. Read the file again and make the edit more specific."
        );

        let created = tools.execute(&json!({
            "id":"create-file",
            "name":"write_project_file",
            "arguments":{"path":"src/new.txt","content":"new file\n","createParents":true}
        }));
        assert_eq!(created["isError"], false);
        assert_eq!(
            std::fs::read_to_string(project.path().join("src").join("new.txt")).unwrap(),
            "new file\n"
        );

        let replaced = tools.execute(&json!({
            "id":"replace-file",
            "name":"write_project_file",
            "arguments":{"path":"notes.txt","content":"whole file\n"}
        }));
        assert_eq!(replaced["isError"], false);
        assert_eq!(
            std::fs::read_to_string(project.path().join("notes.txt")).unwrap(),
            "\u{feff}whole file\r\n"
        );
    }

    #[test]
    fn native_project_edit_keeps_a_verified_before_and_after_receipt() {
        let data = tempdir().unwrap();
        let project = tempdir().unwrap();
        std::fs::write(project.path().join("notes.txt"), "first line\n").unwrap();
        let store = Arc::new(Store::open(data.path()).unwrap());
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(project.path())
            .unwrap();
        let session_id = SessionId::new("windows-project-receipt").unwrap();
        let session = NativeSession::create(
            store.clone(),
            session_id.clone(),
            "windows-project-receipt-origin",
            project.path().to_str().unwrap(),
            "2026-11-13T00:00:00.000Z",
            1,
            config(),
        )
        .unwrap();
        session
            .append_message(
                "receipt-user",
                "receipt-user",
                "2026-11-13T00:00:01.000Z",
                json!({"role":"user","content":"Update notes.txt."}),
            )
            .unwrap();
        let call = json!({
            "type":"toolCall",
            "id":"receipt-edit-notes",
            "name":"edit_project_file",
            "arguments":{"path":"notes.txt","oldText":"first","newText":"second"}
        });
        session
            .append_message(
                "receipt-assistant",
                "receipt-assistant",
                "2026-11-13T00:00:02.000Z",
                json!({"role":"assistant","content":[call.clone()]}),
            )
            .unwrap();

        let tools = LocalProjectTools::with_write_access(
            workspace.clone(),
            ProjectWriteAccess::AllowWrites,
        );
        let change = LocalChangeContext {
            store: store.as_ref(),
            session: session_id.as_str(),
            operation: "receipt-assistant",
            timestamp_ms: 1_763_000_002_000,
        };
        let result = tools.execute_with_changes(&call, Some(&change));
        assert_eq!(result["isError"], false);
        assert_eq!(
            std::fs::read_to_string(project.path().join("notes.txt")).unwrap(),
            "second line\n"
        );

        let receipt_value = store
            .native_file_changes(session_id.as_str(), &["receipt-edit-notes".into()])
            .unwrap()
            .pop()
            .unwrap();
        let receipt: ChangeReceipt = serde_json::from_value(receipt_value).unwrap();
        assert_eq!(receipt.state, "committed");
        assert_eq!(receipt.call_entry.as_deref(), Some("receipt-assistant"));
        assert_eq!(receipt.tool, "edit_project_file");
        assert_eq!(receipt.path, "notes.txt");
        let records = session.records().unwrap();
        change_receipts::verify_branch(
            &receipt,
            records.as_ref(),
            session_id.as_str(),
            workspace.id().as_str(),
        )
        .unwrap();
        assert_eq!(
            change_receipts::read_snapshot(
                store.as_ref(),
                session_id.as_str(),
                receipt.before.as_ref().unwrap(),
            )
            .unwrap(),
            b"first line\n"
        );
        assert_eq!(
            change_receipts::read_snapshot(store.as_ref(), session_id.as_str(), &receipt.after)
                .unwrap(),
            b"second line\n"
        );
    }

    #[test]
    fn project_command_uses_an_owned_direct_child_and_never_accepts_a_shell() {
        let project = tempdir().unwrap();
        let logs = tempdir().unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(project.path())
            .unwrap();
        let tools = LocalProjectTools::with_command_runner(
            workspace,
            ProjectWriteAccess::AllowWrites,
            logs.path().to_path_buf(),
        );

        let definitions = tools.request_definitions();
        assert_eq!(definitions.len(), 6);
        assert_eq!(definitions[5]["name"], "run_project_command");

        // Cargo is already the current test runner's direct executable. This
        // fixture proves AG1W uses its owned Windows child route without asking
        // a shell to interpret a command string.
        let completed = tools.execute(&json!({
            "id":"cargo-version",
            "name":"run_project_command",
            "arguments":{"program":"cargo","args":["--version"],"timeoutMs":10000}
        }));
        assert_eq!(completed["isError"], false);
        assert!(
            completed["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Program: cargo")
        );
        assert!(
            completed["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Exit code: 0")
        );
        assert!(std::fs::read_dir(logs.path()).unwrap().next().is_none());

        let rejected = tools.execute(&json!({
            "id":"shell-is-not-a-program",
            "name":"run_project_command",
            "arguments":{"program":"cmd","args":["/c","echo no"]}
        }));
        assert_eq!(rejected["isError"], true);
        assert_eq!(
            rejected["content"][0]["text"],
            "Use one direct developer program: cargo, dotnet, git, node, or python. Shell command strings are not supported."
        );

        let eval = tools.execute(&json!({
            "id":"python-eval-is-not-a-script-tool",
            "name":"run_project_command",
            "arguments":{"program":"python","args":["-c","print('no')"]}
        }));
        assert_eq!(eval["isError"], true);
        assert_eq!(
            eval["content"][0]["text"],
            "Use a short array of plain command-line arguments; shell and interpreter eval flags are not supported."
        );
    }

    #[test]
    fn project_file_list_is_bounded_before_it_enters_model_context() {
        let paths = (0..512)
            .map(|index| format!("src/{index:04}-{}.rs", "x".repeat(96)))
            .collect();
        let list = project_file_list(paths);

        assert!(list.len() <= PROJECT_TOOL_MAX_TEXT);
        assert!(list.contains("[File list truncated; ask for a narrower path.]"));
    }

    #[test]
    fn project_search_limits_results_before_they_reach_model_context() {
        let project = tempdir().unwrap();
        std::fs::write(project.path().join("matches.txt"), "needle\n".repeat(120)).unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(project.path())
            .unwrap();

        let result = project_text_search(workspace.as_ref(), "needle").unwrap();
        assert_eq!(result.lines().count(), PROJECT_SEARCH_MAX_MATCHES);
        assert!(result.len() <= PROJECT_TOOL_MAX_TEXT);
        assert!(result.starts_with("matches.txt:1:needle"));
    }

    #[test]
    fn project_tool_evidence_survives_context_compilation() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-project-tool-context").unwrap();
        let mut configuration = config();
        configuration.tools = LocalProjectTools::definitions();
        let session = NativeSession::create(
            store,
            id,
            "windows-origin",
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            configuration,
        )
        .unwrap();

        session
            .append_message(
                "tool-context-user",
                "tool-context-user",
                "2026-11-12T00:00:01.000Z",
                json!({"role":"user","content":"Read notes.txt."}),
            )
            .unwrap();
        session
            .append_message(
                "tool-context-call",
                "tool-context-call",
                "2026-11-12T00:00:02.000Z",
                json!({
                    "role":"assistant",
                    "content":[{
                        "type":"toolCall",
                        "id":"read-notes",
                        "name":"read_project_file",
                        "arguments":{"path":"notes.txt"}
                    }]
                }),
            )
            .unwrap();
        session
            .append_message(
                "tool-context-result",
                "tool-context-result",
                "2026-11-12T00:00:03.000Z",
                json!({
                    "role":"toolResult",
                    "toolCallId":"read-notes",
                    "toolName":"read_project_file",
                    "content":[{"type":"text","text":"first line"}],
                    "isError":false
                }),
            )
            .unwrap();

        let compiled = session
            .compile_context("tool-context-compile", "2026-11-12T00:00:04.000Z")
            .unwrap();
        assert_eq!(compiled.projection.messages.len(), 3);
        assert_eq!(
            compiled.projection.messages[1]["content"][0]["name"],
            "read_project_file"
        );
        assert_eq!(compiled.projection.messages[2]["toolCallId"], "read-notes");
        assert_eq!(
            compiled.projection.messages[2]["content"][0]["text"],
            "first line"
        );
    }

    #[test]
    fn project_tools_use_the_llama_cpp_function_call_shape() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-project-tool-request").unwrap();
        let mut configuration = config();
        configuration.model["api"] = json!("openai-completions");
        configuration.model["baseUrl"] = json!("http://127.0.0.1:18082/v1");
        configuration.tools = LocalProjectTools::definitions();
        let session = NativeSession::create(
            store,
            id,
            "windows-origin",
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            configuration,
        )
        .unwrap();

        session
            .append_message(
                "tool-request-user",
                "tool-request-user",
                "2026-11-12T00:00:01.000Z",
                json!({"role":"user","content":"List the project files."}),
            )
            .unwrap();
        let preparation = session.state().unwrap().prepare_request().unwrap();
        let compiled = session
            .compile_preparation(
                &preparation,
                "tool-request-compile",
                "2026-11-12T00:00:02.000Z",
            )
            .unwrap();
        let (_, generation) = NativeSession::build_local_generation(
            &preparation,
            compiled.projection,
            "tool-request-generate",
            None,
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&generation.request.body).unwrap();

        assert_eq!(
            generation.tool_names,
            [
                "list_project_files",
                "read_project_file",
                "search_project_files"
            ]
        );
        assert_eq!(body["tools"].as_array().unwrap().len(), 3);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "list_project_files");
        assert_eq!(
            body["tools"][1]["function"]["parameters"]["required"],
            json!(["path"])
        );
        assert_eq!(body["tools"][2]["function"]["name"], "search_project_files");
    }

    #[test]
    fn write_tool_schemas_are_sent_only_for_an_explicit_allow_changes_prompt() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-project-write-request").unwrap();
        let mut configuration = config();
        configuration.model["api"] = json!("openai-completions");
        configuration.model["baseUrl"] = json!("http://127.0.0.1:18082/v1");
        configuration.tools = LocalProjectTools::definitions();
        let session = NativeSession::create(
            store,
            id,
            "windows-origin",
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            configuration,
        )
        .unwrap();

        session
            .append_message(
                "write-request-user",
                "write-request-user",
                "2026-11-12T00:00:01.000Z",
                json!({"role":"user","content":"Create a project note."}),
            )
            .unwrap();
        let preparation = session.state().unwrap().prepare_request().unwrap();
        let compiled = session
            .compile_preparation(
                &preparation,
                "write-request-compile",
                "2026-11-12T00:00:02.000Z",
            )
            .unwrap();
        let write_tools = LocalProjectTools::definitions_for(ProjectWriteAccess::AllowWrites);
        let (_, generation) = NativeSession::build_local_generation(
            &preparation,
            compiled.projection,
            "write-request-generate",
            Some(&write_tools),
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&generation.request.body).unwrap();

        assert_eq!(generation.tool_names.len(), 5);
        assert_eq!(body["tools"].as_array().unwrap().len(), 5);
        assert_eq!(body["tools"][3]["function"]["name"], "edit_project_file");
        assert_eq!(body["tools"][4]["function"]["name"], "write_project_file");
    }

    #[test]
    fn durable_session_resumes_and_compiles_real_context() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-session").unwrap();
        let session = NativeSession::create(
            store.clone(),
            id.clone(),
            "windows-origin",
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            config(),
        )
        .unwrap();

        let commit = session
            .append_message(
                "user-message",
                "user-message",
                "2026-11-12T00:00:01.000Z",
                json!({"role":"user","content":"Inspect this Windows lab."}),
            )
            .unwrap();
        assert_eq!(commit.revision, 1);

        let compiled = session
            .compile_context("compile-context", "2026-11-12T00:00:02.000Z")
            .unwrap();
        assert_eq!(compiled.revision, 1);
        assert_eq!(compiled.projection.messages.len(), 1);
        assert_eq!(
            compiled.projection.messages[0]["content"],
            "Inspect this Windows lab."
        );
        assert_ne!(compiled.original_journal.id, compiled.original_request.id);

        let resumed = NativeSession::resume(store, id, "windows-origin", 2, config()).unwrap();
        assert_eq!(resumed.journal_revision().unwrap(), 1);
        assert_eq!(resumed.records().unwrap().len(), 1);
    }

    #[test]
    fn durable_task_state_is_projected_into_the_next_local_request() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-task-projection").unwrap();
        let session = NativeSession::create(
            store,
            id,
            "windows-task-projection-origin",
            "C:\\lab",
            "2026-11-13T00:00:00.000Z",
            1,
            config(),
        )
        .unwrap();

        session
            .update_task(
                "set-task",
                "set-task-state",
                "2026-11-13T00:00:01.000Z",
                &json!({"action":"set","objective":"Document the Windows-native runtime."}),
            )
            .unwrap();
        session
            .append_message(
                "task-user",
                "task-user",
                "2026-11-13T00:00:02.000Z",
                json!({"role":"user","content":"What should I work on now?"}),
            )
            .unwrap();

        let preparation = session.state().unwrap().prepare_request().unwrap();
        let compiled = session
            .compile_preparation(
                &preparation,
                "task-projection-compile",
                "2026-11-13T00:00:03.000Z",
            )
            .unwrap();
        let request = serde_json::to_string(&compiled.projection.messages).unwrap();
        assert!(request.contains("Document the Windows-native runtime."));
        assert!(request.contains("Recorded task state"));
        session.discard_preparation(preparation, DomainError::Cancelled);
    }

    #[test]
    fn resumed_session_keeps_its_original_local_model() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-model-history").unwrap();
        NativeSession::create(
            store.clone(),
            id.clone(),
            "windows-origin",
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            config(),
        )
        .unwrap();

        let mut later_catalog_choice = config();
        later_catalog_choice.model["id"] = json!("different-local-model");
        later_catalog_choice.system_prompt = "A different prompt.".into();

        let resumed =
            NativeSession::resume(store, id, "windows-origin", 2, later_catalog_choice).unwrap();
        let configuration = resumed.configuration().unwrap();
        assert_eq!(configuration.model["id"], "test-model");
        assert_eq!(configuration.system_prompt, "You are a local test model.");
    }

    #[test]
    fn context_selection_survives_a_resume() {
        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-context").unwrap();
        let session = NativeSession::create(
            store.clone(),
            id.clone(),
            "context-origin",
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            config(),
        )
        .unwrap();

        session
            .select_context(
                "context-limit",
                "context-limit",
                "2026-11-12T00:00:01.000Z",
                2048,
            )
            .unwrap();

        let resumed = NativeSession::resume(store, id, "context-origin", 2, config()).unwrap();
        assert_eq!(resumed.configuration().unwrap().context_limit(), 2048);
    }

    #[tokio::test]
    async fn local_turn_uses_real_context_transport_and_durable_output() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = socket.read(&mut request).unwrap();
            let body = concat!(
                "data: {\"id\":\"windows-fixture\",\"choices\":[{\"delta\":{\"content\":\"Native reply\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2,\"total_tokens\":9}}\n\n",
                "data: [DONE]\n\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            socket.flush().unwrap();
        });

        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let session_id = SessionId::new("windows-local-turn").unwrap();
        let origin = "windows-local-origin";
        let mut configuration = config();
        configuration.model = json!({
            "provider": "llamacpp",
            "id": "fixture-model",
            "api": "openai-completions",
            "baseUrl": format!("http://{address}/v1"),
            "contextWindow": 4096,
            "maxTokens": 256,
            "input": ["text"],
            "reasoning": false,
        });
        let session = NativeSession::create(
            store.clone(),
            session_id.clone(),
            origin,
            "C:\\lab",
            "2026-11-12T00:00:00.000Z",
            1,
            configuration,
        )
        .unwrap();
        let endpoint = format!("http://{address}");
        let http = ProviderHttp::new(&[endpoint], Duration::from_secs(5)).unwrap();
        let scheduler = Scheduler::default();
        let (_source, mut cancel) = crate::provider::http::cancellation();

        let result = session
            .run_local_turn(
                LocalTurnInput {
                    command: "windows-local-command".into(),
                    user_id: "windows-local-user".into(),
                    assistant_id: "windows-local-assistant".into(),
                    timestamp: "2026-11-12T00:00:01.000Z".into(),
                    timestamp_ms: 1_763_000_001_000,
                    message: json!({"role":"user","content":"Reply with the fixture phrase."}),
                    system_addendum: None,
                },
                &scheduler,
                &http,
                &mut cancel,
                |_| Ok(()),
            )
            .await
            .unwrap();

        assert_eq!(result.user.revision, 1);
        assert_eq!(result.assistant.revision, 2);
        assert_eq!(result.message["content"][0]["text"], "Native reply");
        assert!(result.estimated_input_tokens > 0);
        let output = store
            .page(session_id.as_str(), origin, &result.output.id, 0, 32 * 1024)
            .unwrap();
        assert!(
            String::from_utf8(output.bytes)
                .unwrap()
                .contains("Native reply")
        );
        assert_eq!(session.records().unwrap().len(), 2);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn manual_compaction_keeps_raw_evidence_and_updates_the_context_epoch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = vec![0u8; 128 * 1024];
            let _ = socket.read(&mut request).unwrap();
            let body = concat!(
                "data: {\"id\":\"windows-compaction\",\"choices\":[{\"delta\":{\"content\":\"The earlier research established the local Windows runtime.\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":10,\"total_tokens\":110}}\n\n",
                "data: [DONE]\n\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            socket.flush().unwrap();
        });

        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let id = SessionId::new("windows-manual-compaction").unwrap();
        let mut configuration = config();
        configuration.model = json!({
            "provider":"llamacpp",
            "id":"fixture-model",
            "api":"openai-completions",
            "baseUrl":format!("http://{address}/v1"),
            "contextWindow":16_384,
            "maxTokens":256,
            "input":["text"],
            "reasoning":false,
        });
        let session = NativeSession::create(
            store.clone(),
            id.clone(),
            "windows-compaction-origin",
            "C:\\lab",
            "2026-11-13T00:00:00.000Z",
            1,
            configuration,
        )
        .unwrap();

        // This deliberately crosses the four-thousand-token retained tail for
        // a 16K context while keeping the summary request well inside it.
        for number in 0..5 {
            let content = format!("turn {number}: {}", "evidence ".repeat(700));
            session
                .append_message(
                    &format!("compact-user-{number}"),
                    &format!("compact-user-{number}"),
                    &format!("2026-11-13T00:00:0{number}.000Z"),
                    json!({"role":"user","content":content}),
                )
                .unwrap();
        }

        let endpoint = format!("http://{address}");
        let http = ProviderHttp::new(&[endpoint], Duration::from_secs(5)).unwrap();
        let scheduler = Scheduler::default();
        let (_source, mut cancel) = crate::provider::http::cancellation();
        let result = session
            .compact_local(
                "windows-manual-compact",
                "2026-11-13T00:01:00.000Z",
                Some("Preserve the runtime evidence."),
                &scheduler,
                &http,
                &mut cancel,
                |_| Ok(()),
            )
            .await
            .unwrap()
            .expect("the long fixture history should need compaction");

        assert!(result.context_epoch > 1);
        assert_eq!(result.journal_revision, 6);
        assert!(
            result.result["summary"]
                .as_str()
                .unwrap()
                .contains("earlier research established")
        );
        assert!(
            result.result["details"]["trajectoryArtifactId"]
                .as_str()
                .is_some()
        );
        assert_eq!(
            session.records().unwrap().last().unwrap()["type"],
            "compaction"
        );

        let resumed =
            NativeSession::resume(store, id, "windows-compaction-origin", 2, config()).unwrap();
        let record = resumed.records().unwrap().last().unwrap().clone();
        assert_eq!(record["type"], "compaction");
        assert!(record["details"]["originalJournal"].is_object());
        server.join().unwrap();
    }

    #[tokio::test]
    async fn local_plan_draft_and_plan_transitions_are_durable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 32 * 1024];
            let _ = socket.read(&mut request).unwrap();
            let body = concat!(
                "data: {\"id\":\"windows-plan\",\"choices\":[{\"delta\":{\"content\":\"## Summary\\nCreate the native planning workflow.\\n\\n## Plan\\n1. Add the durable planner.\\n2. Validate its journal records.\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":24,\"total_tokens\":36}}\n\n",
                "data: [DONE]\n\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            socket.flush().unwrap();
        });

        let directory = tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let mut configuration = config();
        configuration.model = json!({
            "provider":"llamacpp",
            "id":"fixture-model",
            "api":"openai-completions",
            "baseUrl":format!("http://{address}/v1"),
            "contextWindow":4096,
            "maxTokens":256,
            "input":["text"],
            "reasoning":false,
        });
        let session = NativeSession::create(
            store,
            SessionId::new("windows-local-plan").unwrap(),
            "windows-plan-origin",
            "C:\\lab",
            "2026-11-13T00:00:00.000Z",
            1,
            configuration,
        )
        .unwrap();
        let endpoint = format!("http://{address}");
        let http = ProviderHttp::new(&[endpoint], Duration::from_secs(5)).unwrap();
        let scheduler = Scheduler::default();
        let (_source, mut cancel) = crate::provider::http::cancellation();

        let result = session
            .draft_plan_local(
                LocalTurnInput {
                    command: "windows-plan-command".into(),
                    user_id: "windows-plan-user".into(),
                    assistant_id: "windows-plan-assistant".into(),
                    timestamp: "2026-11-13T00:00:01.000Z".into(),
                    timestamp_ms: 1_763_000_001_000,
                    message: json!({"role":"user","content":"Plan the native workflow."}),
                    system_addendum: Some(
                        "Draft a read-only plan in markdown with Summary and Plan headings.".into(),
                    ),
                },
                "Plan the native workflow.",
                None,
                None,
                &scheduler,
                &http,
                &mut cancel,
                |_| Ok(()),
            )
            .await
            .unwrap();
        let plan = result.plan.expect("fixture markdown should create a plan");
        assert_eq!(plan.status, Status::Draft);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(session.plan_snapshot(8).unwrap()[0].plan_id, plan.plan_id);
        assert!(session.state().unwrap().settled());

        let accepted = session
            .change_plan(
                "windows-plan-accept",
                "windows-plan-accept",
                "2026-11-13T00:00:02.000Z",
                "accept",
                &json!({"planId":plan.plan_id,"revision":plan.revision}),
            )
            .unwrap();
        assert_eq!(accepted.status, Status::Accepted);
        let completed = session
            .change_plan(
                "windows-plan-complete",
                "windows-plan-complete",
                "2026-11-13T00:00:03.000Z",
                "complete_step",
                &json!({"planId":plan.plan_id,"revision":plan.revision,"stepIndex":0}),
            )
            .unwrap();
        assert_eq!(completed.completed_step_indexes, Some(vec![0]));
        server.join().unwrap();
    }
}
