//! Native file tools. Preparation captures bytes; publication consumes an approval capability.
//! Construct one service per registered workspace. All writes also take P03's workspace/Git claim.
use crate::workspace::{MAX_READ, Workspace};
use ag1_core::{
    DomainError, FileRevision,
    tools::{
        approval::AuthorizedCall,
        file, preview,
        registry::{Origin, Registry, Scope, ValidatedCall},
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct QueueState {
    next: u64,
    waiting: VecDeque<u64>,
    active: bool,
}
#[derive(Default)]
struct Queue {
    state: Mutex<QueueState>,
    changed: Condvar,
}
struct QueuePermit(Arc<Queue>);
impl Drop for QueuePermit {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.active = false;
        self.0.changed.notify_all();
    }
}
impl Queue {
    fn enter(
        self: Arc<Self>,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<QueuePermit, DomainError> {
        let mut state = self.state.lock().unwrap();
        if state.waiting.len() >= 64 {
            return Err(DomainError::QueueFull);
        }
        let id = state.next;
        state.next = state.next.checked_add(1).ok_or(DomainError::Conflict)?;
        state.waiting.push_back(id);
        loop {
            let error = if cancel.load(Ordering::Acquire) {
                Some(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Some(DomainError::TimedOut)
            } else {
                None
            };
            if let Some(error) = error {
                state.waiting.retain(|v| *v != id);
                self.changed.notify_all();
                return Err(error);
            }
            if !state.active && state.waiting.front() == Some(&id) {
                state.waiting.pop_front();
                state.active = true;
                drop(state);
                return Ok(QueuePermit(self));
            }
            let wait =
                Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now()));
            state = self.changed.wait_timeout(state, wait).unwrap().0;
        }
    }
}
struct PreviewRecord {
    revision: FileRevision,
}
pub struct FileService {
    workspace: Arc<Workspace>,
    read_roots: RwLock<ReadRoots>,
    queues: Mutex<BTreeMap<String, Weak<Queue>>>,
    previews: Mutex<BTreeMap<(String, u64, String), PreviewRecord>>,
}
#[derive(Default)]
struct ReadRoots {
    revision: u64,
    roots: Vec<Arc<Workspace>>,
}
struct ReadSource {
    root: Arc<Workspace>,
    grant_revision: u64,
}
/// Private proposal; fields cannot be changed after policy/approval binds the revision.
pub struct PreparedFile {
    workspace: Arc<Workspace>,
    scope: Scope,
    call: ValidatedCall,
    path: String,
    expected: Option<FileRevision>,
    stamp: String,
    transformation: file::Transformation,
    preview_success: bool,
    streamed_read: bool,
    read_source: Option<ReadSource>,
    queue_permit: Option<QueuePermit>,
}
pub struct FileOutcome {
    pub result: Value,
    pub published: Option<FileRevision>,
}
pub struct FileExecution<'a, F: Fn() -> u64> {
    pub registry: &'a Registry,
    pub scope: &'a Scope,
    pub policy_revision: u64,
    pub now: F,
    pub cancel: &'a AtomicBool,
    pub deadline: Instant,
}
impl PreparedFile {
    /// Host read-only revision fence for higher-level settings transformations.
    pub fn source_revision(&self) -> Option<&ag1_core::FileRevision> {
        self.expected.as_ref()
    }
    pub fn target_revision(&self) -> &str {
        &self.stamp
    }
    pub fn proposed_result(&self) -> &Value {
        &self.transformation.result
    }
    pub fn will_mutate(&self) -> bool {
        self.transformation.bytes.is_some()
    }
}
impl FileService {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self {
            workspace,
            read_roots: RwLock::new(ReadRoots::default()),
            queues: Mutex::new(BTreeMap::new()),
            previews: Mutex::new(BTreeMap::new()),
        }
    }
    /// Host-only read capabilities for explicitly loaded skill/resource roots.
    /// Model paths cannot register a root. Replacing this set invalidates pending
    /// external reads; it grants no mutation, listing or search capability.
    pub fn set_read_roots(
        &self,
        expected: u64,
        roots: Vec<Arc<Workspace>>,
    ) -> Result<u64, DomainError> {
        if roots.len() > 64 {
            return Err(DomainError::LimitExceeded("resource read roots".into()));
        }
        for root in &roots {
            root.verify_root()?;
        }
        let mut current = self.read_roots.write().map_err(|_| DomainError::Conflict)?;
        if current.revision != expected {
            return Err(DomainError::Conflict);
        }
        let revision = expected.checked_add(1).ok_or(DomainError::Conflict)?;
        *current = ReadRoots { revision, roots };
        Ok(revision)
    }
    /// Lexical expansion is followed by P03 capability-relative opens, never a path-only containment check.
    pub fn relative_path(&self, input: &str) -> Result<String, DomainError> {
        let normal = self.absolute_path(input)?;
        Self::relative_to(&normal, self.workspace.root())
    }
    /// Recovery context for an already rejected lexical escape. It inspects only
    /// this workspace, never the attempted foreign path or loaded resource roots.
    pub fn rejection_hint(
        &self,
        reason: &DomainError,
        input: &str,
        check: &mut impl FnMut() -> Result<(), DomainError>,
    ) -> Result<Option<String>, DomainError> {
        check()?;
        if !matches!(reason, DomainError::PermissionDenied)
            || !matches!(
                self.relative_path(input),
                Err(DomainError::PermissionDenied)
            )
        {
            return Ok(None);
        }
        let attempted = self.absolute_path(input)?;
        let mut message = format!(
            "Path escapes the workspace. Workspace root: {}. Use relative paths from the root.",
            self.workspace.root().display()
        );
        let Some(basename) = attempted.file_name().and_then(|s| s.to_str()) else {
            return Ok(Some(message));
        };
        let mut pending = VecDeque::from([(String::new(), 0usize)]);
        let mut matches = Vec::new();
        let mut directories = 0;
        let mut entries_seen = 0;
        while let Some((directory, depth)) = pending.pop_front() {
            check()?;
            if directories >= 64 || entries_seen >= 4096 || matches.len() >= 3 {
                break;
            }
            directories += 1;
            let mut entries = match self.workspace.directory_entries(&directory) {
                Ok(entries) => entries,
                // Hints are best effort; failure never changes the rejection.
                Err(_) => continue,
            };
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            for entry in entries {
                check()?;
                if entries_seen >= 4096 || matches.len() >= 3 {
                    break;
                }
                entries_seen += 1;
                if entry.symlink {
                    continue;
                }
                let relative = if directory.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{directory}/{}", entry.name)
                };
                if entry.name == basename {
                    matches.push(relative);
                } else if entry.directory && depth < 4 && pending.len() < 64 {
                    pending.push_back((relative, depth + 1));
                }
            }
        }
        if !matches.is_empty() {
            let suffix = format!(
                " Files with this name exist in the workspace: {}.",
                matches.join(", ")
            );
            if message.len() + suffix.len() <= 8192 {
                message.push_str(&suffix);
            }
        }
        check()?;
        self.workspace.verify_root()?;
        Ok(Some(message))
    }
    fn relative_to(normal: &Path, root: &Path) -> Result<String, DomainError> {
        let relative = normal
            .strip_prefix(root)
            .map_err(|_| DomainError::PermissionDenied)?;
        if relative.components().count() > 32 {
            return Err(DomainError::InvalidInput("contained file path".into()));
        }
        relative
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| DomainError::InvalidInput("UTF-8 file path".into()))
    }
    fn absolute_path(&self, input: &str) -> Result<PathBuf, DomainError> {
        let home = std::env::var("HOME").ok();
        let mut value = ag1_core::tools::path::expand(input, home.as_deref())?;
        // Repair only the exact workspace prefix, not arbitrary guessed roots.
        let root = self
            .workspace
            .root()
            .to_str()
            .ok_or_else(|| DomainError::InvalidInput("UTF-8 workspace root".into()))?;
        if let Some(without_slash) = root.strip_prefix('/')
            && !without_slash.is_empty()
            && (value == without_slash
                || value
                    .strip_prefix(without_slash)
                    .is_some_and(|suffix| suffix.starts_with('/')))
        {
            value.insert(0, '/');
        }
        let raw = Path::new(&value);
        let absolute = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            self.workspace.root().join(raw)
        };
        let mut normal = PathBuf::new();
        for c in absolute.components() {
            match c {
                Component::ParentDir => {
                    normal.pop();
                }
                Component::CurDir => {}
                _ => normal.push(c.as_os_str()),
            }
        }
        Ok(normal)
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
    ) -> Result<PreparedFile, DomainError> {
        self.prepare_with_control(call, scope, &mut || Ok(()))
    }
    /// Queue an ordinary tool dispatch before capturing its source. The returned
    /// proposal holds only the per-file turn through policy/approval/execution;
    /// dropping a rejected, cancelled or expired proposal releases that turn.
    /// No workspace/Git claim is held while awaiting a user decision. Explicit
    /// optimistic proposals may still use `prepare` and conflict if bytes change.
    pub fn prepare_queued(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<PreparedFile, DomainError> {
        if call.origin() != Origin::BuiltIn || scope.workspace != self.workspace.id().as_str() {
            return Err(DomainError::PermissionDenied);
        }
        let permit = if matches!(
            call.name(),
            "write" | "edit" | "append" | "editApply" | "editPreview"
        ) {
            let raw = call.args()["path"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("path".into()))?;
            Some(
                self.queue(&self.relative_path(raw)?)?
                    .enter(cancel, deadline)?,
            )
        } else {
            None
        };
        let mut prepared = self.prepare_with_control(call, scope, &mut || {
            if cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                Ok(())
            }
        })?;
        prepared.queue_permit = permit;
        Ok(prepared)
    }
    /// Native operation control is checked between captures, streamed chunks and
    /// bounded image stages. Individual codec and filesystem calls are synchronous.
    pub fn prepare_with_control(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        check: &mut impl FnMut() -> Result<(), DomainError>,
    ) -> Result<PreparedFile, DomainError> {
        check()?;
        if call.origin() != Origin::BuiltIn || scope.workspace != self.workspace.id().as_str() {
            return Err(DomainError::PermissionDenied);
        }
        let raw = call.args()["path"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("path".into()))?;
        let mut path = match self.relative_path(raw) {
            Err(DomainError::PermissionDenied) if call.name() == "read" => {
                return self.prepare_allowed_read(call, scope, raw, check);
            }
            other => other?,
        };
        if call.name() == "read" {
            path = Self::read_path(&self.workspace, path, check)?;
        }
        if call.name() == "read" && self.workspace.directory_exists(&path)? {
            return Err(DomainError::InvalidInput(format!(
                "Path is a directory: {raw}. Use ls to inspect it, then read a specific file."
            )));
        }
        if path.is_empty() {
            return Err(DomainError::InvalidInput("contained file path".into()));
        }
        let snapshot = match self.workspace.snapshot(&path, MAX_READ) {
            Ok(s) => Some(s),
            Err(DomainError::NotFound) => None,
            Err(DomainError::LimitExceeded(_)) if call.name() == "read" => {
                return self.prepare_streamed(call, scope, path, check);
            }
            Err(e) => return Err(e),
        };
        check()?;
        let expected = snapshot.as_ref().map(|s| s.revision.clone());
        if call.name() == "editApply" {
            let records = self.previews.lock().unwrap();
            if let Some(prior) = records.get(&(scope.session.clone(), scope.epoch, path.clone()))
                && Some(&prior.revision) != expected.as_ref()
            {
                return Err(DomainError::Conflict);
            }
        }
        let mut preview_success = false;
        let transformation = if call.name() == "read" {
            let snapshot = snapshot.as_ref().ok_or_else(|| DomainError::InvalidInput(format!("Path not found: {raw}. Use find to locate files or ls to inspect directories before retrying read.")))?;
            let mut result =
                match crate::image_tools::read_with_control(&snapshot.bytes, true, check)? {
                    Some(image) => image,
                    None => ag1_core::tools::read::text(call.args(), &snapshot.bytes)?,
                };
            if result["details"]["progress"].is_object()
                && result["details"]["progress"]["reason"] != "line_too_large"
            {
                let source =
                    ag1_core::evidence::SourceReference::new(&path, &snapshot.revision.sha256)?;
                result["details"]["inspectionEvidence"] = serde_json::json!({"version":1,"kind":"read_range","source":source.reference(),"toolCallId":call.call_id(),"scope":{"startLine":result["details"]["progress"]["startLine"],"endLine":result["details"]["progress"]["endLine"]}});
            }
            file::Transformation {
                bytes: None,
                result,
                create_parents: false,
            }
        } else if call.name() == "editPreview" {
            let original = snapshot.as_ref().map(|s| String::from_utf8_lossy(&s.bytes));
            let p = preview::preview(
                raw,
                original.as_deref(),
                call.args()["oldText"]
                    .as_str()
                    .ok_or_else(|| DomainError::InvalidInput("oldText".into()))?,
                call.args()["newText"].as_str().unwrap_or(""),
            )?;
            preview_success = p.error.is_none();
            file::Transformation {
                bytes: None,
                result: file::structured(&p)?,
                create_parents: false,
            }
        } else if call.name() == "append"
            && snapshot.is_none()
            && !self.workspace.directory_exists(
                Path::new(&path)
                    .parent()
                    .and_then(Path::to_str)
                    .unwrap_or(""),
            )?
        {
            let parent = self
                .workspace
                .root()
                .join(&path)
                .parent()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            file::Transformation {
                bytes: None,
                create_parents: false,
                result: serde_json::json!({"content":[{"type":"text","text":serde_json::json!({"error":format!("parent directory does not exist: {parent}")}).to_string()}],"details":{"totalLines":0,"bytesAppended":0,"wasCreated":true}}),
            }
        } else {
            file::transform(
                call.name(),
                call.args(),
                snapshot.as_ref().map(|s| s.bytes.as_slice()),
            )?
        };
        let stamp = revision_stamp(self.workspace.id().as_str(), &path, expected.as_ref());
        Ok(PreparedFile {
            workspace: self.workspace.clone(),
            scope: scope.clone(),
            call: call.clone(),
            path,
            expected,
            stamp,
            transformation,
            preview_success,
            streamed_read: false,
            read_source: None,
            queue_permit: None,
        })
    }
    fn read_path(
        root: &Workspace,
        path: String,
        check: &mut impl FnMut() -> Result<(), DomainError>,
    ) -> Result<String, DomainError> {
        check()?;
        if root.exists(&path)? {
            return Ok(path);
        }
        for candidate in ag1_core::tools::path::read_variants(&path) {
            check()?;
            if root.exists(&candidate)? {
                return Ok(candidate);
            }
        }
        Ok(path)
    }
    fn prepare_streamed(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        path: String,
        check: &mut impl FnMut() -> Result<(), DomainError>,
    ) -> Result<PreparedFile, DomainError> {
        let mut window = ag1_core::tools::read::TextWindow::new(call.args())?;
        let mut first = true;
        let revision =
            self.workspace
                .stream(&path, crate::workspace::MAX_STREAM_READ, |bytes| {
                    check()?;
                    if first && crate::image_tools::supported_header(bytes) {
                        return Err(DomainError::LimitExceeded("encoded image input".into()));
                    }
                    first = false;
                    window.push(bytes)
                })?;
        check()?;
        let mut result = window.finish()?;
        if result["details"]["progress"]["reason"] != "line_too_large" {
            let source = ag1_core::evidence::SourceReference::new(&path, &revision.sha256)?;
            result["details"]["inspectionEvidence"] = serde_json::json!({"version":1,"kind":"read_range","source":source.reference(),"toolCallId":call.call_id(),"scope":{"startLine":result["details"]["progress"]["startLine"],"endLine":result["details"]["progress"]["endLine"]}});
        }
        let stamp = revision_stamp(self.workspace.id().as_str(), &path, Some(&revision));
        Ok(PreparedFile {
            workspace: self.workspace.clone(),
            scope: scope.clone(),
            call: call.clone(),
            path,
            expected: Some(revision),
            stamp,
            transformation: file::Transformation {
                bytes: None,
                result,
                create_parents: false,
            },
            preview_success: false,
            streamed_read: true,
            read_source: None,
            queue_permit: None,
        })
    }
    fn prepare_allowed_read(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        raw: &str,
        check: &mut impl FnMut() -> Result<(), DomainError>,
    ) -> Result<PreparedFile, DomainError> {
        check()?;
        let absolute = self.absolute_path(raw)?;
        let (root, path, grant_revision) = {
            let grants = self.read_roots.read().map_err(|_| DomainError::Conflict)?;
            let (root, path) = grants
                .roots
                .iter()
                .find_map(|root| {
                    Self::relative_to(&absolute, root.root())
                        .ok()
                        .map(|path| (root.clone(), path))
                })
                .ok_or(DomainError::PermissionDenied)?;
            (root, path, grants.revision)
        };
        let path = Self::read_path(&root, path, check)?;
        if root.directory_exists(&path)? {
            return Err(DomainError::InvalidInput(format!(
                "Path is a directory: {raw}. Use ls to inspect it, then read a specific file."
            )));
        }
        let (expected, result, streamed_read) = match root.snapshot(&path, MAX_READ) {
            Ok(snapshot) => {
                check()?;
                let result =
                    match crate::image_tools::read_with_control(&snapshot.bytes, true, check)? {
                        Some(image) => image,
                        None => ag1_core::tools::read::text(call.args(), &snapshot.bytes)?,
                    };
                (snapshot.revision, result, false)
            }
            Err(DomainError::LimitExceeded(_)) => {
                let mut window = ag1_core::tools::read::TextWindow::new(call.args())?;
                let mut first = true;
                let revision = root.stream(&path, crate::workspace::MAX_STREAM_READ, |bytes| {
                    check()?;
                    if first && crate::image_tools::supported_header(bytes) {
                        return Err(DomainError::LimitExceeded("encoded image input".into()));
                    }
                    first = false;
                    window.push(bytes)
                })?;
                (revision, window.finish()?, true)
            }
            Err(error) => return Err(error),
        };
        check()?;
        let stamp = format!(
            "resource-read-{grant_revision}-{}",
            revision_stamp(root.id().as_str(), &path, Some(&expected))
        );
        // External resources are not workspace source inspections. Their exact
        // root/revision remains in the private plan; original args/result have P05 custody.
        Ok(PreparedFile {
            workspace: self.workspace.clone(),
            scope: scope.clone(),
            call: call.clone(),
            path,
            expected: Some(expected),
            stamp,
            transformation: file::Transformation {
                bytes: None,
                result,
                create_parents: false,
            },
            preview_success: false,
            streamed_read,
            queue_permit: None,
            read_source: Some(ReadSource {
                root,
                grant_revision,
            }),
        })
    }
    fn queue(&self, path: &str) -> Result<Arc<Queue>, DomainError> {
        let mut queues = self.queues.lock().unwrap();
        queues.retain(|_, q| q.strong_count() > 0);
        if let Some(queue) = queues.get(path).and_then(Weak::upgrade) {
            return Ok(queue);
        }
        if queues.len() >= 256 {
            return Err(DomainError::QueueFull);
        }
        let queue = Arc::new(Queue::default());
        queues.insert(path.into(), Arc::downgrade(&queue));
        Ok(queue)
    }
    /// Host supplies the current parent scope/policy and clock, not values echoed by an approval client.
    /// Call from a bounded blocking worker. A successful publication remains successful after cancellation.
    pub fn execute<F: Fn() -> u64>(
        &self,
        mut prepared: PreparedFile,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<FileOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if !Arc::ptr_eq(&prepared.workspace, &self.workspace) || prepared.scope != *scope {
            return Err(DomainError::PermissionDenied);
        }
        let read_grants = prepared
            .read_source
            .as_ref()
            .map(|_| self.read_roots.read().map_err(|_| DomainError::Conflict))
            .transpose()?;
        let source_workspace = if let Some(source) = &prepared.read_source {
            let read_grants = read_grants.as_ref().ok_or(DomainError::Conflict)?;
            if prepared.call.name() != "read"
                || prepared.will_mutate()
                || prepared.preview_success
                || source.grant_revision != read_grants.revision
                || !read_grants
                    .roots
                    .iter()
                    .any(|root| Arc::ptr_eq(root, &source.root))
            {
                return Err(DomainError::PermissionDenied);
            }
            &source.root
        } else {
            &self.workspace
        };
        let _permit = if let Some(permit) = prepared.queue_permit.take() {
            if !Arc::ptr_eq(&permit.0, &self.queue(&prepared.path)?) {
                return Err(DomainError::PermissionDenied);
            }
            Some(permit)
        } else if prepared.will_mutate() || prepared.preview_success {
            Some(self.queue(&prepared.path)?.enter(cancel, deadline)?)
        } else {
            None
        };
        let call = authorization.consume(
            registry,
            ag1_core::tools::approval::ExecutionFence {
                scope,
                policy_revision,
                target_revision: &prepared.stamp,
                now: now(),
            },
        )?;
        if call.operation() != prepared.call.operation()
            || call.call_id() != prepared.call.call_id()
            || call.name() != prepared.call.name()
            || call.args_hash() != prepared.call.args_hash()
        {
            return Err(DomainError::PermissionDenied);
        }
        let check = || {
            if cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                Ok(())
            }
        };
        check()?;
        // Compare again even for a read-only preview: never record a preview for bytes replaced while approval waited.
        let current = match if prepared.streamed_read {
            source_workspace.stream(&prepared.path, crate::workspace::MAX_STREAM_READ, |_| {
                check()
            })
        } else {
            source_workspace
                .snapshot(&prepared.path, MAX_READ)
                .map(|s| s.revision)
        } {
            Ok(revision) => Some(revision),
            Err(DomainError::NotFound) => None,
            Err(e) => return Err(e),
        };
        if current != prepared.expected {
            return Err(DomainError::Conflict);
        }
        let key = (scope.session.clone(), scope.epoch, prepared.path.clone());
        let published = if let Some(bytes) = &prepared.transformation.bytes {
            let revision = match &prepared.expected {
                Some(expected) => {
                    self.workspace
                        .replace_checked(&prepared.path, expected, bytes, check)?
                }
                None => self.workspace.create_checked(
                    &prepared.path,
                    bytes,
                    prepared.transformation.create_parents,
                    check,
                )?,
            };
            self.previews.lock().unwrap().remove(&key);
            Some(revision)
        } else {
            None
        };
        if prepared.preview_success {
            let mut records = self.previews.lock().unwrap();
            if !records.contains_key(&key) && records.len() >= 256 {
                return Err(DomainError::LimitExceeded(
                    "retained previews; close the session or explicitly discard a preview".into(),
                ));
            }
            records.insert(
                key,
                PreviewRecord {
                    revision: prepared.expected.ok_or(DomainError::Conflict)?,
                },
            );
        }
        Ok(FileOutcome {
            result: prepared.transformation.result,
            published,
        })
    }
    pub fn list_target_revision(&self) -> String {
        format!("directory:{}", self.workspace.id().as_str())
    }
    /// Directory listing observes the tree after policy approval. It produces no source-inspection claims.
    pub fn execute_list<F: Fn() -> u64>(
        &self,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<FileOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if scope.workspace != self.workspace.id().as_str()
            || authorization.call().name() != "ls"
            || authorization.call().origin() != Origin::BuiltIn
        {
            return Err(DomainError::PermissionDenied);
        }
        let check = || {
            if cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                Ok(())
            }
        };
        check()?;
        let target = self.list_target_revision();
        let call = authorization.consume(
            registry,
            ag1_core::tools::approval::ExecutionFence {
                scope,
                policy_revision,
                target_revision: &target,
                now: now(),
            },
        )?;
        let input = call.args().get("path").and_then(Value::as_str);
        let path = self.relative_path(input.filter(|s| !s.is_empty()).unwrap_or("."))?;
        if !self.workspace.exists(&path)? {
            return Err(DomainError::InvalidInput(format!(
                "Path not found: {}",
                self.workspace.root().join(&path).display()
            )));
        }
        if !self.workspace.directory_exists(&path)? {
            return Err(DomainError::InvalidInput(format!(
                "Not a directory: {}",
                self.workspace.root().join(&path).display()
            )));
        }
        let limit = call
            .args()
            .get("limit")
            .map(|v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| {
                        DomainError::InvalidInput("directory limit must be an integer".into())
                    })
            })
            .transpose()?;
        let entries = self.workspace.directory_entries(&path)?;
        check()?;
        Ok(FileOutcome {
            result: ag1_core::tools::list::project(input, &entries, limit)?,
            published: None,
        })
    }
    /// Parent session teardown only; tool arguments cannot discard a stale preview fence.
    pub fn close_session(&self, session: &str) {
        self.previews
            .lock()
            .unwrap()
            .retain(|(s, _, _), _| s != session);
    }
}
fn revision_stamp(workspace: &str, path: &str, revision: Option<&FileRevision>) -> String {
    let revision = revision.map(|r| {
        serde_json::json!([
            r.sha256,
            r.device,
            r.inode,
            r.size,
            r.modified_ns.to_string(),
            r.changed_ns.to_string(),
            r.mode
        ])
    });
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::json!([workspace, path, revision])
                .to_string()
                .as_bytes()
        )
    )
}
