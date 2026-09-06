//! Native owned rg/fd transports. Single-file grep searches the captured bytes on stdin.
//! Directory searches make no whole-tree inspection claim; every displayed match is checked
//! against a capability-read source, and the pinned search directory is revalidated.
use crate::{
    file_tools::{FileExecution, FileOutcome, FileService},
    process::{ProcessSpec, ProcessSupervisor},
    workspace::{MAX_READ, Workspace},
};
use ag1_core::{
    DomainError,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        registry::Origin,
        search::{self, GrepProjection, Match},
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
struct SearchSpool(PathBuf);
impl Drop for SearchSpool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub struct SearchService {
    workspace: Arc<Workspace>,
    processes: Arc<ProcessSupervisor>,
    rg: PathBuf,
    fd: PathBuf,
    spools: PathBuf,
}
fn integer(
    args: &Value,
    key: &str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, DomainError> {
    let n = args
        .get(key)
        .map(|v| {
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| DomainError::InvalidInput(format!("integer {key}")))
        })
        .transpose()?
        .unwrap_or(default);
    if n < minimum || n > maximum {
        return Err(DomainError::LimitExceeded(format!("search {key}")));
    }
    Ok(n)
}
fn bounded_text<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, DomainError> {
    let text = args
        .get(key)
        .map(|v| {
            v.as_str()
                .ok_or_else(|| DomainError::InvalidInput(key.into()))
        })
        .transpose()?;
    if text.is_some_and(|s| s.len() > 4096 || s.contains('\0')) {
        return Err(DomainError::LimitExceeded(format!("search {key}")));
    }
    Ok(text)
}
impl SearchService {
    /// All paths are host configuration, never model or renderer supplied executables.
    pub fn new(
        workspace: Arc<Workspace>,
        processes: Arc<ProcessSupervisor>,
        rg: &Path,
        fd: &Path,
        spools: &Path,
    ) -> Result<Self, DomainError> {
        if !rg.is_absolute() || !fd.is_absolute() || !spools.is_absolute() || !spools.is_dir() {
            return Err(DomainError::InvalidInput("host search paths".into()));
        }
        Ok(Self {
            workspace,
            processes,
            rg: rg.into(),
            fd: fd.into(),
            spools: spools.into(),
        })
    }
    pub fn target_revision(&self) -> String {
        format!("search:{}", self.workspace.id().as_str())
    }
    /// User-facing recovery accompanies the typed rejection at the executor
    /// boundary. It cannot authorize a retry or inspect the rejected destination.
    pub fn rejection_hint(
        &self,
        reason: &DomainError,
        path: &str,
        check: &mut impl FnMut() -> Result<(), DomainError>,
    ) -> Result<Option<String>, DomainError> {
        Ok(FileService::new(self.workspace.clone()).rejection_hint(reason, path, check)?
            .map(|hint| format!("{hint} Retry without path or use a workspace-relative path like ., scripts, src, or packages/*/src.")))
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<FileOutcome, DomainError> {
        self.execute_with_barrier(authorization, execution, || {})
    }
    /// Host test seam immediately after the owned search exits, before source admission.
    pub fn execute_with_barrier<F: Fn() -> u64>(
        &self,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
        after_search: impl FnOnce(),
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
            || authorization.call().origin() != Origin::BuiltIn
            || !["grep", "find"].contains(&authorization.call().name())
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
        let target = self.target_revision();
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &target,
                now: now(),
            },
        )?;
        let args = call.args();
        let pattern = bounded_text(args, "pattern")?
            .ok_or_else(|| DomainError::InvalidInput("search pattern".into()))?;
        let paths = FileService::new(self.workspace.clone());
        let path = paths.relative_path(
            bounded_text(args, "path")?
                .filter(|s| !s.is_empty())
                .unwrap_or("."),
        )?;
        let directory = self.workspace.directory_exists(&path)?;
        let grep = call.name() == "grep";
        if !directory && !grep {
            return Err(DomainError::InvalidInput(
                "find requires a directory".into(),
            ));
        }
        let source = if directory {
            None
        } else {
            Some(self.workspace.snapshot(&path, MAX_READ)?)
        };
        let parent = if directory {
            path.as_str()
        } else {
            path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
        };
        let pinned = self.workspace.pin_directory(parent)?;
        let limit = integer(args, "limit", if grep { 100 } else { 1000 }, 1, 10000)?;
        let context = integer(args, "context", 0, 0, 100)?;
        let literal = args["literal"].as_bool().unwrap_or(false);
        let ignore_case = args["ignoreCase"].as_bool().unwrap_or(false);
        let mut argv: Vec<String> = if grep {
            [
                "--json",
                "--line-number",
                "--color=never",
                "--hidden",
                "--no-config",
            ]
            .map(str::to_string)
            .into()
        } else {
            [
                "--glob",
                "--color=never",
                "--hidden",
                "--print0",
                "--max-results",
                &limit.to_string(),
            ]
            .map(str::to_string)
            .into()
        };
        if grep {
            if literal {
                argv.push("--fixed-strings".into());
            }
            if ignore_case {
                argv.push("--ignore-case".into());
            }
            if let Some(glob) = bounded_text(args, "glob")? {
                argv.extend(["--glob".into(), glob.into()]);
            }
        } else {
            // The reference adds nested ignore files explicitly, including outside Git repos.
            let mut pending = vec![path.clone()];
            let mut seen = 0;
            while let Some(dir) = pending.pop() {
                check()?;
                for entry in self.workspace.directory_entries(&dir)? {
                    seen += 1;
                    if seen > 100000 || pending.len() > 10000 {
                        return Err(DomainError::LimitExceeded("find ignore discovery".into()));
                    }
                    let child = if dir.is_empty() {
                        entry.name.clone()
                    } else {
                        format!("{dir}/{}", entry.name)
                    };
                    if child.split('/').count() > 32 {
                        return Err(DomainError::LimitExceeded("find directory depth".into()));
                    }
                    if entry.directory
                        && !entry.symlink
                        && !["node_modules", ".git"].contains(&entry.name.as_str())
                    {
                        pending.push(child.clone());
                    }
                    if entry.name == ".gitignore" && !entry.directory {
                        if entry.symlink {
                            return Err(DomainError::PermissionDenied);
                        }
                        if argv.len() >= 245 {
                            return Err(DomainError::LimitExceeded("find ignore files".into()));
                        }
                        let relative = child.strip_prefix(&format!("{path}/")).unwrap_or(&child);
                        argv.extend(["--ignore-file".into(), format!("./{relative}")]);
                    }
                }
            }
        }
        argv.extend([
            "--".into(),
            pattern.into(),
            if source.is_some() { "-" } else { "." }.into(),
        ]);
        let spec = ProcessSpec {
            executable: if grep {
                self.rg.clone()
            } else {
                self.fd.clone()
            },
            argv,
            cwd: pinned.process_cwd(),
            env: BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("LC_ALL".into(), "C.UTF-8".into()),
                ("RIPGREP_CONFIG_PATH".into(), "/dev/null".into()),
            ]),
            timeout_ms: deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .clamp(1, 86_400_000) as u64,
            grace_ms: 100,
            resident: false,
        };
        check()?;
        pinned.verify_identity()?;
        let spool = self.spools.join(format!(
            "search-{}-{}.stderr",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut child = self
            .processes
            .spawn_protocol_limited(spec, &spool, 1024 * 1024 * 1024)?;
        let _spool = SearchSpool(spool);
        let input = source.as_ref().map(|s| s.bytes.as_slice()).unwrap_or(&[]);
        let mut sent = 0;
        let mut closed = false;
        let mut pending = vec![];
        let mut total = 0usize;
        let mut matches = vec![];
        let mut found = vec![];
        let mut whole = false;
        let mut stopped_at_limit = false;
        let status;
        loop {
            check()?;
            if !closed {
                if sent < input.len() {
                    if let Some(count) =
                        child.try_write_input(&input[sent..(sent + 32768).min(input.len())])?
                    {
                        sent += count;
                    }
                } else {
                    child.close_input();
                    closed = true;
                }
            }
            let bytes = child.read_stdout(32768)?;
            let empty = bytes.is_empty();
            total += bytes.len();
            if total > 32 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("search transport".into()));
            }
            pending.extend(bytes);
            let delimiter = if grep { b'\n' } else { 0 };
            let mut consumed = 0;
            for end in pending
                .iter()
                .enumerate()
                .filter_map(|(i, b)| (*b == delimiter).then_some(i))
            {
                let row = &pending[consumed..end];
                consumed = end + 1;
                if grep {
                    let event: Value = serde_json::from_slice(row)
                        .map_err(|_| DomainError::InvalidInput("rg JSON".into()))?;
                    if event["type"] == "match" {
                        let data = &event["data"];
                        let reported = data["path"]["text"].as_str().ok_or_else(|| {
                            DomainError::InvalidInput("non-UTF8 search path".into())
                        })?;
                        let logical = if source.is_some() {
                            path.clone()
                        } else {
                            join_result(&path, reported)?
                        };
                        let line = data["line_number"]
                            .as_u64()
                            .and_then(|v| usize::try_from(v).ok())
                            .filter(|v| *v > 0)
                            .ok_or_else(|| DomainError::InvalidInput("rg line number".into()))?;
                        let text = data["lines"]["text"]
                            .as_str()
                            .ok_or_else(|| DomainError::InvalidInput("binary search match".into()))?
                            .trim_end_matches('\n')
                            .to_string();
                        matches.push(Match {
                            path: logical,
                            line,
                            text,
                        });
                        if matches.len() >= limit {
                            stopped_at_limit = true;
                            break;
                        }
                    } else if event["type"] == "summary" {
                        let stats = &event["data"]["stats"];
                        whole = source.is_some()
                            && stats["searches"] == 1
                            && stats["bytes_searched"].as_u64() == Some(input.len() as u64);
                    }
                } else {
                    let reported = std::str::from_utf8(row)
                        .map_err(|_| DomainError::InvalidInput("non-UTF8 find path".into()))?;
                    found.push(join_result(&path, reported)?);
                    if found.len() > 10000 {
                        return Err(DomainError::LimitExceeded("find results".into()));
                    }
                }
            }
            pending.drain(..consumed);
            if pending.len() > 16 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("search record".into()));
            }
            if stopped_at_limit {
                child.stop()?;
                status = None;
                break;
            }
            if empty && let Some(exit) = child.try_wait()? {
                status = Some(exit);
                break;
            }
            if empty {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if !stopped_at_limit && !pending.is_empty() {
            return Err(DomainError::InvalidInput(
                "truncated search protocol".into(),
            ));
        }
        after_search();
        check()?;
        pinned.verify_identity()?;
        if let Some(status) = status
            && !status.success()
            && !(grep && status.code() == Some(1))
        {
            return Err(DomainError::InvalidInput(
                String::from_utf8_lossy(&child.tail(65536)?.bytes)
                    .trim()
                    .to_string(),
            ));
        }
        if let Some(before) = &source
            && self.workspace.snapshot(&path, MAX_READ)?.revision != before.revision
        {
            return Err(DomainError::Conflict);
        }
        let result = if grep {
            let mut files = BTreeMap::new();
            let mut captured = 0;
            for hit in &matches {
                if files.contains_key(&hit.path) {
                    continue;
                }
                let bytes = if let Some(source) = &source {
                    source.bytes.clone()
                } else {
                    self.workspace.snapshot(&hit.path, MAX_READ)?.bytes
                };
                captured += bytes.len();
                if captured > 32 * 1024 * 1024 {
                    return Err(DomainError::LimitExceeded("search sources".into()));
                }
                files.insert(
                    hit.path.clone(),
                    String::from_utf8(bytes)
                        .map_err(|_| DomainError::InvalidInput("binary search source".into()))?,
                );
            }
            // Validate every rg line against the captured source without quadratic whole-file rescans.
            let mut expected: BTreeMap<&str, BTreeMap<usize, &str>> = BTreeMap::new();
            for hit in &matches {
                expected
                    .entry(&hit.path)
                    .or_default()
                    .insert(hit.line, &hit.text);
            }
            for (path, rows) in expected {
                let source = files.get(path).ok_or(DomainError::Conflict)?;
                let mut checked = 0;
                for (i, line) in source.split('\n').enumerate() {
                    if let Some(expected) = rows.get(&(i + 1)) {
                        if line != *expected {
                            return Err(DomainError::Conflict);
                        }
                        checked += 1;
                    }
                }
                if checked != rows.len() {
                    return Err(DomainError::Conflict);
                }
            }
            let mut result = search::grep_projection(
                GrepProjection {
                    pattern,
                    search_dir: &path,
                    directory,
                    literal,
                    context,
                    limit,
                },
                &matches,
                &files,
            )?;
            if matches.is_empty()
                && whole
                && pattern.len() <= 1000
                && status.is_some_and(|s| s.code() == Some(1))
                && let Some(source) = source
            {
                let source = ag1_core::evidence::SourceSnapshot::new(&path, source)?;
                result["details"]["inspectionEvidence"] = json!({"version":1,"kind":"no_match","source":source.reference().reference(),"toolCallId":call.call_id(),"scope":{"pattern":pattern,"literal":literal,"ignoreCase":ignore_case}});
            }
            result
        } else {
            for path in &found {
                if !self.workspace.exists(path.trim_end_matches('/'))? {
                    return Err(DomainError::Conflict);
                }
            }
            search::find_projection(
                pattern,
                self.workspace.root().join(&path).to_str(),
                &found,
                limit,
            )?
        };
        check()?;
        self.workspace.verify_root()?;
        Ok(FileOutcome {
            result,
            published: None,
        })
    }
}
fn join_result(base: &str, reported: &str) -> Result<String, DomainError> {
    let path = reported.strip_prefix("./").unwrap_or(reported);
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\n', '\r', '\0'])
        || path
            .trim_end_matches('/')
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(DomainError::PermissionDenied);
    }
    Ok(if base.is_empty() {
        path.into()
    } else {
        format!("{base}/{path}")
    })
}
