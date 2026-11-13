//! Sealed local-runtime configuration for the Windows-native host.
//!
//! This reads the same capture format as Linux. It intentionally stops at
//! verification: importing a Linux host configuration or binding credentials
//! is a separate Windows desktop concern, not something a launch helper may do.

use super::{Result, bounded, json, private_dir};
use crate::llama::{LaunchPlan, Material, ResolveInput, resolve_launch_plan};
use ag1_core::DomainError;
use ag1_storage::digest;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedInput {
    pub path: PathBuf,
    pub sha256: String,
}

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
    // Private input names can leak through serde diagnostics, so callers get a
    // stable generic error instead of the raw parser message.
    serde_json::from_slice(bytes)
        .map_err(|_| DomainError::InvalidInput("Private configuration format".into()))
}

impl Archive {
    fn verify_inventory(
        &self,
        check_materials: bool,
    ) -> Result<(Vec<LaunchPlan>, crate::llama::native::MaterialVerifier)> {
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
            if digest(self.inputs.get(&name).ok_or(DomainError::NotFound)?) != input.sha256 {
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
                .any(|material| material.path == plan.command && material.kind == "executable")
                || !model
                    .materials
                    .iter()
                    .any(|material| material.path == plan.model_path && material.kind == "weight")
                || plan.projector.as_ref().is_some_and(|projector| {
                    !model
                        .materials
                        .iter()
                        .any(|material| &material.path == projector && material.kind == "projector")
                })
            {
                return Err(DomainError::PermissionDenied);
            }
            if check_materials {
                verifier.verify(&model.materials)?;
            }
            plans.push(plan);
        }
        if seen != profiles.keys().cloned().collect() {
            return Err(DomainError::Conflict);
        }
        Ok((plans, verifier))
    }
}

pub fn import_configuration(
    destination: &Path,
    request: &ImportRequest,
) -> Result<ConfigurationCapture> {
    if !destination.is_absolute() || destination.exists() {
        return Err(DomainError::PermissionDenied);
    }
    private_dir(destination.parent().ok_or(DomainError::PermissionDenied)?)?;
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
    let (plans, _) = archive.verify_inventory(true)?;
    let bytes = serde_json::to_vec(&archive).map_err(json)?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("Private runtime capture".into()));
    }
    // Re-read inputs before publishing so a changed profile cannot race its
    // way into a capture whose expected plan was computed from older bytes.
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
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(super::io)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(super::io)?;
    Ok(ConfigurationCapture {
        path: destination.to_path_buf(),
        sha256: digest(&bytes),
        plan_sha256: plans
            .into_iter()
            .map(|plan| (plan.model_id, plan.plan_sha256))
            .collect(),
    })
}

impl ConfigurationCapture {
    pub fn verify(&self) -> Result<Vec<LaunchPlan>> {
        self.verify_inventory(true).map(|(plans, _)| plans)
    }

    // Opening the model catalog checks the plan and original configuration,
    // but delays hashing model weights until the selected plan is launched.
    pub(crate) fn verify_launch_plans(&self) -> Result<Vec<LaunchPlan>> {
        self.verify_inventory(false).map(|(plans, _)| plans)
    }

    fn verify_inventory(
        &self,
        check_materials: bool,
    ) -> Result<(Vec<LaunchPlan>, crate::llama::native::MaterialVerifier)> {
        let bytes = bounded(&self.path, 128 * 1024 * 1024)?;
        if digest(&bytes) != self.sha256 {
            return Err(DomainError::Conflict);
        }
        let archive: Archive = decode(&bytes)?;
        let (plans, verifier) = archive.verify_inventory(check_materials)?;
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
            .map(|plan| (plan.model_id.clone(), plan.plan_sha256.clone()))
            .collect();
        if hashes != self.plan_sha256 {
            return Err(DomainError::Conflict);
        }
        Ok((plans, verifier))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_rejects_profile_drift_and_defers_weight_hashing_when_requested() {
        let root = tempfile::Builder::new()
            .prefix("ag1w-capture-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let server = root.path().join("server.exe");
        let model = root.path().join("model.gguf");
        std::fs::write(&server, b"server").unwrap();
        std::fs::write(&model, b"model").unwrap();
        let profile = serde_json::json!({
            "serverPath":server,
            "contextWindow":32768,
            "maxTokens":4096,
            "priority":false
        });
        let write_input = |name: &str, value: serde_json::Value| {
            let bytes = serde_json::to_vec(&value).unwrap();
            let path = root.path().join(name);
            std::fs::write(&path, &bytes).unwrap();
            PinnedInput {
                path,
                sha256: digest(&bytes),
            }
        };
        let materials = vec![
            Material {
                path: server.display().to_string(),
                sha256: digest(b"server"),
                kind: "executable".into(),
            },
            Material {
                path: model.display().to_string(),
                sha256: digest(b"model"),
                kind: "weight".into(),
            },
        ];
        let plan = resolve_launch_plan(ResolveInput {
            model_id: "fixture".into(),
            model_path: model.display().to_string(),
            profile_bytes: serde_json::to_vec(&profile).unwrap(),
            environment: BTreeMap::new(),
            context_override: None,
            forced_moe: None,
            cached_moe: None,
            vram_mib: None,
            physical_cores: 1,
            logical_cores: 1,
            discovered_projector: None,
            materials: materials.clone(),
        })
        .unwrap();
        let request = ImportRequest {
            profile: write_input(
                "profile.json",
                serde_json::json!({"version":1,"models":{"fixture":profile}}),
            ),
            inherited_environment: write_input("environment.json", serde_json::json!({})),
            additional_inputs: BTreeMap::new(),
            physical_cores: 1,
            logical_cores: 1,
            vram_mib: None,
            models: vec![ModelImport {
                model_id: plan.model_id.clone(),
                model_path: plan.model_path.clone(),
                context_override: None,
                forced_moe: None,
                cached_moe: None,
                discovered_projector: None,
                materials,
                expected: ExpectedLaunch {
                    command: plan.command.clone(),
                    args: plan.args.clone(),
                    environment: plan.environment.clone(),
                    context_window: plan.requested_context,
                    max_tokens: plan.max_tokens,
                },
            }],
        };
        let capture = import_configuration(&root.path().join("capture.json"), &request).unwrap();
        assert_eq!(
            capture.verify_launch_plans().unwrap()[0].model_id,
            "fixture"
        );
        assert_eq!(capture.verify().unwrap()[0].model_id, "fixture");
        std::fs::write(&request.profile.path, b"{}").unwrap();
        assert!(matches!(
            capture.verify_launch_plans(),
            Err(DomainError::Conflict)
        ));
    }
}
