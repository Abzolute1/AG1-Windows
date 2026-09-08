//! P14 supervised legacy workload. The real native lease and permit outlive
//! every child request; the TS session delegates runtime preparation over a
//! private authenticated socket, never spawning or tuning a model itself.
use ag1_core::{
    DomainError, OperationId,
    inference::{EndpointKind, Scheduler},
};
use ag1_platform::{
    cutover::{Registry, canary::native::LiveRequest},
    llama::{
        LaunchPlan,
        custody::RuntimeBundle,
        lifecycle::Observation,
        native::{MaterialVerifier, NativeRuntimeHost},
    },
    process::{ProcessSpec, ProcessSupervisor},
    provider::http::{GenerationRuntime, HostHeaders, HttpMethod, HttpRequest, ProviderHttp},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Observer = Box<dyn FnMut(&LaunchPlan) -> std::result::Result<Observation, DomainError> + Send>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    live: LiveRequest,
    model: String,
    workload: Value,
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
fn admitted(value: &Value, secret: &str, model: &str) -> bool {
    value.as_object().is_some_and(|v| v.len() == 3)
        && value["key"] == secret
        && value["action"] == "prepare"
        && value["model"] == model
}
fn request(origin: &str, model: &str) -> HttpRequest {
    HttpRequest {
        url: format!("{origin}/v1/chat/completions"),
        method: HttpMethod::Post,
        headers: HostHeaders::default(),
        body: json!({"model":model}).to_string().into_bytes(),
        max_response_bytes: 1024,
    }
}
async fn run(input: Input) -> Result<()> {
    let live = input.live;
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let base = repo.join(".local/p14-long-horizon").canonicalize()?;
    if live.destination.exists()
        || !live
            .destination
            .parent()
            .ok_or("parent")?
            .canonicalize()?
            .starts_with(&base)
    {
        return Err("New private long-horizon destination required".into());
    }
    if input.workload["mode"] != "seed-and-typescript"
        || input.workload["rawTokens"] != 2_500_000
        || input.workload["phases"] != 64
    {
        return Err(
            "Preflight workload must retain the original 2.5M-token, 64-phase fixture".into(),
        );
    }
    let socket = live.destination.join("control.sock");
    if socket.as_os_str().len() > 100 {
        return Err("Private control path too long".into());
    }
    let registry = d(Registry::for_user())?;
    let lease = d(registry.runtime_lease())?;
    d(ag1_platform::llama::window::verify(None))?;
    let plans = d(live.capture.verify())?;
    if plans.len() != 3 || live.models.len() != 3 {
        return Err("Three preserved plans required".into());
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
            || d(ag1_platform::provider::http::endpoint_origin(
                model["baseUrl"].as_str().ok_or("origin")?,
            ))? != d(ag1_platform::llama::observation::origin(plan))?
        {
            return Err("Model differs from preserved plan".into());
        }
    }
    let model = live
        .models
        .iter()
        .find(|m| m["id"] == input.model)
        .ok_or("Selected preserved model missing")?
        .clone();
    let mut verifier = MaterialVerifier::default();
    d(verifier.verify(&[
        live.reference.node.clone(),
        live.reference.probe.clone(),
        live.process_host.clone(),
    ]))?;
    for chunk in live.reference.materials.chunks(1024) {
        d(verifier.verify(chunk))?;
    }
    if !live.reference.materials.iter().any(|m| {
        Path::new(&m.path)
            == live
                .reference
                .source
                .join("packages/coding-agent/dist/core/agent-session.js")
    }) {
        return Err("Full TS session closure required".into());
    }
    fs::create_dir(&live.destination)?;
    fs::set_permissions(&live.destination, fs::Permissions::from_mode(0o700))?;
    let handle = tokio::runtime::Handle::current();
    let observer: Observer = Box::new(move |plan| {
        let origin = ag1_platform::llama::observation::origin(plan)?;
        handle.block_on(async {
            let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(2))?;
            ag1_platform::llama::observation::observe(&http, &origin).await
        })
    });
    let host = d(NativeRuntimeHost::new(
        Path::new(&live.process_host.path),
        &live.destination,
        observer,
    ))?
    .require_exclusive_gpu();
    let runtime = Arc::new(d(RuntimeBundle::new(
        host,
        plans,
        lease,
        Duration::from_secs(900),
    ))?);
    let scheduler = Scheduler::default();
    d(scheduler.register(runtime.origin(), EndpointKind::Local))?;
    let ticket = d(scheduler.submit(
        runtime.origin(),
        d(OperationId::new("p14-long-horizon-ts"))?,
    ))?;
    let permit = d(scheduler.start(&ticket))?.ok_or("Runtime permit unavailable")?;
    let started = Instant::now();
    let outcome:Result<Value>=async {
        d(runtime.prepare(&request(runtime.origin(),&input.model)).await)?;
        let secret=d(ag1_platform::client::nonce())?;
        let listener=UnixListener::bind(&socket)?;listener.set_nonblocking(true)?;
        let stop=Arc::new(AtomicBool::new(false));
        let done=stop.clone();let owned=runtime.clone();let selected=input.model.clone();let key=secret.clone();let handle=tokio::runtime::Handle::current();
        let control=std::thread::spawn(move || -> Result<u64> {
            let mut count=0;
            while !done.load(Ordering::Acquire) {
                let (mut stream,_)=match listener.accept() {Ok(v)=>v,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{std::thread::sleep(Duration::from_millis(10));continue;},Err(e)=>return Err(e.into())};
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut body=vec![];
                loop {let mut byte=[0];if stream.read(&mut byte)?==0 || byte[0]==b'\n' {break;}body.push(byte[0]);if body.len()>4096 {break;}}
                let value=serde_json::from_slice(&body).unwrap_or(Value::Null);
                let allowed=body.len()<=4096 && admitted(&value,&key,&selected) && !done.load(Ordering::Acquire);
                let result=if allowed {handle.block_on(owned.prepare(&request(owned.origin(),&selected)))} else {Err(DomainError::PermissionDenied)};
                if result.is_ok(){count+=1;}
                // No model paths, credentials or runtime authority handles cross this channel.
                let response=json!({"prepared":result.is_ok() && !done.load(Ordering::Acquire)});
                let _=writeln!(stream,"{response}");
            }
            Ok(count)
        });
        let child_input=live.destination.join("worker-input.json");
        let prepared=(||->Result<()> {save(&child_input,&json!({"model":model,"workload":input.workload,"source":live.reference.source,"destination":live.destination,"control":{"socket":socket,"key":secret},"fixture":false}))?;Ok(())})();
        let child_result=if prepared.is_ok() {
            let helper=live.process_host.path.clone();let node=live.reference.node.path.clone();let script=live.reference.probe.path.clone();let destination=live.destination.clone();
            tokio::task::spawn_blocking(move || -> Result<i32> {
                let supervisor=d(ProcessSupervisor::new(Path::new(&helper)))?;
                let node_dir=Path::new(&node).parent().ok_or("Node parent")?;
                let mut child=d(supervisor.spawn(ProcessSpec {executable:node.clone().into(),argv:vec![script,child_input.display().to_string()],cwd:destination.clone(),env:[("PATH".into(),format!("{}:/usr/bin:/bin",node_dir.display()))].into(),timeout_ms:10_800_000,grace_ms:1000,resident:false},&destination.join("worker.log")))?;
                let status=d(child.wait(Duration::from_secs(10805)));
                let settlement=d(child.stop());settlement?;
                Ok(status?.code().unwrap_or(-1))
            }).await.map_err(|_|"Supervised worker panicked".into()).and_then(|r|r)
        } else {Err(prepared.unwrap_err())};
        stop.store(true,Ordering::Release);
        let preparations=control.join().map_err(|_|"Owner control thread panicked")??;
        fs::remove_file(&socket)?;
        let code=child_result?;
        save(&live.destination.join("worker-settlement.json"),&json!({"exit_code":code,"owned_tree_stopped":true,"runtime_preparations":preparations}))?;
        Ok(json!({"worker_exit_code":code,"runtime_preparations":preparations}))
    }.await;
    // Always settle the authenticated model, including rejected/failed worker work.
    let settlement = d(runtime.stop().await);
    let events = d(runtime.events())?;
    save(
        &live.destination.join("runtime-events.json"),
        &json!(events),
    )?;
    settlement?;
    d(permit.terminal())?;
    let preservation = d(live.capture.verify());
    save(
        &live.destination.join("execution.json"),
        &json!({"status":if outcome.is_ok(){"executed"}else{"failed"},"fixture":false,"release_acceptance":false,"runtime_settled":true,"preservation_verified":preservation.is_ok(),"seconds":started.elapsed().as_secs_f64(),"result":outcome.as_ref().ok(),"error":outcome.as_ref().err().map(ToString::to_string)}),
    )?;
    preservation?;
    let result = outcome?;
    if result["worker_exit_code"] != 0 {
        return Err("Long-horizon worker did not pass; evidence retained".into());
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Private owner input required")?;
    let bytes = fs::read(path)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Owner input too large".into());
    }
    run(serde_json::from_slice(&bytes)?).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delegation_is_bound_to_one_secret_model_and_prepare_action() {
        assert!(admitted(
            &json!({"key":"secret","model":"model","action":"prepare"}),
            "secret",
            "model"
        ));
        for value in [
            json!({"key":"copied","model":"model","action":"prepare"}),
            json!({"key":"secret","model":"other","action":"prepare"}),
            json!({"key":"secret","model":"model","action":"stop"}),
            json!({"key":"secret","model":"model","action":"prepare","endpoint":"override"}),
            Value::Null,
        ] {
            assert!(!admitted(&value, "secret", "model"));
        }
    }
}
