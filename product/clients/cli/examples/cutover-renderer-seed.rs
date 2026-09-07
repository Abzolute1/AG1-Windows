//! Disposable P13 renderer rehearsal. Creates a new fixture tree only.
use ag1_platform::cutover::{FrozenBundle, Prepare, Registry};
use ag1_storage::{FileKind, FrozenFile, FrozenManifest, digest};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("NEW_FIXTURE_DIRECTORY LOOPBACK_PROVIDER".into());
    }
    let origin = &args[1];
    if !origin.starts_with("http://127.0.0.1:")
        || origin[17..]
            .parse::<u16>()
            .ok()
            .is_none_or(|port| port == 0)
    {
        return Err("Owned loopback fixture provider required".into());
    }
    let root = PathBuf::from(&args[0]);
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let root = root.canonicalize()?;
    let workspace = root.join("workspace");
    let source = root.join("original");
    let destination = root.join("native");
    let archive = root.join("archive");
    fs::create_dir(&workspace)?;
    fs::create_dir(&source)?;
    let git = std::process::Command::new("/usr/bin/git")
        .args(["init", "--initial-branch=p13-fixture"])
        .current_dir(&workspace)
        .env_clear()
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()?;
    if !git.status.success() {
        return Err("Fixture Git initialization failed".into());
    }
    fs::write(
        workspace.join("retained.txt"),
        b"P13 renderer fixture source\n",
    )?;
    let header = json!({"type":"session","version":3,"id":"p13-renderer","cwd":workspace,"timestamp":"2026-09-07T00:00:00Z"});
    let message = json!({"type":"message","id":"original-message","parentId":null,"timestamp":"2026-09-07T00:00:01Z",
        "message":{"role":"user","content":"Imported P13 renderer history","timestamp":1}});
    let journal = format!("{header}\n{message}\n");
    fs::write(source.join("session.jsonl"), &journal)?;
    let config = serde_json::to_vec(
        &json!({"models":[{"provider":"fixture","id":"initial","api":"openai-completions","baseUrl":origin,
        "contextWindow":32768,"maxTokens":4096,"input":["text"],"reasoning":false}],"selected":"initial","credential_env":null,
        "system_prompt":"Owned P13 renderer rehearsal. Return the requested marker only.","fixture":true}),
    )?;
    let registry = Registry::for_user().map_err(|e| format!("{e:?}"))?;
    let mut owner = registry
        .open_typescript(&workspace, &source)
        .map_err(|e| format!("{e:?}"))?;
    owner.begin_drain().map_err(|e| format!("{e:?}"))?;
    owner.quiesced().map_err(|e| format!("{e:?}"))?;
    drop(owner);
    let mut lease = registry
        .resume_transfer(&workspace)
        .map_err(|e| format!("{e:?}"))?;
    let source_snapshot = lease
        .snapshot_source(&root.join("source-archive"))
        .map_err(|e| format!("{e:?}"))?;
    let bundle = FrozenBundle {
        version: 1,
        files: FrozenManifest {
            version: 1,
            frozen: true,
            files: vec![FrozenFile {
                path: "session.jsonl".into(),
                sha256: digest(journal.as_bytes()),
                bytes: journal.len() as u64,
                kind: FileKind::Journal,
                session: Some("p13-renderer".into()),
                origin: None,
            }],
        },
        selected_session: "p13-renderer".into(),
        config_sha256: digest(&config),
        runtime_files: vec![],
        runtime_configuration: None,
        source_snapshot: Some(source_snapshot),
    };
    lease
        .freeze(Prepare {
            id: "p13-renderer",
            source: &source,
            archive: &archive,
            destination: &destination,
            bundle: &bundle,
            config: &config,
        })
        .map_err(|e| format!("{e:?}"))?;
    lease.import().map_err(|e| format!("{e:?}"))?;
    lease.validate().map_err(|e| format!("{e:?}"))?;
    lease.revoke().map_err(|e| format!("{e:?}"))?;
    lease.commit_fixture().map_err(|e| format!("{e:?}"))?;
    println!(
        "{}",
        json!({"fixture":true,"workspace":workspace,"session":"p13-renderer","original_state":source,"destination":destination})
    );
    Ok(())
}
