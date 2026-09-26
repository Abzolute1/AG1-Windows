//! Measured workflow in an already migrated native session. No import, runtime
//! adoption or acceptance is inferred from this runner's success.
use super::*;
use crate::client::{Config, Host};
use ag1_protocol as w;
use serde_json::{Value, json as value};
use std::{future::Future, sync::Arc, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub workspace: PathBuf,
    pub session: String,
    pub evidence: PathBuf,
    pub input_sha256: String,
    pub read_path: String,
    pub read_sha256: String,
    pub steering_marker: String,
    pub timeout_seconds: u64,
}

struct Recorder {
    root: PathBuf,
    scope: Value,
}
impl Recorder {
    async fn operation<F>(&self, name: &str, action: F) -> Result<Value>
    where
        F: Future<Output = Result<Value>>,
    {
        let path = self.root.join(name);
        let measured = measurement::Measurement::begin(&path, &[name.into()])?;
        let result = action.await.map_err(|e| format!("{e:?}"));
        measured.finish(&result)?;
        let finish: Value =
            serde_json::from_slice(&bounded(&path.join("result.json"), 32 * 1024 * 1024)?)
                .map_err(json)?;
        let mut report = self.scope.clone();
        report["operation"] = value!(name);
        if ["compaction", "resume", "stale-client-rejection"].contains(&name) {
            report["case"] = value!(name);
        }
        report["status"] = value!(if result.is_ok() { "passed" } else { "failed" });
        report["executed"] = value!(true);
        report["clock"] = value!("monotonic");
        report["elapsed_ms"] = finish["elapsed_ms"].clone();
        report["observations"] = value!([finish["result"]]);
        report["error"] = finish["error"].clone();
        for (field, file) in [
            ("started_receipt", "started.json"),
            ("finished_receipt", "result.json"),
        ] {
            let p = path.join(file);
            report[field] = value!({"path":p,"sha256":digest(&bounded(&p,32*1024*1024)?)});
        }
        measurement::create(&path.join("observation.json"), &report)?;
        result.map_err(DomainError::Unavailable)
    }
}

fn envelope(host: &Host, command: w::Command) -> Result<w::CommandEnvelope> {
    let snapshot = host.commands.snapshot()?;
    Ok(w::CommandEnvelope {
        version: Default::default(),
        id: crate::client::nonce()?
            .try_into()
            .map_err(DomainError::InvalidInput)?,
        scope: snapshot.scope,
        expected_revision: snapshot.revision,
        command,
        extension: None,
    })
}
async fn dispatch(host: &Arc<Host>, command: w::Command) -> Result<Value> {
    let request = envelope(host, command)?;
    let reply = host.dispatch(request.clone()).await?;
    if !matches!(&reply, w::BridgeFrame::Reply { status, .. } if status == "accepted") {
        return Err(DomainError::Unavailable(format!(
            "Workflow command was not accepted: {reply:?}"
        )));
    }
    Ok(value!({"command":request,"reply":reply}))
}
async fn settled(host: &Host, seconds: u64) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        loop {
            let view = host.settled_view().await?;
            if view.error.is_some()
                || !view.snapshot.pending.is_empty()
                || matches!(
                    view.snapshot.status.as_str(),
                    "recovery_required" | "persistence_failure"
                )
            {
                return Err(DomainError::Unavailable(
                    "Workflow needs explicit recovery or approval".into(),
                ));
            }
            if view.snapshot.status == "idle" {
                if host
                    .commands
                    .snapshot()?
                    .last_terminal
                    .is_some_and(|op| op.status != w::OperationStatus::Completed)
                {
                    return Err(DomainError::Unavailable(
                        "Workflow operation did not complete".into(),
                    ));
                }
                return serde_json::to_value(view).map_err(json);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| DomainError::TimedOut)?
}

pub async fn run(request: Request) -> Result<Value> {
    execute(request, &Registry::for_user()?, None, false).await
}

pub async fn run_file(path: &Path) -> Result<Value> {
    let request = serde_json::from_slice(&bounded(path, 256 * 1024)?).map_err(json)?;
    run(request).await
}

/// Explicit fixture injection cannot produce production evidence or launch a
/// production configuration through the injected runtime.
pub async fn rehearse(
    request: Request,
    registry: &Registry,
    runtime: Option<Arc<dyn crate::provider::http::GenerationRuntime>>,
) -> Result<Value> {
    execute(request, registry, runtime, true).await
}

async fn execute(
    request: Request,
    registry: &Registry,
    runtime: Option<Arc<dyn crate::provider::http::GenerationRuntime>>,
    fixture: bool,
) -> Result<Value> {
    if request.input_sha256.len() != 64
        || !request.input_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || request.read_sha256.len() != 64
        || request.steering_marker.is_empty()
        || request.steering_marker.len() > 128
        || !request
            .steering_marker
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || !(1..=1800).contains(&request.timeout_seconds)
    {
        return Err(DomainError::InvalidInput("Workflow request bounds".into()));
    }
    let record = registry
        .inspect(&request.workspace)?
        .ok_or(DomainError::NotFound)?;
    let transfer = record.transfer.clone().ok_or(DomainError::Conflict)?;
    if record.owner != Owner::Native
        || record.stage != Stage::Active
        || !record.quiescent
        || record.isolated
        || request.session != transfer.session
        || record.state.as_ref() != Some(&transfer.destination)
    {
        return Err(DomainError::Conflict);
    }
    let config: Config = serde_json::from_slice(&bounded(
        &transfer.destination.join("host.json"),
        1024 * 1024,
    )?)
    .map_err(json)?;
    if config.fixture != fixture {
        return Err(DomainError::PermissionDenied);
    }
    let root = super::migration::new_path(&request.evidence)?;
    for path in [
        &record.workspace,
        &transfer.destination,
        &transfer.original_state,
        &transfer.archive,
    ] {
        if root.starts_with(path) || path.starts_with(&root) {
            return Err(DomainError::PermissionDenied);
        }
    }
    // The real audit checks acceptance/material custody before any workflow
    // mutation. Fixture injection is deliberately separate and stays labeled.
    let audit = registry.audit_cutover(&record.workspace)?;
    for path in [
        audit["source_snapshot"]["path"].as_str(),
        audit["runtime_acceptance"]["precommit_recovery"].as_str(),
        audit["rollback_archive"]["archive"].as_str(),
    ]
    .into_iter()
    .flatten()
    {
        let path = Path::new(path);
        if root.starts_with(path) || path.starts_with(&root) {
            return Err(DomainError::PermissionDenied);
        }
    }
    let whole = measurement::Measurement::begin(
        &root,
        &[
            "run-workflow".into(),
            record.workspace.display().to_string(),
            request.session.clone(),
        ],
    )?;
    let recorder = Recorder {
        root,
        scope: value!({"version":1,"fixture":fixture,"input_sha256":request.input_sha256,
        "selection":{"workspace":record.workspace,"session":request.session,"original_state":transfer.original_state,"destination":transfer.destination},
        "phase_acceptance":false,"automatic_retry_allowed":false}),
    };
    let open = || async {
        if let Some(runtime) = &runtime {
            Host::open_rehearsal_with_runtime(
                &record.workspace,
                &transfer.destination,
                config.clone(),
                registry,
                runtime.clone(),
            )
            .await
        } else {
            Host::open_with_registry(
                &record.workspace,
                &transfer.destination,
                config.clone(),
                registry,
            )
            .await
        }
    };
    let outcome = async {
        let host=open().await?;
        let first = async {
            if host.commands.snapshot()?.scope.session.as_str()!=request.session { return Err(DomainError::Conflict); }
            let read=host.workspace.snapshot(&request.read_path,1024*1024)?;
            if digest(&read.bytes)!=request.read_sha256 { return Err(DomainError::Conflict); }
            let before=host.store.snapshot(&request.session)?;
            let prior_tools:Vec<String>=host.registry.selected()?.services.runtime().configuration()?.tools.iter()
                .filter_map(|tool|tool["name"].as_str().map(str::to_owned)).collect();
            // The verification task admits only the read tool. This changes no
            // model, sampling or process launch parameter, and grants no approval.
            recorder.operation("select-read-tool",dispatch(&host,w::Command::Runtime { action:"tools".into(),args:value!({"names":["read"]}) })).await?;
            settled(&host,request.timeout_seconds).await?;
            let text=format!("P13 migration verification: use the read tool to read the workspace path {}. Report its contents and preserve the marker {}. Perform no other task.",serde_json::to_string(&request.read_path).map_err(json)?,request.steering_marker);
            recorder.operation("prompt-admission",dispatch(&host,w::Command::Prompt { input:w::PromptInput::Inline{text} })).await?;
            recorder.operation("generation-start",async {
                tokio::time::timeout(Duration::from_secs(request.timeout_seconds),async {
                    loop {
                        if host.registry.selected()?.services.runtime().pending_transport()?.is_some() {
                            return Ok(value!({"owned_generation_ticket_observed":true}));
                        }
                        if host.settled_view().await?.error.is_some() { return Err(DomainError::Unavailable("Generation failed before steering".into())); }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                }).await.map_err(|_|DomainError::TimedOut)?
            }).await?;
            recorder.operation("steering-admission",dispatch(&host,w::Command::Session {action:"steer".into(),args:value!({"input":{"kind":"inline","text":format!("Preserve {} in the response and summary.",request.steering_marker)}})})).await?;
            recorder.operation("prompt-settlement",settled(&host,request.timeout_seconds)).await?;
            let after=host.store.snapshot(&request.session)?;
            let new:Vec<Value>=after.entries.iter().filter(|entry|!before.entries.iter().any(|old|old.id==entry.id))
                .map(|e|serde_json::from_slice(&e.normalized).map_err(json)).collect::<Result<_>>()?;
            let calls:Vec<&Value>=new.iter().filter(|v|v["message"]["role"]=="assistant")
                .filter_map(|v|v["message"]["content"].as_array()).flatten()
                .filter(|v|v["type"]=="toolCall" && v["name"]=="read" && v["arguments"]["path"]==request.read_path).collect();
            if calls.len()!=1 || !new.iter().any(|v|v["message"]["role"]=="toolResult"
                && v["message"]["toolCallId"]==calls[0]["id"] && v["message"]["isError"]!=true)
                || !new.iter().any(|v|v["message"]["role"]=="user" && v.to_string().contains(&format!("Preserve {} in the response",request.steering_marker))) {
                return Err(DomainError::Unavailable("Missing unique read result or retained steering".into()));
            }
            recorder.operation("restore-tools",dispatch(&host,w::Command::Runtime {action:"tools".into(),args:value!({"names":prior_tools})})).await?;
            settled(&host,request.timeout_seconds).await?;
            recorder.operation("compaction",async {
                dispatch(&host,w::Command::Session {action:"compact".into(),args:value!({"focus":format!("Keep {}",request.steering_marker)})}).await?;
                settled(&host,request.timeout_seconds).await?;
                let journal=host.store.snapshot(&request.session)?;
                let rows:Vec<Value>=journal.entries.iter().filter(|e|!after.entries.iter().any(|old|old.id==e.id))
                    .map(|e|serde_json::from_slice(&e.normalized).map_err(json)).collect::<Result<_>>()?;
                if !rows.iter().any(|v|v["type"]=="compaction" && v.to_string().contains(&request.steering_marker)) {
                    return Err(DomainError::Unavailable("No new verified compaction summary".into()));
                }
                Ok(value!({"new_records":rows}))
            }).await?;
            let journal=host.store.snapshot(&request.session)?;
            if digest(&host.workspace.snapshot(&request.read_path,1024*1024)?.bytes)!=request.read_sha256 { return Err(DomainError::Conflict); }
            let stale=envelope(&host,w::Command::Prompt {input:w::PromptInput::Inline{text:"P13 stale command must never execute".into()}})?;
            for case in ["prompt","tool","steering"] {
                let mut output=recorder.scope.clone();
                output["status"]=value!("passed");output["case"]=value!(case);output["executed"]=value!(true);
                output["observations"]=value!(new);
                measurement::create(&recorder.root.join(format!("{case}.json")),&output)?;
            }
            Ok((journal,stale))
        }.await;
        let shutdown=recorder.operation("shutdown-before-resume",async { host.shutdown().await?; Ok(value!({"host_shutdown":true})) }).await;
        if shutdown.is_err() { let _=host.shutdown().await; }
        drop(host);
        let (journal,stale)=first?;
        shutdown?;
        let mut reopened=None;
        let resumed_open=recorder.operation("resume",async {
            let host=open().await?;
            reopened=Some(host.clone());
            let actual=host.store.snapshot(&request.session)?;
            if actual.entries!=journal.entries || actual.leaf!=journal.leaf { return Err(DomainError::Conflict); }
            Ok(value!({"snapshot":host.settled_view().await?,"journal_sha256":digest(&serde_json::to_vec(&actual).map_err(json)?)}))
        }).await;
        if let Err(error)=resumed_open {
            if let Some(host)=reopened { host.shutdown().await?; }
            return Err(error);
        }
        let host=reopened.ok_or(DomainError::DurabilityUnknown)?;
        let resumed=async {
            recorder.operation("stale-client-rejection",async {
                if host.dispatch(stale).await.is_ok() || host.store.snapshot(&request.session)?.entries!=journal.entries {
                    return Err(DomainError::Conflict);
                }
                Ok(value!({"stale_command_rejected":true,"journal_unchanged":true}))
            }).await?;
            recorder.operation("client-resnapshot",async { Ok(value!({"snapshot":host.settled_view().await?,"renderer_measured":false})) }).await?;
            Ok::<(),DomainError>(())
        }.await;
        let drain=recorder.operation("drain",async { host.drain_for_cutover().await?; Ok(value!({"host_drained":true})) }).await;
        if drain.is_err() { let _=host.shutdown().await; }
        drop(host);
        resumed?;drain?;
        let lease=registry.resume_transfer(&record.workspace)?;
        let (_,bundle)=lease.bundle()?;
        bundle.files.preflight(&transfer.original_state)?;
        let mut report=recorder.scope.clone();
        report["status"]=value!("passed");report["authority"]=serde_json::to_value(lease.record()?).map_err(json)?;
        report["renderer_acceptance"]=value!("pending_actual_renderer");
        report["next_boundary"]=value!("quiescent native recovery/rollback; no automatic retry");
        Ok::<Value,DomainError>(report)
    }.await.map_err(|e|format!("{e:?}"));
    whole.finish(&outcome)?;
    outcome.map_err(DomainError::Unavailable)
}
