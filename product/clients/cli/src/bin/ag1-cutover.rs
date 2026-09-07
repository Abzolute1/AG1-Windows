//! Host-side inspection and private evidence preparation. Authority publication
//! and runtime control are deliberately not inferred from command-line paths.
use ag1_platform::cutover::{Registry, configuration, restore_native_archive};
use std::{fs, path::Path};

async fn run(args: &[String]) -> Result<serde_json::Value, String> {
    match args {
        [action, workspace] if action == "import-workspace-memory" => {
            let registry=Registry::for_user().map_err(|e|format!("{e:?}"))?;
            registry.resume_transfer(Path::new(workspace)).and_then(|lease|lease.import_workspace_memory()).map_err(|e|format!("{e:?}"))
        }
        [action, capture, unbound, destination, helper, output] if action == "bind-runtime-config" => {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let capture:configuration::ConfigurationCapture=serde_json::from_slice(&fs::read(capture).map_err(|_|"Capture receipt unavailable")?).map_err(|_|"Invalid capture receipt")?;
            let bytes=fs::read(unbound).map_err(|_|"Host configuration unavailable")?;
            if bytes.len()>1024*1024 {return Err("Host configuration exceeds bounds".into());}
            let mut config:ag1_platform::client::Config=serde_json::from_slice(&bytes).map_err(|_|"Invalid host configuration")?;
            if config.fixture {return Err("Production binding requires production configuration".into());}
            let helper=Path::new(helper).canonicalize().map_err(|_|"Supervisor unavailable")?;
            let material=ag1_platform::llama::Material {path:helper.display().to_string(),sha256:ag1_storage::digest(&fs::read(helper).map_err(|_|"Supervisor unavailable")?),kind:"process-host".into()};
            capture.bind_runtime(Path::new(destination),material,&mut config).map_err(|e|format!("{e:?}"))?;
            let raw=serde_json::to_vec(&config).map_err(|e|e.to_string())?;
            let mut file=fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(output).map_err(|_|"New private output required")?;
            file.write_all(&raw).and_then(|_|file.sync_all()).map_err(|_|"Configuration receipt requires inspection")?;
            fs::File::open(Path::new(output).parent().ok_or("Output parent missing")?).and_then(|f|f.sync_all()).map_err(|_|"Configuration receipt requires inspection")?;
            Ok(serde_json::json!({"path":output,"sha256":ag1_storage::digest(&raw),"model_actions":0,"profile_writes":0}))
        }

        [action, workspace, state, manifest] if action == "register-stopped-legacy" => {
            let input=fs::read(manifest).map_err(|_|"Legacy manifest unavailable")?;
            if input.len()>32*1024*1024 {return Err("Legacy manifest exceeds bounds".into());}
            let manifest:ag1_storage::FrozenManifest=serde_json::from_slice(&input).map_err(|_|"Invalid legacy manifest")?;
            let record=Registry::for_user().and_then(|registry|registry.register_stopped_legacy(Path::new(workspace),Path::new(state),&manifest)).map_err(|e|format!("{e:?}"))?;
            serde_json::to_value(record).map_err(|e|e.to_string())
        }

        [action, request] if action == "run-workflow" => {
            let report=ag1_platform::cutover::workflow::run_file(Path::new(request)).await.map_err(|_|"Workflow did not pass; inspect private receipts and durable authority before retrying")?;
            Ok(serde_json::json!({"status":report["status"],"fixture":report["fixture"],"phase_acceptance":false,"authority":report["authority"],"renderer_acceptance":report["renderer_acceptance"]}))
        }
        [action, workspace] if action == "audit-cutover" => {
            Registry::for_user().and_then(|registry| registry.audit_cutover(Path::new(workspace)))
                .map_err(|e| format!("{e:?}"))
        }
        [action, evidence] if action == "verify-canaries" => {
            let report = ag1_platform::cutover::canary::verify_recording_file(Path::new(evidence)).map_err(|_|"Canary evidence verification failed")?;
            let sha256 = ag1_platform::cutover::canary::recording_digest(&report).map_err(|_|"Canary evidence verification failed")?;
            Ok(serde_json::json!({"status":"verified", "fixture":report.fixture, "cases":report.rows.len(),"report_sha256":sha256,
                "authority_transferred":false,"runtime_control_granted":false}))
        }
        [action, workspace, destination] if action == "snapshot-source" || action == "snapshot-source-streaming" => {
            let registry = Registry::for_user().map_err(|e|format!("{e:?}"))?;
            let lease = registry.resume_transfer(Path::new(workspace)).map_err(|e|format!("{e:?}"))?;
            let receipt = if action == "snapshot-source-streaming" { lease.snapshot_source_streaming(Path::new(destination)) }
                else { lease.snapshot_source(Path::new(destination)) }.map_err(|e|format!("{e:?}"))?;
            serde_json::to_value(receipt).map_err(|e|e.to_string())
        }
        [action, workspace] if action == "begin-transfer" => {
            let registry=Registry::for_user().map_err(|e|format!("{e:?}"))?;
            let lease=registry.begin_transfer(Path::new(workspace)).map_err(|e|format!("{e:?}"))?;
            serde_json::to_value(lease.record().map_err(|e|format!("{e:?}"))?).map_err(|e|e.to_string())
        }
        [action, request] if action == "freeze" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Freeze { workspace: std::path::PathBuf, id: String, source: std::path::PathBuf,
                archive: std::path::PathBuf, destination: std::path::PathBuf,
                bundle: ag1_platform::cutover::FrozenBundle, config_file: std::path::PathBuf }
            let input: Freeze = serde_json::from_slice(&fs::read(request).map_err(|_|"Private transfer request unavailable")?)
                .map_err(|_|"Invalid private transfer request")?;
            let config=fs::read(input.config_file).map_err(|_|"Private host configuration unavailable")?;
            let registry=Registry::for_user().map_err(|e|format!("{e:?}"))?;
            let mut lease=registry.resume_transfer(&input.workspace).map_err(|e|format!("{e:?}"))?;
            lease.freeze(ag1_platform::cutover::Prepare { id:&input.id, source:&input.source,
                archive:&input.archive, destination:&input.destination, bundle:&input.bundle, config:&config }).map_err(|e|format!("{e:?}"))?;
            serde_json::to_value(lease.record().map_err(|e|format!("{e:?}"))?).map_err(|e|e.to_string())
        }
        [action, workspace] if ["import", "validate", "revoke", "cancel-unaccepted"].contains(&action.as_str()) => {
            let registry=Registry::for_user().map_err(|e|format!("{e:?}"))?;
            let mut lease=registry.resume_transfer(Path::new(workspace)).map_err(|e|format!("{e:?}"))?;
            match action.as_str() {
                "import" => lease.import(), "validate" => lease.validate(), "revoke" => lease.revoke(),
                _ => lease.cancel_unaccepted(),
            }.map_err(|e|format!("{e:?}"))?;
            serde_json::to_value(lease.record().map_err(|e|format!("{e:?}"))?).map_err(|e|e.to_string())
        }
        [action, request] if action == "live-canaries" => {
            let input = fs::read(request).map_err(|_| "Private canary request unavailable")?;
            let input: ag1_platform::cutover::canary::native::LiveRequest = serde_json::from_slice(&input)
                .map_err(|_| "Invalid private canary request")?;
            let destination = input.destination.clone();
            let acceptance = ag1_platform::cutover::canary::native::run_live(input).await.map_err(|e|format!("{e:?}"))?;
            Ok(serde_json::json!({"status":acceptance.report().status,"cases":acceptance.report().rows.len(),
                "destination":destination,"authority_transferred":false,"fixture":false}))
        }
        [action, workspace, request] if action == "commit-native-with-canaries" => {
            let registry = Registry::for_user().map_err(|e|format!("{e:?}"))?;
            let mut lease = registry.resume_transfer(Path::new(workspace)).map_err(|e|format!("{e:?}"))?;
            let record = lease.record().map_err(|e|format!("{e:?}"))?;
            if record.stage != ag1_platform::cutover::Stage::Revoked || !record.quiescent {
                return Err("Cutover must already be quiescent, imported, validated and revoked".into());
            }
            let input = fs::read(request).map_err(|_| "Private canary request unavailable")?;
            let input: ag1_platform::cutover::canary::native::LiveRequest = serde_json::from_slice(&input)
                .map_err(|_| "Invalid private canary request")?;
            let acceptance = ag1_platform::cutover::canary::native::run_live(input).await.map_err(|e|format!("{e:?}"))?;
            lease.commit_live(&acceptance).map_err(|e|format!("{e:?}"))?;
            serde_json::to_value(lease.record().map_err(|e|format!("{e:?}"))?).map_err(|e|e.to_string())
        }
        [action, workspace] if action == "inspect" => {
            let record = Registry::for_user().and_then(|registry| registry.inspect(Path::new(workspace)))
                .map_err(|e| format!("{e:?}"))?;
            Ok(serde_json::json!({"version":1,"authority":record,"runtime_control_granted":false}))
        }
        [action, request, destination] if action == "capture-runtime" => {
            let request = fs::read(request).map_err(|_| "Private request is unavailable")?;
            let request: configuration::ImportRequest = serde_json::from_slice(&request)
                .map_err(|_| "Invalid private configuration request")?;
            let capture = configuration::import_configuration(Path::new(destination), &request)
                .map_err(|e| format!("{e:?}"))?;
            serde_json::to_value(capture).map_err(|_| "Cannot encode capture receipt".into())
        }
        [action, workspace, archive] if action == "archive-native" => {
            let registry = Registry::for_user().map_err(|e| format!("{e:?}"))?;
            let lease = registry.resume_transfer(Path::new(workspace)).map_err(|e| format!("{e:?}"))?;
            lease.rollback_archive(Path::new(archive)).map_err(|e| format!("{e:?}"))
        }
        [action, archive, sha256, destination] if action == "restore-native" => {
            let manifest = restore_native_archive(Path::new(archive), Path::new(destination), sha256)
                .map_err(|e| format!("{e:?}"))?;
            Ok(serde_json::json!({"version":1,"destination":destination,"archived_authority":manifest.authority,
                "authority_transferred":false,"runtime_control_granted":false}))
        }
        [action, workspace, archive, destination, session, model] if action == "freeze-ts-rollback" => {
            let registry = Registry::for_user().map_err(|e| format!("{e:?}"))?;
            let mut lease = registry.resume_transfer(Path::new(workspace)).map_err(|e| format!("{e:?}"))?;
            lease.freeze_rollback(Path::new(archive), Path::new(destination), session, model).map_err(|e| format!("{e:?}"))?;
            serde_json::to_value(lease.record().map_err(|e| format!("{e:?}"))?).map_err(|e| e.to_string())
        }
        [action, workspace] if ["prepare-ts-rollback", "commit-ts-rollback", "cancel-ts-rollback"].contains(&action.as_str()) => {
            let registry = Registry::for_user().map_err(|e| format!("{e:?}"))?;
            let mut lease = registry.resume_transfer(Path::new(workspace)).map_err(|e| format!("{e:?}"))?;
            match action.as_str() {
                "prepare-ts-rollback" => lease.prepare_rollback(),
                "commit-ts-rollback" => lease.commit_rollback(),
                _ => lease.cancel_rollback(),
            }.map_err(|e| format!("{e:?}"))?;
            serde_json::to_value(lease.record().map_err(|e| format!("{e:?}"))?).map_err(|e| e.to_string())
        }
        _ => Err("Usage: ag1-cutover inspect WORKSPACE | capture-runtime PRIVATE_REQUEST NEW_PRIVATE_FILE | live-canaries PRIVATE_REQUEST | commit-native-with-canaries REVOKED_WORKSPACE PRIVATE_REQUEST | archive-native DRAINED_WORKSPACE NEW_ARCHIVE | restore-native ARCHIVE RECORDED_SHA256 NEW_STATE | freeze-ts-rollback DRAINED_WORKSPACE NEW_ARCHIVE NEW_TS_STATE SESSION MODEL | prepare-ts-rollback WORKSPACE | commit-ts-rollback WORKSPACE | cancel-ts-rollback WORKSPACE".into()),
    }
}

#[tokio::main]
async fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let outcome = if arguments.first().is_some_and(|arg| arg == "measure") {
        if arguments.len() < 3 {
            Err("Usage: ag1-cutover measure NEW_PRIVATE_DIRECTORY COMMAND ...".into())
        } else {
            match ag1_platform::cutover::measurement::Measurement::begin(
                Path::new(&arguments[1]),
                &arguments[2..],
            ) {
                Err(_) => Err(
                    "Cannot reserve new private measurement directory; no command executed".into(),
                ),
                Ok(measurement) => {
                    let result = run(&arguments[2..]).await;
                    match measurement.finish(&result) {
                        Ok(()) => result,
                        Err(_) => Err("Command returned but measurement publication failed; inspect durable authority before retrying".into()),
                    }
                }
            }
        }
    } else {
        run(&arguments).await
    };
    match outcome {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("ag1-cutover: {error}");
            std::process::exit(1);
        }
    }
}
