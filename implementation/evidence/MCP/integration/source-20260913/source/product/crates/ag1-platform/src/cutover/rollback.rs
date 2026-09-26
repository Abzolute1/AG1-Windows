//! Quiescent rollback into a new TS bridge state. The original TS state and the
//! complete native archive remain immutable recovery sources.
use super::recovery::{RecoveryFile, copy_tree, tree, verify_archive};
use super::*;
use ag1_storage::FileKind;
use serde_json::{Value, json as value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rollback {
    pub archive: PathBuf,
    pub archive_sha256: String,
    pub destination: PathBuf,
    pub staging: PathBuf,
    pub session: String,
    pub model: String,
    pub candidate_sha256: Option<String>,
    pub opened: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    version: u32,
    archive_sha256: String,
    workspace: PathBuf,
    destination: PathBuf,
    session: String,
    model: String,
    files: Vec<RecoveryFile>,
    directories: Vec<PathBuf>,
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&bounded(path, 32 * 1024 * 1024)?).map_err(json)
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    private_dir(path.parent().ok_or(DomainError::PermissionDenied)?)?;
    atomic(path, &serde_json::to_vec(value).map_err(json)?)
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
}
fn unavailable() -> DomainError {
    DomainError::Unavailable(
        "TS rollback is unsupported for this state; keep both archives and resume native recovery"
            .into(),
    )
}

impl Lease {
    pub fn freeze_rollback(
        &mut self,
        archive: &Path,
        destination: &Path,
        session: &str,
        model: &str,
    ) -> Result<()> {
        let mut r = self.record()?.clone();
        if r.owner != Owner::Native
            || r.stage != Stage::Draining
            || !r.quiescent
            || !valid_id(session)
            || !valid_id(model)
        {
            return Err(conflict());
        }
        let destination = super::migration::new_path(destination)?;
        let archive = super::migration::new_path(archive)?;
        let state = r.state.as_ref().ok_or(DomainError::NotFound)?;
        if destination.starts_with(state)
            || destination.starts_with(&r.workspace)
            || destination.starts_with(&archive)
            || archive.starts_with(&destination)
        {
            return Err(DomainError::PermissionDenied);
        }
        let store = Store::open(&state.join("store"))?;
        let snapshot = store.snapshot(session)?;
        let config: crate::client::Config =
            serde_json::from_slice(&bounded(&state.join("host.json"), 1024 * 1024)?)
                .map_err(json)?;
        if !config.fixture {
            let mut selected = config.selected;
            for entry in store.branch(session, snapshot.leaf.as_deref())? {
                let value: Value = serde_json::from_slice(&entry.normalized).map_err(json)?;
                if value["type"] == "model_change" {
                    selected = value["modelId"].as_str().ok_or_else(unavailable)?.into();
                }
            }
            if selected != model {
                return Err(unavailable());
            }
        }
        drop(store);
        let receipt = self.rollback_archive(&archive)?;
        r.rollback = Some(Rollback {
            archive,
            archive_sha256: receipt["cutover_recovery_sha256"]
                .as_str()
                .ok_or(DomainError::Conflict)?
                .into(),
            staging: destination
                .parent()
                .unwrap()
                .join(format!(".ag1-ts-rollback-{}", crate::client::nonce()?)),
            destination,
            session: session.into(),
            model: model.into(),
            candidate_sha256: None,
            opened: false,
        });
        r.stage = Stage::RollbackFrozen;
        self.save(r)
    }

    pub fn prepare_rollback(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if r.owner != Owner::Native
            || !r.quiescent
            || !matches!(r.stage, Stage::RollbackFrozen | Stage::RollbackPrepared)
        {
            return Err(conflict());
        }
        let mut rollback = r.rollback.clone().ok_or(DomainError::NotFound)?;
        let (manifest, _) = verify_archive(&rollback.archive, &rollback.archive_sha256)?;
        if manifest.authority.workspace != r.workspace
            || manifest.authority.epoch != r.epoch
            || manifest.authority.state != r.state
        {
            return Err(DomainError::Conflict);
        }
        if rollback.candidate_sha256.is_none() {
            if rollback.destination.exists() || rollback.staging.exists() {
                return Err(DomainError::Conflict);
            }
            build_candidate(&r, &rollback)?;
            let inventory = tree(&rollback.staging, &[])?;
            let candidate = Candidate {
                version: 1,
                archive_sha256: rollback.archive_sha256.clone(),
                workspace: r.workspace.clone(),
                destination: rollback.destination.clone(),
                session: rollback.session.clone(),
                model: rollback.model.clone(),
                files: inventory.files,
                directories: inventory.directories,
            };
            let bytes = serde_json::to_vec(&candidate).map_err(json)?;
            atomic(&rollback.staging.join("ts-rollback.json"), &bytes)?;
            rollback.candidate_sha256 = Some(digest(&bytes));
            r.rollback = Some(rollback.clone());
            // The candidate hash is durable before publication. Recovery cannot
            // accept a replacement directory with a self-authored manifest.
            self.save(r.clone())?;
        }
        if !rollback.destination.exists() {
            verify_candidate(&r, &rollback, &rollback.staging)?;
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &rollback.staging,
                rustix::fs::CWD,
                &rollback.destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(std::io::Error::from)
            .map_err(io)?;
            File::open(rollback.destination.parent().unwrap())
                .and_then(|f| f.sync_all())
                .map_err(io)?;
        }
        verify_candidate(&r, &rollback, &rollback.destination)?;
        r.stage = Stage::RollbackPrepared;
        self.save(r)
    }

    pub fn commit_rollback(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if r.owner != Owner::Native || r.stage != Stage::RollbackPrepared || !r.quiescent {
            return Err(conflict());
        }
        let rollback = r.rollback.as_ref().ok_or(DomainError::NotFound)?;
        verify_archive(&rollback.archive, &rollback.archive_sha256)?;
        verify_candidate(&r, rollback, &rollback.destination)?;
        r.state = Some(rollback.destination.clone());
        r.owner = Owner::TypeScript;
        r.stage = Stage::Active;
        r.epoch = r.epoch.checked_add(1).ok_or(DomainError::Conflict)?;
        r.transfer = None; // Its full record and archive reference remain in the native recovery manifest.
        self.save(r)
    }

    pub fn cancel_rollback(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if r.owner != Owner::Native
            || !r.quiescent
            || !matches!(r.stage, Stage::RollbackFrozen | Stage::RollbackPrepared)
        {
            return Err(conflict());
        }
        // Retain the rollback record, native state and any partial destinations.
        r.stage = Stage::Active;
        self.save(r)
    }
}

fn verify_candidate(record: &Record, rollback: &Rollback, path: &Path) -> Result<()> {
    let bytes = bounded(&path.join("ts-rollback.json"), 32 * 1024 * 1024)?;
    if Some(digest(&bytes)).as_ref() != rollback.candidate_sha256.as_ref() {
        return Err(DomainError::Conflict);
    }
    let candidate: Candidate = serde_json::from_slice(&bytes).map_err(json)?;
    let actual = tree(path, &["ts-rollback.json"])?;
    if candidate.version != 1
        || candidate.workspace != record.workspace
        || candidate.destination != rollback.destination
        || candidate.archive_sha256 != rollback.archive_sha256
        || candidate.session != rollback.session
        || candidate.model != rollback.model
        || candidate.files != actual.files
        || candidate.directories != actual.directories
    {
        return Err(DomainError::Conflict);
    }
    Ok(())
}

pub(super) fn admit_candidate(record: &Record, state: &Path) -> Result<()> {
    let rollback = record.rollback.as_ref().ok_or(DomainError::NotFound)?;
    verify_archive(&rollback.archive, &rollback.archive_sha256)?;
    verify_candidate(record, rollback, state)
}

fn build_candidate(record: &Record, rollback: &Rollback) -> Result<()> {
    let archive = &rollback.archive;
    let export = read_json(&archive.join("export.json"))?;
    if export["bridge_compatible_resume"] != true {
        return Err(unavailable());
    }
    let metadata = read_json(&archive.join("bridge/metadata.json"))?;
    for row in metadata.as_array().ok_or(DomainError::Conflict)? {
        // These are retained receipts/history, or a selection mapped below.
        // New native execution state needs an explicit downgrade projection.
        if !matches!(
            row["kind"].as_str(),
            Some(
                "historical"
                    | "extra"
                    | "metadata-history"
                    | "memory-history"
                    | "native-runtime-selection"
                    | "tool-operation"
                    | "runtime-interruption"
            )
        ) {
            return Err(unavailable());
        }
    }
    let transfer = record.transfer.as_ref().ok_or_else(unavailable)?;
    let frozen = bounded(&transfer.archive.join("bundle.json"), 32 * 1024 * 1024)?;
    if digest(&frozen) != transfer.manifest_sha256 {
        return Err(DomainError::Conflict);
    }
    let bundle: FrozenBundle = serde_json::from_slice(&frozen).map_err(json)?;
    bundle.files.preflight(&transfer.archive.join("source"))?;
    // Start from the retained original layout, then overlay current compatible
    // state. Only the new staging directory is writable.
    let original = archive.join("original");
    copy_tree(
        &original,
        &rollback.staging,
        &tree(&original, &["ts-rollback.json"])?,
    )?;
    // Original journals stay in the recovery archive. A new filename must not
    // make the stale original session appear as a second active session.
    for file in &bundle.files.files {
        if file.kind == FileKind::Journal {
            let path = rollback.staging.join(&file.path);
            if path.exists() {
                fs::remove_file(path).map_err(io)?;
            }
        }
    }
    let compatible = archive.join("compatible");
    copy_tree(
        &compatible.join("sessions"),
        &rollback.staging.join("sessions"),
        &tree(&compatible.join("sessions"), &[])?,
    )?;
    if compatible.join("artifacts").exists() {
        copy_tree(
            &compatible.join("artifacts"),
            &rollback.staging.join("sessions/artifacts"),
            &tree(&compatible.join("artifacts"), &[])?,
        )?;
    }
    let sessions = read_json(&archive.join("bridge/sessions.json"))?;
    for row in sessions.as_array().ok_or(DomainError::Conflict)? {
        let id = row["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or(DomainError::Conflict)?;
        if row["archived"] == true {
            let target = rollback.staging.join("native-recovery/archived-sessions");
            private_dir(&target)?;
            fs::rename(
                rollback
                    .staging
                    .join("sessions")
                    .join(format!("{id}.jsonl")),
                target.join(format!("{id}.jsonl")),
            )
            .map_err(io)?;
        }
    }
    // Keep the entire native projection behind a private inspection directory.
    copy_tree(
        &archive.join("bridge"),
        &rollback.staging.join("native-recovery"),
        &tree(&archive.join("bridge"), &[])?,
    )?;
    let auxiliary = archive.join("native-state");
    for (source, destination) in [("home/.ag1/agent", "agent"), ("project", "project")] {
        if auxiliary.join(source).exists() {
            copy_tree(
                &auxiliary.join(source),
                &rollback.staging.join(destination),
                &tree(&auxiliary.join(source), &[])?,
            )?;
        }
    }
    if archive.join("bridge/auth.json").exists() {
        write_json(
            &rollback.staging.join("agent/auth.json"),
            &read_json(&archive.join("bridge/auth.json"))?,
        )?;
    }
    let workspace = crate::workspace::WorkspaceService::new()?
        .register(&record.workspace)?
        .id()
        .as_str()
        .to_owned();
    let memory = read_json(&archive.join("bridge/memory.json"))?;
    for item in memory.as_array().ok_or(DomainError::Conflict)? {
        let id = item["id"].as_str().ok_or(DomainError::Conflict)?;
        let source = compatible
            .join("memory")
            .join(format!("{}.json", digest(id.as_bytes())));
        let target = if id == workspace {
            Some("project/memory.json")
        } else {
            bundle
                .files
                .files
                .iter()
                .find(|f| f.kind == FileKind::Memory && f.path == id)
                .map(|f| f.path.as_str())
        };
        if let Some(target) = target {
            let path = rollback.staging.join(target);
            private_dir(path.parent().ok_or(DomainError::PermissionDenied)?)?;
            atomic(&path, &bounded(&source, 32 * 1024 * 1024)?)?;
        } else {
            return Err(unavailable());
        }
    }
    let mut receipts = BTreeMap::<String, Vec<Value>>::new();
    for item in read_json(&archive.join("bridge/receipts.json"))?
        .as_array()
        .ok_or(DomainError::Conflict)?
    {
        let key = item["id"].as_str().ok_or(DomainError::Conflict)?;
        let file = format!("{}.json", digest(key.as_bytes()));
        let receipt = read_json(&archive.join("bridge/receipts").join(&file))?;
        let raw: Vec<u8> = serde_json::from_value(receipt["intent"].clone()).map_err(json)?;
        let intent: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        let envelope = if intent["kind"] == "native-client-effect" {
            &intent["payload"]["envelope"]
        } else {
            &intent["payload"]
        };
        let id = envelope["id"].as_str().unwrap_or(key);
        if !valid_id(id) {
            return Err(unavailable());
        }
        let entries = receipts.entry(id.into()).or_default();
        if entries.len() >= 256 {
            return Err(unavailable());
        }
        entries.push(value!({"key":key,"path":format!("native-recovery/receipts/{file}"),"state":receipt["state"],"kind":intent["kind"],"replay_allowed":false}));
    }
    for (id, receipts) in receipts {
        let path = rollback
            .staging
            .join("commands")
            .join(format!("{id}.command.json"));
        let prior = if path.exists() {
            read_json(&path)?
        } else {
            Value::Null
        };
        write_json(
            &path,
            &value!({"status":"interrupted","fingerprint":"migration-reserved","migration_receipts":receipts,"prior_ts_receipt":prior,"replay_allowed":false}),
        )?;
    }
    let session_file = rollback
        .destination
        .join("sessions")
        .join(format!("{}.jsonl", rollback.session));
    let raw = bounded(
        &rollback
            .staging
            .join("sessions")
            .join(format!("{}.jsonl", rollback.session)),
        256 * 1024 * 1024,
    )?;
    let header: Value = serde_json::from_slice(
        raw.split(|b| *b == b'\n')
            .next()
            .ok_or(DomainError::Conflict)?,
    )
    .map_err(json)?;
    if header["id"] != rollback.session || header["version"] != 3 {
        return Err(unavailable());
    }
    let sessions = read_json(&archive.join("bridge/sessions.json"))?;
    let selected = sessions
        .as_array()
        .ok_or(DomainError::Conflict)?
        .iter()
        .find(|row| row["id"] == rollback.session && row["archived"] == false)
        .ok_or_else(unavailable)?;
    write_json(
        &rollback.staging.join("active.json"),
        &value!({"file":session_file,"model":rollback.model,"leaf":selected["leaf"],"journal_sha256":digest(&raw)}),
    )?;
    write_json(
        &rollback.staging.join("native-recovery/status.json"),
        &value!({"version":1,"archive":rollback.archive,"sha256":rollback.archive_sha256,"replay_allowed":false,"process_observations":"historical-only"}),
    )?;
    Ok(())
}
