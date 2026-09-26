//! P14 original repair workload through the real native Host. All inputs and
//! mutations are confined to a new disposable workspace; production opens use
//! the OS registry and the ordinary preserved-runtime admission path.
use ag1_core::DomainError;
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
    llama::{Material, native::MaterialVerifier},
};
use ag1_protocol as w;
use ag1_storage::{FrozenManifest, Store};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    destination: PathBuf,
    seed: Material,
    manifest: Material,
    specs: Material,
    node: Material,
    config: Config,
    #[serde(default)]
    prepare_only: bool,
}
fn d<T>(result: std::result::Result<T, DomainError>) -> Result<T> {
    result.map_err(|e| format!("{e:?}").into())
}
fn save(path: &Path, value: &Value) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}
async fn command(host: &Arc<Host>, command: w::Command, seconds: u64) -> Result<Value> {
    let s = d(host.commands.snapshot())?;
    let reply = d(host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: d(ag1_platform::client::nonce())?.try_into()?,
            scope: s.scope,
            expected_revision: s.revision,
            command,
            extension: None,
        })
        .await)?;
    let w::BridgeFrame::Reply { status, data, .. } = reply else {
        return Err("Missing command reply".into());
    };
    if status != "accepted" {
        return Err(format!("Rejected command: {data}").into());
    }
    let operation = match serde_json::from_value::<w::CommandResult>(data)? {
        w::CommandResult::Accepted { operation } => operation,
        w::CommandResult::Settled { operation }
            if operation.status == w::OperationStatus::Completed =>
        {
            return Ok(serde_json::to_value(d(host.settled_view().await)?)?);
        }
        other => return Err(format!("Command did not succeed: {other:?}").into()),
    };
    tokio::time::timeout(Duration::from_secs(seconds), async {
        loop {
            let view = d(host.settled_view().await)?;
            if view.error.is_some() || !view.snapshot.pending.is_empty() {
                return Err(format!("Native command requires recovery/approval: {view:?}").into());
            }
            if let Some(terminal) = &d(host.commands.snapshot())?.last_terminal
                && terminal.operation == operation
            {
                if terminal.status != w::OperationStatus::Completed {
                    return Err(format!("Native operation failed: {terminal:?}").into());
                }
                return Ok(serde_json::to_value(view)?);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?
}
fn git(workspace: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("/usr/bin/git")
        .args(args)
        .current_dir(workspace)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
async fn run(input: Input) -> Result<()> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let base = repo.join(".local/p14-long-horizon").canonicalize()?;
    let out = input.destination.canonicalize()?;
    if !out.starts_with(&base) || out.join("state").exists() {
        return Err("New private native destination required".into());
    }
    let mut verifier = MaterialVerifier::default();
    d(verifier.verify(&[
        input.seed.clone(),
        input.manifest.clone(),
        input.specs.clone(),
        input.node.clone(),
    ]))?;
    let seed: Value = serde_json::from_slice(&fs::read(&input.seed.path)?)?;
    let specs: Vec<Value> = serde_json::from_slice(&fs::read(&input.specs.path)?)?;
    if specs.len() != 6 {
        return Err("All six original checkpoints required".into());
    }
    if !input.config.fixture
        && (seed["token_counts_exact"] != true
            || seed["token_count"]
                .as_u64()
                .is_none_or(|n| !(2_000_000..=3_000_000).contains(&n))
            || input.config.runtime.is_none())
    {
        return Err(
            "Live native preflight requires exact tokenized history and preserved runtime".into(),
        );
    }
    let workspace = out.join("workspace");
    let state = out.join("state");
    if !workspace.is_dir()
        || fs::read_dir(&workspace)?.next().is_some()
        || seed["workspace"] != workspace.to_string_lossy().as_ref()
    {
        return Err("Empty rebased fixture workspace required".into());
    }
    let registry = if input.config.fixture && !input.prepare_only {
        let authority = out.join("authority");
        fs::create_dir(&authority)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o700))?;
        d(Registry::at(&authority))?
    } else {
        d(Registry::for_user())?
    };
    d(registry.initialize_isolated(&workspace))?;
    fs::create_dir(workspace.join("src"))?;
    fs::create_dir(&state)?;
    for spec in &specs {
        for (name, content) in [
            ("sourcePath", "initialSource"),
            ("verification", "verificationSource"),
        ] {
            let relative = Path::new(spec[name].as_str().ok_or("Fixture path")?);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|p| !matches!(p, std::path::Component::Normal(_)))
            {
                return Err("Fixture path escapes workspace".into());
            }
            fs::write(
                workspace.join(relative),
                spec[content].as_str().ok_or("Fixture content")?,
            )?;
        }
    }
    fs::write(
        workspace.join("package.json"),
        "{\"name\":\"ag1-long-horizon-fixture\",\"private\":true,\"type\":\"module\"}\n",
    )?;
    git(&workspace, &["init", "-q"])?;
    git(
        &workspace,
        &["config", "user.email", "long-horizon@example.invalid"],
    )?;
    git(
        &workspace,
        &["config", "user.name", "Long Horizon Benchmark"],
    )?;
    git(&workspace, &["add", "-A"])?;
    git(&workspace, &["commit", "-q", "-m", "fixture"])?;
    let manifest: FrozenManifest = serde_json::from_slice(&fs::read(&input.manifest.path)?)?;
    let store = Store::open(&state.join("store"))?;
    store.import_frozen(
        Path::new(seed["source"].as_str().ok_or("Seed source")?),
        &manifest,
    )?;
    drop(store);
    if input.prepare_only {
        save(
            &out.join("prepared-state.json"),
            &json!({"status":"prepared","fixture":input.config.fixture,"release_acceptance":false,"model_actions":0,"workspace":workspace,"state":state,"session":seed["session"],"externalized_tokens":seed["token_count"],"scope":"Imported derived immutable seed only; no host/runtime admission or task work."}),
        )?;
        return Ok(());
    }
    let requests = Arc::new(Mutex::new(0usize));
    let serial = requests.clone();
    let directory = out.clone();
    let selected_model = input
        .config
        .models
        .iter()
        .find(|model| model["id"] == input.config.selected)
        .ok_or("Selected model absent")?;
    let expected_thinking = selected_model["reasoning"] == true;
    let observer = Arc::new(
        move |url: &str, bytes: &[u8]| -> std::result::Result<(), DomainError> {
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("P14 request capture".into()));
            }
            let mut count = serial.lock().map_err(|_| DomainError::DurabilityUnknown)?;
            *count += 1;
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| DomainError::InvalidInput("Request JSON".into()))?;
            save(
                &directory.join(format!("request-{:04}.json", *count)),
                &json!({"url":url,"body":value}),
            )
            .map_err(|_| DomainError::DurabilityUnknown)?;
            if expected_thinking && value["chat_template_kwargs"]["enable_thinking"] != true {
                return Err(DomainError::Conflict);
            }
            Ok(())
        },
    );
    let admission = Instant::now();
    let host = d(Host::open_with_request_observer(
        &workspace,
        &state,
        input.config.clone(),
        &registry,
        observer,
    )
    .await)?;
    let admission_seconds = admission.elapsed().as_secs_f64();
    let mut checkpoints = vec![];
    let outcome:Result<()>=async {
        let session=seed["session"].as_str().ok_or("Seed session")?;
        for (index,spec) in specs.iter().enumerate(){
            let file=spec["sourcePath"].as_str().ok_or("Source path")?;
            fs::write(workspace.join(file),spec["brokenSource"].as_str().ok_or("Regression source")?)?;
            git(&workspace,&["add",file])?;git(&workspace,&["commit","-q","-m","introduce regression"])?;
            let resumed=command(&host,w::Command::Session{action:"resume".into(),args:json!({"session":session})},60).await?;
            if resumed["snapshot"]["scope"]["session"]!=session {return Err("Wrong session after resume".into());}
            command(&host,w::Command::Runtime{action:"tools".into(),args:json!({"names":["read","grep","ls","bash","edit","write","artifact","sourceContext"]})},60).await?;
            let selected=d(host.registry.selected())?;
            let configured=d(selected.services.runtime().configuration())?;
            if configured.thinking != if expected_thinking { "low" } else { "off" } { return Err("Selected native thinking differs from workload".into()); }
            let before=d(selected.services.runtime().client_records())?.len();
            let started=Instant::now();
            let result=command(&host,w::Command::Prompt{input:w::PromptInput::Inline{text:spec["task"].as_str().ok_or("Original task")?.into()}},1200).await;
            let records=d(selected.services.runtime().client_records())?;
            save(&out.join(format!("checkpoint-{}.messages.json",index+1)),&json!(*records))?;
            let required=spec["currentContract"].as_str().ok_or("Exact contract")?;
            let recalled=records.iter().skip(before).any(|r| r["type"]=="message" && r["message"]["role"]=="toolResult" && r["message"]["toolName"]=="artifact" && r["message"]["content"].as_array().is_some_and(|blocks|blocks.iter().any(|b|b["text"].as_str().is_some_and(|s|s.contains(required)))));
            let check=std::process::Command::new(&input.node.path).arg(spec["verification"].as_str().ok_or("Verifier")?).current_dir(&workspace).output()?;
            fs::write(out.join(format!("checkpoint-{}.verifier.log",index+1)),[check.stdout.as_slice(),check.stderr.as_slice()].concat())?;
            let unchanged=specs.iter().all(|v| fs::read_to_string(workspace.join(v["verification"].as_str().unwrap())).is_ok_and(|text|text==v["verificationSource"].as_str().unwrap()));
            let changed=git(&workspace,&["status","--porcelain"])?;
            let scoped=changed.lines().all(|line|line.get(3..)==Some(file));
            let diff=git(&workspace,&["diff","--no-ext-diff","--",file])?;fs::write(out.join(format!("checkpoint-{}.diff.patch",index+1)),&diff)?;
            let context=d(selected.services.runtime().client_context())?;
            save(&out.join(format!("checkpoint-{}.context.json",index+1)),&context)?;
            let within=result.is_ok() && context["stats"]["overTargetTokens"]==0;
            let passed=result.is_ok() && recalled && check.status.success() && unchanged && scoped && within;
            let row=json!({"id":spec["id"],"passed":passed,"duration_seconds":started.elapsed().as_secs_f64(),"evidence_rehydrated":recalled,"verification_passed":check.status.success() && scoped,"verifiers_unchanged":unchanged,"active_within_target":within,"changed_files_porcelain":changed,"view":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string)});
            save(&out.join(format!("checkpoint-{}.json",index+1)),&row)?;checkpoints.push(row);
            eprintln!("P14 native checkpoint={} passed={passed}",spec["id"]);
            if !passed {return Err("Native long-horizon checkpoint failed; raw evidence retained".into());}
            git(&workspace,&["add",file])?;git(&workspace,&["commit","-q","-m","checkpoint"])?;
        }
        Ok(())
    }.await;
    let settlement = d(host.shutdown().await);
    drop(host);
    settlement?;
    let preserved = if let Some(runtime) = &input.config.runtime {
        d(runtime.capture.verify()).map(|_| ())
    } else {
        Ok(())
    };
    let intact = seed["artifacts"]
        .as_array()
        .ok_or("Seed artifacts")?
        .iter()
        .all(|a| {
            let sha = a["sha256"].as_str().unwrap_or("");
            fs::read(state.join("store/blobs").join(sha))
                .is_ok_and(|bytes| ag1_storage::digest(&bytes) == sha)
        });
    save(
        &out.join("native-result.json"),
        &json!({"status":if outcome.is_ok() && preserved.is_ok() && intact {"passed"}else{"failed"},"fixture":input.config.fixture,"release_acceptance":false,"admission_seconds":admission_seconds,"request_count":*requests.lock().unwrap(),"checkpoints":checkpoints,"runtime_settled":true,"preservation_verified":preserved.is_ok(),"all_original_artifact_hashes_intact":intact,"scope":"Six original live repair tasks through native session resume; exact current evidence, original external verifiers, mutation scope, context limits and actual outgoing request bodies retained. No duration acceptance."}),
    )?;
    outcome?;
    preserved?;
    if !intact {
        return Err("Seed artifact integrity failed".into());
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Private native workload input required")?;
    run(serde_json::from_slice(&fs::read(path)?)?).await
}
