//! Final authority inspection. Evidence inspection grants no execution lease to
//! its caller and never starts a model or repairs a session's durable state.
use super::*;
use serde_json::{Value, json as value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeReceipt {
    version: u32,
    report: canary::Report,
    models: Vec<Value>,
    transfer: Transfer,
    precommit_recovery: PathBuf,
    recovery_manifest_sha256: String,
}

fn verify_export(path: &Path, expected: &str) -> Result<()> {
    let raw = bounded(&path.join("export.json"), 32 * 1024 * 1024)?;
    let manifest: Value = serde_json::from_slice(&raw).map_err(json)?;
    if digest(&serde_json::to_vec(&manifest).map_err(json)?) != expected
        || manifest["version"] != 1
        || manifest["schema"] != 1
        || manifest["kind"] != "ag1-shadow-lossless-export"
        || manifest["runtime_authority"] != "historical-only"
        || manifest["bridge_compatible_resume"] != true
    {
        return Err(DomainError::Conflict);
    }
    let mut files: Vec<recovery::RecoveryFile> =
        serde_json::from_value(manifest["files"].clone()).map_err(json)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut actual = recovery::tree(path, &["export.json"])?.files;
    actual.sort_by(|a, b| a.path.cmp(&b.path));
    if files != actual
        || !files
            .iter()
            .any(|file| file.path == Path::new("storage.sqlite"))
    {
        return Err(DomainError::Conflict);
    }
    Ok(())
}

impl Registry {
    /// Requires the native owner to have stopped cleanly and released its lock.
    /// Fixture inspection stays explicitly ineligible for local acceptance.
    pub fn audit_cutover(&self, workspace: &Path) -> Result<Value> {
        let lease = self.locked(workspace)?;
        let record = lease.record()?;
        if record.owner != Owner::Native
            || record.stage != Stage::Active
            || record.isolated
            || !record.quiescent
            || record.transfer.is_none()
        {
            return Err(conflict());
        }
        let (transfer, bundle) = lease.bundle()?;
        if record.state.as_ref() != Some(&transfer.destination) {
            return Err(DomainError::Conflict);
        }
        bundle.files.preflight(&transfer.original_state)?;
        let raw_config = bounded(&transfer.destination.join("host.json"), 1024 * 1024)?;
        if digest(&raw_config) != transfer.config_sha256 {
            return Err(DomainError::Conflict);
        }
        let config: crate::client::Config = serde_json::from_slice(&raw_config).map_err(json)?;
        let _runtime_lease = if config.fixture {
            None
        } else {
            let lease = self.runtime_lease()?;
            crate::llama::window::verify(None)?;
            Some(lease)
        };
        if !config.fixture && bundle.source_snapshot.is_none() {
            return Err(DomainError::Conflict);
        }
        if let Some(source) = &bundle.source_snapshot {
            source.verify_archive(&record.workspace)?;
        }
        let (health, snapshot) =
            Store::inspect_quiescent(&transfer.destination.join("store"), &transfer.session)?;
        if health.staged_files != 0 {
            return Err(DomainError::Unavailable(
                "Unreconciled staged state requires native recovery before acceptance".into(),
            ));
        }
        let mut runtime_receipt = Value::Null;
        if !config.fixture {
            let path = transfer.destination.join("runtime-acceptance.json");
            let raw = bounded(&path, 128 * 1024 * 1024)?;
            let receipt: RuntimeReceipt = serde_json::from_slice(&raw).map_err(json)?;
            let runtime = config.runtime.as_ref().ok_or(DomainError::Conflict)?;
            // Freeze relocates the capture archive; compare content identity
            // and verify each retained path independently.
            if receipt.version != 1
                || receipt.transfer != transfer
                || receipt.report.fixture
                || bundle
                    .runtime_configuration
                    .as_ref()
                    .map(|c| (&c.sha256, &c.plan_sha256))
                    != Some((&runtime.capture.sha256, &runtime.capture.plan_sha256))
                || receipt
                    .models
                    .iter()
                    .any(|model| !config.models.contains(model))
            {
                return Err(DomainError::Conflict);
            }
            canary::verify_recording(&receipt.report, &receipt.models)?;
            let plans = runtime.capture.verify()?;
            if plans.len() != 3 || receipt.models.len() != 3 {
                return Err(DomainError::Conflict);
            }
            for plan in plans {
                let model = receipt
                    .models
                    .iter()
                    .find(|model| model["id"] == plan.model_id)
                    .ok_or(DomainError::Conflict)?;
                if model["provider"] != "llamacpp"
                    || model["api"] != "openai-completions"
                    || model["contextWindow"].as_u64() != Some(plan.requested_context)
                    || model["maxTokens"].as_u64() != Some(plan.max_tokens)
                {
                    return Err(DomainError::Conflict);
                }
            }
            crate::llama::native::MaterialVerifier::default()
                .verify(std::slice::from_ref(&runtime.process_host))?;
            verify_export(
                &receipt.precommit_recovery,
                &receipt.recovery_manifest_sha256,
            )?;
            runtime_receipt = value!({"path":path,"sha256":digest(&raw),
                "canary_report_sha256":canary::recording_digest(&receipt.report)?,
                "precommit_recovery":receipt.precommit_recovery,"recovery_manifest_sha256":receipt.recovery_manifest_sha256,
                "capture_sha256":runtime.capture.sha256,"plan_sha256":runtime.capture.plan_sha256});
        }
        let rollback = if let Some(rollback) = &record.rollback {
            let (manifest, _) =
                recovery::verify_archive(&rollback.archive, &rollback.archive_sha256)?;
            if manifest.authority.workspace != record.workspace
                || manifest.authority.state != record.state
            {
                return Err(DomainError::Conflict);
            }
            value!({"archive":rollback.archive,"sha256":rollback.archive_sha256,
                "candidate_retained":rollback.candidate_sha256.is_some(),"authority_reverted":false})
        } else {
            Value::Null
        };
        if self.inspect(workspace)?.as_ref() != Some(record) {
            return Err(DomainError::Conflict);
        }
        Ok(
            value!({"version":1,"status":if config.fixture {"fixture_verified"} else {"production_authority_verified"},
            "fixture":config.fixture,"authority":record,"selected_session":transfer.session,
            "snapshot_sha256":digest(&serde_json::to_vec(&snapshot).map_err(json)?),"storage":health,
            "original_state_unchanged":true,"source_snapshot":bundle.source_snapshot,
            "runtime_acceptance":runtime_receipt,"rollback_archive":rollback,
            "runtime_control_granted":false,"phase_acceptance":false}),
        )
    }
}
