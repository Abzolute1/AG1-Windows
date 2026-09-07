//! Read-only configuration capture and pinned TS/Rust launch comparison. This
//! creates private evidence; it never launches, tunes, or adopts a runtime.
use super::*;
use crate::llama::{LaunchPlan, Material, ResolveInput, resolve_launch_plan};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedInput {
    pub path: PathBuf,
    pub sha256: String,
}

/// Host-only input: inherited environment and expected environment may contain
/// credentials. Do not expose this type through protocol DTOs or diagnostics.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub context_window: u64,
    pub max_tokens: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelImport {
    pub model_id: String,
    pub model_path: String,
    pub context_override: Option<u64>,
    pub forced_moe: Option<i64>,
    pub cached_moe: Option<i64>,
    pub discovered_projector: Option<String>,
    pub materials: Vec<Material>,
    pub expected: ExpectedLaunch,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub profile: PinnedInput,
    pub inherited_environment: PinnedInput,
    /// Exact settings/vault bytes are retained privately; capture alone does
    /// not claim that unsupported settings or credentials have been mapped.
    pub additional_inputs: BTreeMap<String, PinnedInput>,
    pub physical_cores: u64,
    pub logical_cores: u64,
    pub vram_mib: Option<u64>,
    pub models: Vec<ModelImport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationCapture {
    pub path: PathBuf,
    pub sha256: String,
    pub plan_sha256: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    version: u32,
    request: ImportRequest,
    inputs: BTreeMap<String, Vec<u8>>,
}

fn pinned(input: &PinnedInput) -> Result<Vec<u8>> {
    if !input.path.is_absolute() {
        return Err(DomainError::PermissionDenied);
    }
    let bytes = bounded(&input.path, 16 * 1024 * 1024)?;
    if digest(&bytes) != input.sha256 {
        return Err(DomainError::Conflict);
    }
    Ok(bytes)
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    // serde diagnostics can include unknown field names from private inputs.
    serde_json::from_slice(bytes)
        .map_err(|_| DomainError::InvalidInput("Private configuration format".into()))
}

impl Archive {
    fn verify(&self) -> Result<Vec<LaunchPlan>> {
        if self.version != 1
            || self.request.models.is_empty()
            || self.request.additional_inputs.len() > 128
        {
            return Err(DomainError::InvalidInput(
                "Runtime configuration version/inventory".into(),
            ));
        }
        let mut expected = BTreeMap::from([
            ("profile".to_owned(), &self.request.profile),
            (
                "environment".to_owned(),
                &self.request.inherited_environment,
            ),
        ]);
        for (name, input) in &self.request.additional_inputs {
            if expected.insert(name.clone(), input).is_some() {
                return Err(DomainError::Conflict);
            }
        }
        if expected.len() != self.inputs.len() {
            return Err(DomainError::Conflict);
        }
        for (name, input) in expected {
            let bytes = self.inputs.get(&name).ok_or(DomainError::NotFound)?;
            if digest(bytes) != input.sha256 {
                return Err(DomainError::Conflict);
            }
        }
        let profiles: serde_json::Value = decode(&self.inputs["profile"])?;
        let profiles = profiles
            .get("models")
            .filter(|_| profiles["version"] == 1)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| DomainError::InvalidInput("Model profile inventory".into()))?;
        let environment: BTreeMap<String, String> = decode(&self.inputs["environment"])?;
        let mut seen = BTreeSet::new();
        let mut plans = Vec::new();
        let mut verifier = crate::llama::native::MaterialVerifier::default();
        for model in &self.request.models {
            if !seen.insert(model.model_id.clone()) {
                return Err(DomainError::Conflict);
            }
            let profile = profiles.get(&model.model_id).ok_or(DomainError::NotFound)?;
            let plan = resolve_launch_plan(ResolveInput {
                model_id: model.model_id.clone(),
                model_path: model.model_path.clone(),
                profile_bytes: serde_json::to_vec(profile).map_err(json)?,
                environment: environment.clone(),
                context_override: model.context_override,
                forced_moe: model.forced_moe,
                cached_moe: model.cached_moe,
                vram_mib: self.request.vram_mib,
                physical_cores: self.request.physical_cores,
                logical_cores: self.request.logical_cores,
                discovered_projector: model.discovered_projector.clone(),
                materials: model.materials.clone(),
            })?;
            if plan.command != model.expected.command
                || plan.args != model.expected.args
                || plan.environment != model.expected.environment
                || plan.requested_context != model.expected.context_window
                || plan.max_tokens != model.expected.max_tokens
            {
                return Err(DomainError::Conflict);
            }
            if !model
                .materials
                .iter()
                .any(|m| m.path == plan.command && m.kind == "executable")
                || !model
                    .materials
                    .iter()
                    .any(|m| m.path == plan.model_path && m.kind == "weight")
                || plan.projector.as_ref().is_some_and(|p| {
                    !model
                        .materials
                        .iter()
                        .any(|m| &m.path == p && m.kind == "projector")
                })
            {
                return Err(DomainError::PermissionDenied);
            }
            verifier.verify(&model.materials)?;
            plans.push(plan);
        }
        if seen != profiles.keys().cloned().collect() {
            return Err(DomainError::Conflict);
        }
        Ok(plans)
    }
}

/// Capture exact source bytes only after every launch matches its independently
/// captured TS expectation. A successful capture is still offline evidence.
pub fn import_configuration(
    destination: &Path,
    request: &ImportRequest,
) -> Result<ConfigurationCapture> {
    let destination = super::migration::new_path(destination)?;
    let mut inputs = BTreeMap::from([
        ("profile".into(), pinned(&request.profile)?),
        (
            "environment".into(),
            pinned(&request.inherited_environment)?,
        ),
    ]);
    for (name, input) in &request.additional_inputs {
        if inputs.insert(name.clone(), pinned(input)?).is_some() {
            return Err(DomainError::Conflict);
        }
    }
    let archive = Archive {
        version: 1,
        request: request.clone(),
        inputs,
    };
    let plans = archive.verify()?;
    let bytes = serde_json::to_vec(&archive).map_err(json)?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("Private runtime capture".into()));
    }
    // Check read-only sources again before publishing, including settings/vault.
    if pinned(&request.profile)? != archive.inputs["profile"]
        || pinned(&request.inherited_environment)? != archive.inputs["environment"]
    {
        return Err(DomainError::Conflict);
    }
    for (name, input) in &request.additional_inputs {
        if pinned(input)? != archive.inputs[name] {
            return Err(DomainError::Conflict);
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&destination)
        .map_err(io)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(io)?;
    File::open(destination.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    Ok(ConfigurationCapture {
        path: destination,
        sha256: digest(&bytes),
        plan_sha256: plans
            .into_iter()
            .map(|p| (p.model_id, p.plan_sha256))
            .collect(),
    })
}

impl ConfigurationCapture {
    /// Resolve private input references for a destination that does not exist
    /// yet. This does not write that destination or any original input.
    pub fn bind_host(&self, destination: &Path, config: &mut crate::client::Config) -> Result<()> {
        self.verify()?;
        if !destination.is_absolute() {
            return Err(DomainError::PermissionDenied);
        }
        let archive: Archive = decode(&bounded(&self.path, 128 * 1024 * 1024)?)?;
        let input = |name: &str| {
            archive
                .inputs
                .get(name)
                .map(|bytes| crate::client::imported::Input {
                    path: destination.join("imported").join(format!("{name}.json")),
                    sha256: digest(bytes),
                })
        };
        let inputs = crate::client::imported::Inputs {
            settings: input("settings"),
            credentials: input("credentials"),
        };
        if let Some(settings) = archive.inputs.get("settings") {
            let settings: serde_json::Value = decode(settings)?;
            if let Some(model) = settings
                .get("defaultModel")
                .and_then(serde_json::Value::as_str)
            {
                let provider = settings
                    .get("defaultProvider")
                    .and_then(serde_json::Value::as_str);
                if !config
                    .models
                    .iter()
                    .any(|m| m["id"] == model && provider.is_none_or(|p| m["provider"] == p))
                {
                    return Err(DomainError::Conflict);
                }
                config.selected = model.into();
            }
        }
        config.imported = Some(inputs);
        Ok(())
    }

    /// Bind the already verified runtime capture and explicit supervisor hash
    /// into the new destination. No process is started by configuration import.
    pub fn bind_runtime(
        &self,
        destination: &Path,
        process_host: Material,
        config: &mut crate::client::Config,
    ) -> Result<()> {
        self.bind_host(destination, config)?;
        crate::llama::native::MaterialVerifier::default()
            .verify(std::slice::from_ref(&process_host))?;
        let mut capture = self.clone();
        capture.path = destination.join("imported/runtime-configuration.json");
        config.runtime = Some(crate::client::local_runtime::Runtime {
            capture,
            process_host,
        });
        Ok(())
    }
    pub(super) fn materialize(
        &self,
        destination: &Path,
        config: &crate::client::Config,
    ) -> Result<()> {
        self.verify()?;
        let Some(inputs) = &config.imported else {
            return if config.fixture {
                Ok(())
            } else {
                Err(DomainError::PermissionDenied)
            };
        };
        let bytes = bounded(&self.path, 128 * 1024 * 1024)?;
        let archive: Archive = decode(&bytes)?;
        if let Some(runtime) = &config.runtime {
            let mut expected = self.clone();
            expected.path = destination.join("imported/runtime-configuration.json");
            if runtime.capture != expected {
                return Err(DomainError::Conflict);
            }
            private_dir(expected.path.parent().unwrap())?;
            if expected.path.exists() {
                if bounded(&expected.path, 128 * 1024 * 1024)? != bytes {
                    return Err(DomainError::Conflict);
                }
            } else {
                atomic(&expected.path, &bytes)?;
            }
        }
        for (name, input) in [
            ("settings", &inputs.settings),
            ("credentials", &inputs.credentials),
        ] {
            match (archive.inputs.get(name), input) {
                (None, None) => (),
                (Some(bytes), Some(input))
                    if input.path == destination.join("imported").join(format!("{name}.json"))
                        && input.sha256 == digest(bytes) =>
                {
                    private_dir(input.path.parent().unwrap())?;
                    if input.path.exists() {
                        if input.read()? != *bytes {
                            return Err(DomainError::Conflict);
                        }
                    } else {
                        atomic(&input.path, bytes)?;
                    }
                }
                _ => return Err(DomainError::Conflict),
            }
        }
        Ok(())
    }

    /// Recheck the private capture and all original inputs/materials. This is
    /// suitable before freeze/import/open; it does not establish control rights.
    pub fn verify(&self) -> Result<Vec<LaunchPlan>> {
        let bytes = bounded(&self.path, 128 * 1024 * 1024)?;
        if digest(&bytes) != self.sha256 {
            return Err(DomainError::Conflict);
        }
        let archive: Archive = decode(&bytes)?;
        let plans = archive.verify()?;
        if pinned(&archive.request.profile)? != archive.inputs["profile"]
            || pinned(&archive.request.inherited_environment)? != archive.inputs["environment"]
        {
            return Err(DomainError::Conflict);
        }
        for (name, input) in &archive.request.additional_inputs {
            if pinned(input)? != archive.inputs[name] {
                return Err(DomainError::Conflict);
            }
        }
        let hashes: BTreeMap<_, _> = plans
            .iter()
            .map(|p| (p.model_id.clone(), p.plan_sha256.clone()))
            .collect();
        if hashes != self.plan_sha256 {
            return Err(DomainError::Conflict);
        }
        Ok(plans)
    }
}
