//! Source capsules use captured retrieval signals and the active journal owner.
use crate::{
    file_tools::FileExecution,
    git::GitService,
    retrieval::SourceHost,
    session::tool_journal::{JournalMutationError, ToolJournal},
    source::SourceService,
};
use ag1_core::{
    DomainError, SessionId,
    context::{
        ContextArtifactPort,
        capsule::{self, CapsuleRecord},
        retrieval::{self, RecentAccess, RetrievalOptions, SourceRetrievalPort, WorkingTree},
        signals::{self, GitOutput},
    },
    evidence::{Freshness, SourceSnapshot},
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Scope, ValidatedCall},
    },
};
use ag1_storage::{Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub struct SourceTools {
    host: SourceHost,
    git: GitService,
    store: Arc<Store>,
    origin: String,
}
pub struct PreparedSource {
    call: ValidatedCall,
    target: String,
    journal: Arc<ToolJournal>,
}
impl PreparedSource {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    pub fn may_mutate(&self) -> bool {
        !matches!(self.call.args()["action"].as_str(), Some("list" | "status"))
    }
}
fn result(text: String, details: Value, effect: Effect) -> AdapterOutcome {
    AdapterOutcome {
        result: json!({"content":[{"type":"text","text":text}],"details":details}),
        effect,
        artifact_page: false,
    }
}
fn rejected(error: DomainError, effect: Effect) -> AdapterOutcome {
    let mut output = result(
        format!("Source context failed: {error:?}"),
        json!({}),
        effect,
    );
    output.result["isError"] = json!(true);
    output
}
struct Health {
    status: Freshness,
    available: bool,
    files: Vec<(String, Freshness)>,
}
fn health(
    record: &CapsuleRecord,
    source: &mut SourceService,
    artifacts: &impl ContextArtifactPort,
) -> Health {
    let available = artifacts
        .get(&record.artifact_id)
        .is_ok_and(|a| a.kind == "source-capsule");
    let files: Vec<_> = record
        .files
        .iter()
        .map(|file| (file.path.clone(), source.validate(&file.reference).status))
        .collect();
    let status = retrieval::capsule_status(&record.files, available, |reference| {
        record
            .files
            .iter()
            .position(|file| file.reference == reference)
            .map(|i| files[i].1)
            .unwrap_or(Freshness::Invalid)
    });
    Health {
        status,
        available,
        files,
    }
}
impl SourceTools {
    pub fn new(host: SourceHost, git: GitService, store: Arc<Store>, origin: String) -> Self {
        Self {
            host,
            git,
            store,
            origin,
        }
    }
    fn scope(&self, scope: &Scope, call: &ValidatedCall) -> Result<(), DomainError> {
        if scope.workspace != self.host.workspace.id().as_str()
            || call.name() != "sourceContext"
            || call.origin() != Origin::BuiltIn
        {
            Err(DomainError::PermissionDenied)
        } else {
            Ok(())
        }
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        journal: Option<&Arc<ToolJournal>>,
    ) -> Result<PreparedSource, DomainError> {
        self.scope(scope, call)?;
        let journal = journal
            .ok_or_else(|| {
                DomainError::Unavailable("Source context requires the session journal owner".into())
            })?
            .clone();
        journal.snapshot(scope)?;
        Ok(PreparedSource {
            call: call.clone(),
            target: format!(
                "source:{}:{}:{}:{}",
                scope.workspace, scope.session, scope.epoch, scope.revision
            ),
            journal,
        })
    }
    fn tree(&self, cancel: &AtomicBool, deadline: Instant) -> Result<WorkingTree, DomainError> {
        let run = |args: &[&str]| {
            self.git.tool_command(
                &self.host.workspace,
                &args.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
                cancel,
                deadline,
                8 * 1024 * 1024,
            )
        };
        let inside = run(&["rev-parse", "--is-inside-work-tree"])?;
        if !inside.1 || inside.0.trim() != "true" {
            return Ok(WorkingTree::default());
        }
        let tracked = run(&[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "-z",
            "HEAD",
            "--",
        ])?;
        let untracked = run(&[
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ])?;
        let diff = run(&[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=0",
            "--no-color",
            "HEAD",
            "--",
        ])?;
        fn output(pair: &(String, bool)) -> GitOutput<'_> {
            GitOutput {
                stdout: pair.0.as_str(),
                code: i32::from(!pair.1),
            }
        }
        signals::working_tree(
            output(&inside),
            output(&tracked),
            output(&untracked),
            output(&diff),
        )
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedSource,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<AdapterOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        let check = || {
            if cancel.load(Ordering::Acquire) || self.host.cancelled() {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                self.host.workspace.verify_root()
            }
        };
        let prepared = (|| {
            self.scope(scope, authorization.call())?;
            check()?;
            let call = authorization.consume(
                registry,
                ExecutionFence {
                    scope,
                    policy_revision,
                    target_revision: &plan.target,
                    now: now(),
                },
            )?;
            if call.args_hash() != plan.call.args_hash()
                || call.operation() != plan.call.operation()
                || call.call_id() != plan.call.call_id()
            {
                return Err(DomainError::Conflict);
            }
            let branch = plan.journal.snapshot(scope)?;
            let artifacts = ContextStore::new(
                self.store.as_ref(),
                SessionId::new(&scope.session)?,
                &self.origin,
                call.operation(),
                plan.journal.timestamp(),
                Default::default(),
            )?;
            Ok((call, branch, artifacts))
        })();
        let (call, branch, mut artifacts) = match prepared {
            Ok(p) => p,
            Err(error) => return Ok(rejected(error, Effect::None)),
        };
        let args = call.args();
        let action = args["action"].as_str().unwrap_or("build");
        let mut source = SourceService::new(self.host.workspace.clone());
        if matches!(action, "list" | "status") {
            let output = self.read(action, args, &branch, &mut source, &artifacts);
            return Ok(
                match output.and_then(|output| {
                    check()?;
                    plan.journal.snapshot(scope)?;
                    Ok(output)
                }) {
                    Ok(output) => output,
                    Err(error) => rejected(error, Effect::None),
                },
            );
        }
        let draft = (|| {
            let mut input = args.clone();
            for key in ["action", "limit", "capsuleId"] {
                input
                    .as_object_mut()
                    .ok_or(DomainError::Conflict)?
                    .remove(key);
            }
            let options: RetrievalOptions = serde_json::from_value(input)
                .map_err(|_| DomainError::InvalidInput("source context input".into()))?;
            let recent = signals::recent(
                self.host.workspace.root(),
                &branch,
                Some(call.call_id()),
                |path| {
                    if self.host.workspace.directory_exists(path).ok()? {
                        Some(false)
                    } else if self.host.workspace.exists(path).ok()? {
                        Some(true)
                    } else {
                        None
                    }
                },
            );
            let port = CapturedSource {
                host: &self.host,
                tree: self.tree(cancel, deadline)?,
                recent,
                check: &check,
            };
            let draft = retrieval::build_capsule(&options, &port)?;
            check()?;
            plan.journal.snapshot(scope)?;
            Ok((options, draft))
        })();
        let (options, draft) = match draft {
            Ok(p) => p,
            Err(error) => return Ok(rejected(error, Effect::None)),
        };
        // Artifact custody acknowledges before the sole session owner publishes metadata.
        let (record, created) = capsule::prepare_record(
            &branch,
            &mut artifacts,
            plan.journal.timestamp(),
            call.call_id(),
            &draft,
        )?;
        let effect = if created {
            Effect::Committed
        } else {
            Effect::None
        };
        if let Err(error) = check() {
            return Ok(rejected(error, effect));
        }
        if created {
            match plan
                .journal
                .commit_state(scope, &call, "context_capsule", json!(record))
            {
                Ok(()) => (),
                Err(JournalMutationError::BeforeEffect(error)) => {
                    return Ok(rejected(error, effect));
                }
                Err(JournalMutationError::Persistence(error)) => return Err(error),
            }
        }
        let mut body = format!(
            "{}\ncapsule_artifact: {}\npersistence: exact capsule saved; use artifact read/grep to rehydrate it",
            draft.body, record.artifact_id
        );
        let status = health(&record, &mut source, &artifacts).status;
        let mut details = json!({"action":action,"capsuleId":draft.id,"query":draft.query,"files":draft.files,"truncated":draft.truncated,"artifactId":record.artifact_id,"persisted":true,"status":status,"retrieval":draft.retrieval});
        if action == "rehydrate" {
            let terms: Vec<_> = retrieval::query_terms(&options)
                .iter()
                .map(|s| s.to_lowercase())
                .collect();
            let historical: Vec<_> = capsule::list(&branch, 50).into_iter().filter(|c| {
                let haystack = format!("{} {}", c.query, c.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join(" ")).to_lowercase();
                c.id != record.id && (terms.is_empty() || terms.iter().any(|t| haystack.contains(t)))
            }).take(5).map(|c| json!({"id":c.id,"artifactId":c.artifact_id,"status":health(&c, &mut source, &artifacts).status,"query":c.query})).collect();
            body.push_str("\nHISTORICAL CAPSULE POINTERS — NON-AUTHORITATIVE\n");
            body.push_str(&if historical.is_empty() {
                "- No related historical capsules found.".into()
            } else {
                historical
                    .iter()
                    .map(|c| {
                        format!(
                            "- {} {} artifact:{} — {}",
                            c["id"].as_str().unwrap(),
                            c["status"].as_str().unwrap().to_uppercase(),
                            c["artifactId"].as_str().unwrap(),
                            c["query"].as_str().unwrap()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            body.push_str("\nUse artifact read/grep only when historical exact evidence is needed; rebuild current source after edits.");
            details["historicalCapsules"] = json!(historical);
        }
        if status != Freshness::Current {
            body.push_str("\nSaved source changed during retrieval. Treat these slices as historical evidence and rebuild before relying on them.");
        }
        details["chars"] = json!(body.encode_utf16().count());
        Ok(result(body, details, effect))
    }
    fn read(
        &self,
        action: &str,
        args: &Value,
        branch: &[Value],
        source: &mut SourceService,
        artifacts: &impl ContextArtifactPort,
    ) -> Result<AdapterOutcome, DomainError> {
        if action == "list" {
            let capsules = capsule::list(branch, args["limit"].as_u64().unwrap_or(20) as usize);
            let body = if capsules.is_empty() {
                "No saved context capsules in this session.".into()
            } else {
                let mut lines = vec![format!("SAVED CONTEXT CAPSULES ({})", capsules.len())];
                for capsule in &capsules {
                    let health = health(capsule, source, artifacts);
                    lines.push(format!(
                        "{}  {}  {} files  artifact:{} ({})",
                        capsule.id,
                        health.status,
                        capsule.files.len(),
                        capsule.artifact_id,
                        if health.available {
                            "available"
                        } else {
                            "missing"
                        }
                    ));
                    lines.push(format!("  {}", capsule.query));
                }
                lines.push("Use sourceContext action=status with capsuleId before relying on old code, then artifact read to rehydrate exact source slices.".into());
                lines.join("\n")
            };
            return Ok(result(
                body,
                json!({"action":action,"capsuleId":"","query":"","files":[],"chars":0,"truncated":false,"count":capsules.len()}),
                Effect::None,
            ));
        }
        let id = args["capsuleId"].as_str().unwrap_or("").trim();
        let capsule = capsule::find(branch, id).ok_or(DomainError::NotFound)?;
        let health = health(&capsule, source, artifacts);
        let mut lines = vec![
            "CONTEXT CAPSULE STATUS".into(),
            format!("capsule_id: {}", capsule.id),
            format!("status: {}", health.status),
            format!("reason_loaded: {}", capsule.query),
            format!(
                "artifact: {} ({})",
                capsule.artifact_id,
                if health.available {
                    "available"
                } else {
                    "missing"
                }
            ),
        ];
        lines.extend(
            health
                .files
                .iter()
                .map(|(path, status)| format!("- {path}: {status}")),
        );
        lines.push(if health.status == Freshness::Current { "Exact saved slices are available through artifact read/grep." } else { "Do not treat the saved source as current. Rebuild the capsule; use the artifact only as historical evidence." }.into());
        Ok(result(
            lines.join("\n"),
            json!({"action":action,"capsuleId":capsule.id,"query":capsule.query,"files":capsule.files,"chars":capsule.chars,"truncated":capsule.truncated,"artifactId":capsule.artifact_id,"persisted":true,"status":health.status}),
            Effect::None,
        ))
    }
}
struct CapturedSource<'a> {
    host: &'a SourceHost,
    tree: WorkingTree,
    recent: Vec<RecentAccess>,
    check: &'a dyn Fn() -> Result<(), DomainError>,
}
impl SourceRetrievalPort for CapturedSource<'_> {
    fn capture(&self, path: &str) -> Result<SourceSnapshot, DomainError> {
        (self.check)()?;
        self.host.capture(path)
    }
    fn lexical(&self, terms: &[String]) -> Result<BTreeMap<String, BTreeSet<usize>>, DomainError> {
        self.host.lexical_checked(terms, &|| (self.check)())
    }
    fn working_tree(&self) -> Result<WorkingTree, DomainError> {
        (self.check)()?;
        Ok(self.tree.clone())
    }
    fn recent_accesses(&self) -> Result<Vec<RecentAccess>, DomainError> {
        (self.check)()?;
        Ok(self.recent.clone())
    }
    fn cancelled(&self) -> bool {
        (self.check)().is_err()
    }
}
