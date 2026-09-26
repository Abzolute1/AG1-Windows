//! P14 frozen-prompt quality replay. The native owner alone admits generation.
//! This does not mint cutover acceptance or mutate a user session.
use ag1_core::DomainError;
use ag1_platform::{
    cutover::{
        Registry,
        canary::{
            Driver, Request, Runner,
            native::{LiveRequest, TransportDriver},
        },
    },
    llama::{
        LaunchPlan,
        custody::RuntimeBundle,
        lifecycle::Observation,
        native::{MaterialVerifier, NativeRuntimeHost},
    },
    provider::http::ProviderHttp,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Observer = Box<dyn FnMut(&LaunchPlan) -> std::result::Result<Observation, DomainError> + Send>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    context: Value,
    options: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    live: LiveRequest,
    cases: Vec<Case>,
    #[serde(default)]
    diagnostic: Option<Diagnostic>,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Diagnostic {
    model: String,
    repetitions: u8,
}
#[derive(Default)]
struct Wire {
    bytes: Vec<u8>,
    chunk_lengths: Vec<usize>,
    eof: bool,
}

fn domain<T>(result: std::result::Result<T, DomainError>) -> Result<T> {
    result.map_err(|e| format!("{e:?}").into())
}

fn validate(cases: &[Case]) -> Result<()> {
    let ids: std::collections::BTreeSet<_> = cases.iter().map(|c| &c.id).collect();
    if cases.is_empty() || cases.len() > 64 || ids.len() != cases.len() {
        return Err("Missing, duplicate or excessive quality cases".into());
    }
    for case in cases {
        if case.id.is_empty()
            || case.id.len() > 128
            || !case.context["messages"].is_array()
            || !case.context["systemPrompt"].is_string()
            || case.context.get("tools").is_some()
            || serde_json::to_vec(&case.context)?.len() > 2 * 1024 * 1024
            || !(256..=16384).contains(&case.options["maxTokens"].as_u64().unwrap_or(0))
        {
            return Err(
                "Invalid frozen quality request; no tools or endpoint override allowed".into(),
            );
        }
        if case
            .options
            .as_object()
            .is_none_or(|v| v.keys().any(|k| k != "maxTokens"))
        {
            return Err("Quality replay cannot change captured sampling or model settings".into());
        }
    }
    Ok(())
}

async fn execute(input: Input) -> Result<()> {
    validate(&input.cases)?;
    let live = input.live;
    if let Some(d) = &input.diagnostic
        && (!(1..=8).contains(&d.repetitions) || !live.models.iter().any(|m| m["id"] == d.model))
    {
        return Err("Invalid predeclared diagnostic selection".into());
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?
        .join(".local/p14-quality");
    let parent = live
        .destination
        .parent()
        .ok_or("Missing destination parent")?
        .canonicalize()?;
    if !parent.starts_with(base.canonicalize()?) || live.destination.exists() {
        return Err(
            "Quality evidence needs a new private destination under .local/p14-quality".into(),
        );
    }
    let registry = domain(Registry::for_user())?;
    let lease = domain(registry.runtime_lease())?;
    domain(ag1_platform::llama::window::verify(None))?;
    let plans = domain(live.capture.verify())?;
    if plans.len() != 3 || live.models.len() != 3 {
        return Err("All three preserved models are required".into());
    }
    for plan in &plans {
        let model = live
            .models
            .iter()
            .find(|m| m["id"] == plan.model_id)
            .ok_or("Missing model")?;
        if model["provider"] != "llamacpp"
            || model["api"] != "openai-completions"
            || model["contextWindow"].as_u64() != Some(plan.requested_context)
            || model["maxTokens"].as_u64() != Some(plan.max_tokens)
            || domain(ag1_platform::provider::http::endpoint_origin(
                model["baseUrl"].as_str().ok_or("Missing origin")?,
            ))? != domain(ag1_platform::llama::observation::origin(plan))?
        {
            return Err("Quality model differs from preserved plan".into());
        }
    }
    domain(MaterialVerifier::default().verify(std::slice::from_ref(&live.process_host)))?;
    fs::create_dir(&live.destination)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&live.destination, fs::Permissions::from_mode(0o700))?;
    let handle = tokio::runtime::Handle::current();
    let observer: Observer = Box::new(move |plan| {
        let origin = ag1_platform::llama::observation::origin(plan)?;
        handle.block_on(async {
            let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(2))?;
            ag1_platform::llama::observation::observe(&http, &origin).await
        })
    });
    let host = domain(NativeRuntimeHost::new(
        Path::new(&live.process_host.path),
        &live.destination,
        observer,
    ))?
    .require_exclusive_gpu();
    let runtime = Arc::new(domain(RuntimeBundle::new(
        host,
        plans,
        lease,
        Duration::from_secs(900),
    ))?);
    let mut driver = domain(TransportDriver::new(
        runtime.clone(),
        live.reference,
        live.process_host,
        &live.destination,
    ))?;
    let wire = Arc::new(Mutex::new(Wire::default()));
    if input.diagnostic.is_some() {
        let captured = wire.clone();
        driver = driver.with_response_observer(Arc::new(move |chunk| {
            let mut state = captured
                .lock()
                .map_err(|_| DomainError::DurabilityUnknown)?;
            if let Some(bytes) = chunk {
                if bytes.len() > (8 * 1024 * 1024usize).saturating_sub(state.bytes.len()) {
                    return Err(DomainError::LimitExceeded(
                        "diagnostic response capture".into(),
                    ));
                }
                state.chunk_lengths.push(bytes.len());
                state.bytes.extend_from_slice(bytes);
            } else {
                state.eof = true;
            }
            Ok(())
        }));
    }
    let mut raw = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(live.destination.join("rows.jsonl"))?;
    let mut count = 0;
    let outcome: Result<()> = async {
        for model in &live.models {
            if input.diagnostic.as_ref().is_some_and(|d| model["id"] != d.model) { continue; }
            let repetitions = input.diagnostic.as_ref().map_or(1, |d| d.repetitions);
            for trial in 0..repetitions {
            let runners = if trial % 2 == 0 { [Runner::Typescript, Runner::Rust] }
                          else { [Runner::Rust, Runner::Typescript] };
            for runner in runners {
                domain(driver.reset().await)?;
                for case in &input.cases {
                    let request = Request {
                        case: case.id.clone(),
                        model: model.clone(),
                        context: case.context.clone(),
                        options: case.options.clone(),
                        cancel_after_first_content: false,
                    };
                    *wire.lock().map_err(|_| "Capture mutex poisoned")? = Wire::default();
                    let result = driver.run(runner, &request).await;
                    if input.diagnostic.is_some() && matches!(runner, Runner::Rust) {
                        let captured = wire.lock().map_err(|_| "Capture mutex poisoned")?;
                        let stem = format!("wire-row-{}", count + 1);
                        let mut file = fs::OpenOptions::new().create_new(true).write(true)
                            .open(live.destination.join(format!("{stem}.sse")))?;
                        file.write_all(&captured.bytes)?; file.sync_all()?;
                        let mut metadata = fs::OpenOptions::new().create_new(true).write(true)
                            .open(live.destination.join(format!("{stem}.json")))?;
                        serde_json::to_writer_pretty(&mut metadata, &json!({"trial":trial,
                            "case":case.id,"model":model["id"],"eof":captured.eof,
                            "chunk_lengths":captured.chunk_lengths,"bytes":captured.bytes.len(),
                            "generation_error":result.as_ref().err().map(|e|format!("{e:?}"))}))?;
                        metadata.sync_all()?;
                    }
                    let answer = domain(result)?;
                    let row = json!({"model":model["id"],"runner":runner,"case":case.id,"trial":trial,"answer":answer});
                    serde_json::to_writer(&mut raw, &row)?;
                    writeln!(raw)?;
                    raw.sync_all()?;
                    count += 1;
                    println!(
                        "completed quality row {count}: {} {runner:?} {}",
                        model["id"], case.id
                    );
                }
            }
            }
        }
        Ok(())
    }
    .await;
    let settlement = domain(driver.reset().await);
    let verified = domain(live.capture.verify());
    let report = json!({"status":if outcome.is_ok() && settlement.is_ok() && verified.is_ok() {"executed"} else {"failed"},
        "rows":count,"expected_rows":input.diagnostic.as_ref().map_or(live.models.len(), |d| d.repetitions as usize)*2*input.cases.len(),"diagnostic":input.diagnostic,"fixture":false,
        "scope":"Frozen context-intelligence prompts through TS and Rust provider adapters; not full session/task harness superiority",
        "runtime_settled":settlement.is_ok(),"error":outcome.as_ref().err().map(ToString::to_string),
        "configuration":live.capture,"runtime_events":domain(runtime.events())?});
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(live.destination.join("execution.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.sync_all()?;
    outcome?;
    settlement?;
    verified?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Usage: release-quality PRIVATE_INPUT")?;
    let bytes = fs::read(path)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("Quality request too large".into());
    }
    execute(serde_json::from_slice(&bytes)?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn case() -> Case {
        Case {
            id: "current-evidence".into(),
            context: json!({"systemPrompt":"Use current evidence", "messages":[{"role":"user","content":"Recall the constraint"}]}),
            options: json!({"maxTokens":2048}),
        }
    }
    #[test]
    fn frozen_requests_cannot_override_sampling_or_admit_tools() {
        assert!(validate(&[case()]).is_ok());
        let mut changed = case();
        changed.options["temperature"] = json!(0);
        assert!(validate(&[changed]).is_err());
        let mut changed = case();
        changed.context["tools"] = json!([]);
        assert!(validate(&[changed]).is_err());
    }
    #[test]
    fn empty_duplicate_and_unbounded_selections_are_rejected_before_custody() {
        assert!(validate(&[]).is_err());
        assert!(validate(&[case(), case()]).is_err());
        let mut changed = case();
        changed.options["maxTokens"] = json!(65536);
        assert!(validate(&[changed]).is_err());
    }
    #[test]
    fn malformed_context_cannot_become_a_generation_request() {
        let mut changed = case();
        changed.context["messages"] = json!(null);
        assert!(validate(&[changed]).is_err());
        let mut changed = case();
        changed.context["systemPrompt"] = json!(["invalid"]);
        assert!(validate(&[changed]).is_err());
    }
}
