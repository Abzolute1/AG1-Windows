use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenBundle {
    pub version: u32,
    pub files: FrozenManifest,
    pub selected_session: String,
    /// Exact private configuration bytes are archived, not normalized into a
    /// reconstructed profile. Credentials remain private host bindings.
    pub config_sha256: String,
    #[serde(default)]
    pub runtime_files: Vec<crate::llama::Material>,
    #[serde(default)]
    pub runtime_configuration: Option<super::configuration::ConfigurationCapture>,
    #[serde(default)]
    pub source_snapshot: Option<super::SourceSnapshot>,
}
pub struct Prepare<'a> {
    pub id: &'a str,
    pub source: &'a Path,
    pub archive: &'a Path,
    pub destination: &'a Path,
    pub bundle: &'a FrozenBundle,
    pub config: &'a [u8],
}

impl Lease {
    /// Import workspace memory that lives outside the legacy session directory.
    /// Source/Git custody binds this file; the original is never opened to write.
    pub fn import_workspace_memory(&self) -> Result<serde_json::Value> {
        let r = self.record()?;
        if r.owner != Owner::TypeScript || r.stage != Stage::Imported || !r.quiescent {
            return Err(conflict());
        }
        let (t, bundle) = self.transfer_bundle()?;
        let workspace = crate::workspace::WorkspaceService::new()?.register(&r.workspace)?;
        let input = match workspace.snapshot(".ag1/memory.json", 8 * 1024 * 1024) {
            Ok(input) => input,
            Err(DomainError::NotFound) => {
                return Ok(serde_json::json!({"status":"absent","original_data_writes":0}));
            }
            Err(error) => return Err(error),
        };
        let raw_hash = digest(&input.bytes);
        let store = Store::open(&t.destination.join("store"))?;
        let id = workspace.id().as_str();
        match store.memory(id)? {
            Some(existing) if existing.digest == raw_hash && existing.status == "valid" => (),
            Some(_) => return Err(DomainError::Conflict),
            None => {
                store.save_memory(id, None, &input.bytes)?;
            }
        }
        if workspace
            .snapshot(".ag1/memory.json", 8 * 1024 * 1024)?
            .bytes
            != input.bytes
        {
            return Err(DomainError::Conflict);
        }
        let report = serde_json::json!({"version":1,"status":"imported","workspace":r.workspace,
            "memory_id":id,"source":".ag1/memory.json","sha256":raw_hash,"bytes":input.bytes.len(),
            "source_snapshot":bundle.source_snapshot,"original_data_writes":0});
        let receipt = t.destination.join("workspace-memory-import.json");
        let bytes = serde_json::to_vec(&report).map_err(json)?;
        if receipt.exists() {
            if bounded(&receipt, 1024 * 1024)? != bytes {
                return Err(DomainError::Conflict);
            }
        } else {
            atomic(&receipt, &bytes)?;
        }
        Ok(report)
    }

    /// Freeze into a new archive while the workspace is quiescent and unowned.
    /// The manifest must enumerate every source file; P05 rejects missing closure.
    pub fn freeze(&mut self, input: Prepare<'_>) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent
            || r.stage != Stage::Draining
            || r.owner != Owner::TypeScript
            || r.transfer.is_some()
        {
            return Err(conflict());
        }
        if input.id.is_empty()
            || input.id.len() > 128
            || input.bundle.version != 1
            || input.bundle.config_sha256 != digest(input.config)
        {
            return Err(DomainError::InvalidInput("cutover bundle".into()));
        }
        let config: crate::client::Config = serde_json::from_slice(input.config).map_err(json)?;
        if !config.fixture && input.bundle.source_snapshot.is_none() {
            return Err(DomainError::Unavailable(
                "Production transfer requires a separate source/Git snapshot".into(),
            ));
        }
        if let Some(snapshot) = &input.bundle.source_snapshot {
            snapshot.verify_unchanged(&r.workspace)?;
        }
        input.bundle.files.preflight(input.source)?;
        if let Some(configuration) = &input.bundle.runtime_configuration {
            configuration.verify()?;
        }
        let source = fs::canonicalize(input.source).map_err(io)?;
        if r.state.as_ref() != Some(&source) {
            return Err(DomainError::PermissionDenied);
        }
        if !input.bundle.runtime_files.is_empty() {
            crate::llama::native::MaterialVerifier::default()
                .verify(&input.bundle.runtime_files)?;
        }
        let archive = new_path(input.archive)?;
        let destination = new_path(input.destination)?;
        if archive == destination
            || archive.starts_with(&destination)
            || destination.starts_with(&archive)
            || archive.starts_with(&source)
            || destination.starts_with(&source)
            || source.starts_with(&archive)
            || source.starts_with(&destination)
            || archive.starts_with(&r.workspace)
            || destination.starts_with(&r.workspace)
        {
            return Err(DomainError::PermissionDenied);
        }
        // A failed freeze keeps a private, inspectable partial archive. No retry
        // may overwrite it or mistake it for a completed transfer.
        fs::create_dir(&archive).map_err(io)?;
        fs::set_permissions(&archive, fs::Permissions::from_mode(0o700)).map_err(io)?;
        let frozen = archive.join("source");
        private_dir(&frozen)?;
        for file in &input.bundle.files.files {
            let bytes = bounded(&source.join(&file.path), file.bytes)?;
            if bytes.len() as u64 != file.bytes || digest(&bytes) != file.sha256 {
                return Err(DomainError::Conflict);
            }
            let target = frozen.join(&file.path);
            private_dir(target.parent().ok_or(DomainError::PermissionDenied)?)?;
            atomic(&target, &bytes)?;
        }
        input.bundle.files.preflight(&frozen)?;
        input.bundle.files.preflight(&source)?;
        let mut bundle = input.bundle.clone();
        if let Some(configuration) = &mut bundle.runtime_configuration {
            let bytes = bounded(&configuration.path, 128 * 1024 * 1024)?;
            if digest(&bytes) != configuration.sha256 {
                return Err(DomainError::Conflict);
            }
            configuration.path = archive.join("runtime-configuration.json");
            atomic(&configuration.path, &bytes)?;
            configuration.verify()?;
        }
        let manifest = serde_json::to_vec(&bundle).map_err(json)?;
        atomic(&archive.join("bundle.json"), &manifest)?;
        atomic(&archive.join("host.json"), input.config)?;
        File::open(archive.parent().unwrap())
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        if let Some(snapshot) = &bundle.source_snapshot {
            snapshot.verify_unchanged(&r.workspace)?;
        }
        r.transfer = Some(Transfer {
            id: input.id.into(),
            original_state: r.state.clone().ok_or(DomainError::NotFound)?,
            archive,
            destination,
            manifest_sha256: digest(&manifest),
            session: input.bundle.selected_session.clone(),
            config_sha256: input.bundle.config_sha256.clone(),
        });
        r.stage = Stage::Frozen;
        self.save(r)
    }
    pub(super) fn bundle(&self) -> Result<(Transfer, FrozenBundle)> {
        let t = self
            .record()?
            .transfer
            .clone()
            .ok_or(DomainError::NotFound)?;
        let raw = bounded(&t.archive.join("bundle.json"), 32 * 1024 * 1024)?;
        if digest(&raw) != t.manifest_sha256
            || digest(&bounded(&t.archive.join("host.json"), 1024 * 1024)?) != t.config_sha256
        {
            return Err(DomainError::Conflict);
        }
        let b: FrozenBundle = serde_json::from_slice(&raw).map_err(json)?;
        if b.version != 1 || b.selected_session != t.session || b.config_sha256 != t.config_sha256 {
            return Err(DomainError::Conflict);
        }
        b.files.preflight(&t.archive.join("source"))?;
        if !b.runtime_files.is_empty() {
            crate::llama::native::MaterialVerifier::default().verify(&b.runtime_files)?;
        }
        if let Some(configuration) = &b.runtime_configuration {
            configuration.verify()?;
        }
        if let Some(snapshot) = &b.source_snapshot {
            snapshot.verify_archive(&self.record()?.workspace)?;
        }
        Ok((t, b))
    }
    fn transfer_bundle(&self) -> Result<(Transfer, FrozenBundle)> {
        let result = self.bundle()?;
        if let Some(snapshot) = &result.1.source_snapshot {
            snapshot.verify_unchanged(&self.record()?.workspace)?;
        }
        Ok(result)
    }
    pub fn import(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent || !matches!(r.stage, Stage::Frozen | Stage::Imported) {
            return Err(conflict());
        }
        let (t, b) = self.transfer_bundle()?;
        if !t.destination.exists() {
            fs::create_dir(&t.destination).map_err(io)?;
            fs::set_permissions(&t.destination, fs::Permissions::from_mode(0o700)).map_err(io)?;
            atomic(
                &t.destination.join("transfer.json"),
                &serde_json::to_vec(&t).map_err(json)?,
            )?;
        }
        private_dir(&t.destination)?;
        let marker = bounded(&t.destination.join("transfer.json"), 1024 * 1024)?;
        if serde_json::from_slice::<Transfer>(&marker).map_err(json)? != t {
            return Err(DomainError::Conflict);
        }
        let store = Store::open(&t.destination.join("store"))?;
        store.import_frozen(&t.archive.join("source"), &b.files)?;
        store.snapshot(&t.session)?;
        store.reconcile()?;
        if let Some(configuration) = &b.runtime_configuration {
            let config: crate::client::Config =
                serde_json::from_slice(&bounded(&t.archive.join("host.json"), 1024 * 1024)?)
                    .map_err(json)?;
            configuration.materialize(&t.destination, &config)?;
        }
        atomic(
            &t.destination.join("host.json"),
            &bounded(&t.archive.join("host.json"), 1024 * 1024)?,
        )?;
        r.stage = Stage::Imported;
        self.save(r)
    }
    /// Offline validation only; an imported process observation is never acted
    /// upon. This does not certify live per-model acceptance.
    pub fn validate(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent || !matches!(r.stage, Stage::Imported | Stage::Validated) {
            return Err(conflict());
        }
        let (t, _) = self.transfer_bundle()?;
        let config = bounded(&t.destination.join("host.json"), 1024 * 1024)?;
        if digest(&config) != t.config_sha256 {
            return Err(DomainError::Conflict);
        }
        let _: crate::client::Config = serde_json::from_slice(&config).map_err(json)?;
        let store = Store::open(&t.destination.join("store"))?;
        store.reconcile()?;
        store.snapshot(&t.session)?;
        r.stage = Stage::Validated;
        self.save(r)
    }
    pub fn revoke(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent || r.stage != Stage::Validated {
            return Err(conflict());
        }
        self.transfer_bundle()?;
        r.stage = Stage::Revoked;
        self.save(r)
    }
    /// Before native acceptance no native session has executed. Return to the
    /// untouched TS owner only after rechecking its original frozen source.
    pub fn cancel_unaccepted(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent
            || r.owner != Owner::TypeScript
            || !matches!(
                r.stage,
                Stage::Frozen | Stage::Imported | Stage::Validated | Stage::Revoked
            )
        {
            return Err(conflict());
        }
        let (transfer, bundle) = self.bundle()?;
        bundle.files.preflight(&transfer.original_state)?;
        let receipt = self
            .path
            .with_file_name(format!("cancelled-{}.json", crate::client::nonce()?));
        atomic(&receipt, &serde_json::to_vec(&r).map_err(json)?)?;
        r.state = Some(transfer.original_state);
        r.transfer = None;
        r.stage = Stage::Active;
        r.epoch = r.epoch.checked_add(1).ok_or(DomainError::Conflict)?;
        self.save(r)
    }
    /// Fixture-only acceptance. Production configurations require commit_live.
    pub fn commit_fixture(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent || r.stage != Stage::Revoked {
            return Err(conflict());
        }
        let (t, _) = self.transfer_bundle()?;
        let config: crate::client::Config =
            serde_json::from_slice(&bounded(&t.archive.join("host.json"), 1024 * 1024)?)
                .map_err(json)?;
        if !config.fixture {
            return Err(DomainError::Unavailable(
                "Live runtime acceptance is required before cutover".into(),
            ));
        }
        r.state = Some(t.destination);
        r.owner = Owner::Native;
        r.stage = Stage::Active;
        r.isolated = false;
        r.epoch = r.epoch.checked_add(1).ok_or(DomainError::Conflict)?;
        self.save(r)
    }
    /// Only a live, non-serializable runtime proof can open production authority.
    /// The proof retains the per-user runtime lease across this atomic decision.
    pub fn commit_live(&mut self, acceptance: &super::canary::native::Acceptance) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent || r.stage != Stage::Revoked || r.owner != Owner::TypeScript {
            return Err(conflict());
        }
        let (t, bundle) = self.transfer_bundle()?;
        let config: crate::client::Config =
            serde_json::from_slice(&bounded(&t.destination.join("host.json"), 1024 * 1024)?)
                .map_err(json)?;
        if digest(&bounded(&t.destination.join("host.json"), 1024 * 1024)?) != t.config_sha256
            || bundle.runtime_configuration.as_ref().map(|c| &c.sha256)
                != config.runtime.as_ref().map(|c| &c.capture.sha256)
        {
            return Err(DomainError::Conflict);
        }
        acceptance.verify_configuration(&config)?;
        let store = Store::open(&t.destination.join("store"))?;
        store.reconcile()?;
        store.snapshot(&t.session)?;
        // Keep this new lossless export even if downgrade compatibility fails.
        let recovery = t
            .destination
            .parent()
            .ok_or(DomainError::Conflict)?
            .join(format!(
                "p13-precommit-recovery-{}",
                crate::client::nonce()?
            ));
        let exported = store.export(&recovery, false)?;
        if exported["bridge_compatible_resume"] != true {
            return Err(DomainError::Unavailable(
                "Production rollback closure is unsupported; recovery export retained".into(),
            ));
        }
        atomic(&t.destination.join("runtime-acceptance.json"), &serde_json::to_vec(
            &serde_json::json!({"version":1,"report":acceptance.report(),"models":acceptance.models(),"transfer":t,
                "precommit_recovery":recovery,"recovery_manifest_sha256":digest(&serde_json::to_vec(&exported).map_err(json)?)})).map_err(json)?)?;
        r.state = Some(t.destination);
        r.owner = Owner::Native;
        r.stage = Stage::Active;
        r.isolated = false;
        r.epoch = r.epoch.checked_add(1).ok_or(DomainError::Conflict)?;
        self.save(r)
    }
    /// Always preserve a lossless archive first. Unknown commands, corrupt
    /// memory and inherited cursor roots must block an unsafe TS downgrade.
    pub fn rollback_archive(&self, archive: &Path) -> Result<serde_json::Value> {
        let r = self.record()?;
        if r.owner != Owner::Native || r.stage != Stage::Draining || !r.quiescent {
            return Err(conflict());
        }
        let state = r.state.as_ref().ok_or(DomainError::NotFound)?;
        let store = Store::open(&state.join("store"))?;
        super::recovery::archive_native(r, state, &store, archive)
    }
}

pub(super) fn new_path(path: &Path) -> Result<PathBuf> {
    if fs::symlink_metadata(path).is_ok() || !path.is_absolute() {
        return Err(DomainError::Conflict);
    }
    let parent = path.parent().ok_or(DomainError::PermissionDenied)?;
    let parent = fs::canonicalize(parent).map_err(io)?;
    let name = path.file_name().ok_or(DomainError::PermissionDenied)?;
    Ok(parent.join(name))
}
