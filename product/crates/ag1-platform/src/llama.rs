//! Pure, pinned llama.cpp launch resolution. No discovery process, tuning, or fallback
//! executes here. Host-only plans contain environment material and are not UI DTOs.
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[cfg(any(target_os = "linux", windows))]
pub mod custody;
pub mod discovery;
pub mod lifecycle;
mod materials;
#[cfg(target_os = "linux")]
pub mod native;
#[cfg(windows)]
#[path = "llama/native_windows.rs"]
pub mod native;
pub mod observation;
#[cfg(target_os = "linux")]
pub mod window;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextTier {
    pub max_context_window: u64,
    pub batch_size: Option<u64>,
    pub ubatch_size: Option<u64>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub server_path: Option<String>,
    pub lib_path: Option<String>,
    pub mmproj_path: Option<String>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub n_cpu_moe: Option<i64>,
    pub cache_type_k: Option<String>,
    pub cache_type_v: Option<String>,
    pub spec_type: Option<String>,
    pub fit: Option<String>,
    pub fit_target: Option<String>,
    pub load_mode: Option<String>,
    pub n_gpu_layers: Option<Value>,
    pub batch_size: Option<u64>,
    pub ubatch_size: Option<u64>,
    pub decode_threads: Option<u64>,
    pub batch_threads: Option<u64>,
    pub kv_unified: Option<bool>,
    pub kv_offload: Option<bool>,
    pub flash_attention: Option<bool>,
    pub slot_similarity_flag: Option<String>,
    pub priority: Option<Value>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub context_batch_tiers: Vec<ContextTier>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(flatten)]
    pub unknown: BTreeMap<String, Value>,
}
pub struct ResolveInput {
    pub model_id: String,
    pub model_path: String,
    pub profile_bytes: Vec<u8>,
    pub environment: BTreeMap<String, String>,
    /// A /set-context action explicitly bypasses the profile context default.
    pub context_override: Option<u64>,
    pub forced_moe: Option<i64>,
    pub cached_moe: Option<i64>,
    pub vram_mib: Option<u64>,
    pub physical_cores: u64,
    pub logical_cores: u64,
    /// Discovery is a host read-only operation. A pinned model manifest supplies the result.
    pub discovered_projector: Option<String>,
    pub materials: Vec<Material>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Material {
    pub path: String,
    pub sha256: String,
    pub kind: String,
}
/// Intentionally no Debug/Serialize: environment values can contain host secrets.
pub struct LaunchPlan {
    pub command: String,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub model_id: String,
    pub model_path: String,
    pub projector: Option<String>,
    pub requested_context: u64,
    pub max_tokens: u64,
    pub profile_sha256: String,
    pub plan_sha256: String,
    pub sources: BTreeMap<String, String>,
    pub materials: Vec<Material>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn env_number(env: &BTreeMap<String, String>, key: &str, min: u64, default: u64) -> u64 {
    env.get(key)
        .and_then(|s| s.trim().parse().ok())
        .filter(|v| *v >= min)
        .unwrap_or(default)
}
fn cache(env: &BTreeMap<String, String>, key: &str) -> String {
    let value = env
        .get(key)
        .map(|v| v.trim().to_lowercase())
        .unwrap_or_default();
    if [
        "f32", "f16", "bf16", "q8_0", "q4_0", "q4_1", "iq4_nl", "q5_0", "q5_1", "turbo2", "turbo3",
        "turbo4",
    ]
    .contains(&value.as_str())
    {
        value
    } else {
        "q8_0".into()
    }
}
fn moe_default(id: &str, vram: Option<u64>) -> Option<i64> {
    let id = id.to_lowercase();
    let bytes = id.as_bytes();
    let a_experts = bytes.windows(2).enumerate().any(|(i, w)| {
        w[0] == b'a'
            && (b'1'..=b'9').contains(&w[1])
            && bytes[i + 1..].iter().find(|b| !b.is_ascii_digit()) == Some(&b'b')
    });
    if !a_experts
        && ![
            "moe",
            "mixtral",
            "flash-next",
            "flash_next",
            "flash.next",
            "flash next",
            "flashnext",
            "qwen4exp",
        ]
        .iter()
        .any(|v| id.contains(v))
    {
        return None;
    }
    Some(match vram.unwrap_or(16384) {
        0..=8192 => 32,
        8193..=12288 => 25,
        12289..=16384 => 18,
        16385..=24576 => 12,
        _ => 8,
    })
}
pub fn resolve_launch_plan(input: ResolveInput) -> Result<LaunchPlan, DomainError> {
    let profile: Profile = serde_json::from_slice(&input.profile_bytes)
        .map_err(|_| DomainError::InvalidInput("model profile".into()))?;
    if input.physical_cores == 0 || input.logical_cores == 0 || input.logical_cores > 65536 {
        return Err(DomainError::InvalidInput("CPU topology".into()));
    }
    let env = &input.environment;
    let mut sources = BTreeMap::new();
    let command = if let Some(path) = &profile.server_path {
        sources.insert("binary".into(), "profile".into());
        path.clone()
    } else {
        sources.insert("binary".into(), "environment".into());
        env.get("AG1_LLAMA_SERVER")
            .cloned()
            .ok_or_else(|| DomainError::Unavailable("no pinned binary selection".into()))?
    };
    if !std::path::Path::new(&command).is_absolute() {
        return Err(DomainError::InvalidInput("binary must be absolute".into()));
    }
    let context = if let Some(value) = input.context_override.filter(|v| *v >= 1024) {
        sources.insert("context".into(), "explicit".into());
        value.min(262144)
    } else if let Some(value) = profile.context_window {
        sources.insert("context".into(), "profile".into());
        value
    } else {
        sources.insert("context".into(), "environment/default".into());
        env_number(
            env,
            if env.contains_key("AG1_LLAMA_CTX") {
                "AG1_LLAMA_CTX"
            } else {
                "LLAMA_CPP_CTX"
            },
            1024,
            90000,
        )
        .min(262144)
    };
    if !(1024..=262144).contains(&context) {
        return Err(DomainError::InvalidInput("profile context".into()));
    }
    let host = env
        .get("LLAMA_CPP_HOST")
        .or_else(|| env.get("AG1_LLAMA_CPP_HOST"))
        .map(String::as_str)
        .unwrap_or("127.0.0.1");
    let port = env_number(
        env,
        if env.contains_key("LLAMA_CPP_PORT") {
            "LLAMA_CPP_PORT"
        } else {
            "AG1_LLAMA_CPP_PORT"
        },
        1,
        18080,
    );
    if port > 65535 {
        return Err(DomainError::InvalidInput("port".into()));
    }
    let spec = profile.spec_type.clone().or_else(|| {
        env.get("AG1_LLAMA_SPEC")
            .map(|v| v.trim().to_string())
            .filter(|v| !["", "0", "off", "false", "none"].contains(&v.as_str()))
    });
    let projector = if spec.is_some() {
        None
    } else {
        profile.mmproj_path.clone().or(input.discovered_projector)
    };
    let tier = profile
        .context_batch_tiers
        .iter()
        .filter(|t| t.max_context_window >= context)
        .min_by_key(|t| t.max_context_window);
    let batch = tier
        .and_then(|t| t.batch_size)
        .or(profile.batch_size)
        .unwrap_or_else(|| env_number(env, "AG1_LLAMA_BATCH", 64, 256));
    let ubatch = tier
        .and_then(|t| t.ubatch_size)
        .or(profile.ubatch_size)
        .unwrap_or_else(|| env_number(env, "AG1_LLAMA_UBATCH", 64, 128));
    sources.insert(
        "batch".into(),
        if tier.is_some() {
            "context-tier"
        } else {
            "profile/environment/default"
        }
        .into(),
    );
    let decode = profile
        .decode_threads
        .unwrap_or(input.physical_cores.max(4));
    let threads_batch = profile
        .batch_threads
        .unwrap_or(input.logical_cores.min(decode * 2));
    if decode == 0 || threads_batch == 0 || batch == 0 || ubatch == 0 {
        return Err(DomainError::InvalidInput("zero launch limit".into()));
    }
    let env_moe = env
        .get("AG1_LLAMA_N_CPU_MOE")
        .and_then(|v| v.trim().parse::<i64>().ok());
    let (moe, moe_source) = if env_moe.is_some() {
        (env_moe, "environment")
    } else if input.forced_moe.is_some() {
        (input.forced_moe, "explicit")
    } else if profile.n_cpu_moe.is_some() {
        (profile.n_cpu_moe, "profile")
    } else if input.cached_moe.is_some() {
        (input.cached_moe, "vram-profile")
    } else {
        (moe_default(&input.model_id, input.vram_mib), "hardware")
    };
    sources.insert("moe".into(), moe_source.into());
    let mut args = Vec::<String>::new();
    macro_rules! pair {
        ($flag:expr,$value:expr) => {{
            args.push($flag.into());
            args.push($value.to_string());
        }};
    }
    pair!("--model", input.model_path);
    pair!("--alias", input.model_id);
    pair!("--host", host);
    pair!("--port", port);
    pair!(
        "--flash-attn",
        if profile.flash_attention == Some(false) {
            "off"
        } else {
            "on"
        }
    );
    pair!("--ctx-size", context);
    pair!(
        "--cache-type-k",
        profile
            .cache_type_k
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| cache(env, "AG1_LLAMA_CACHE_K"))
    );
    pair!(
        "--cache-type-v",
        profile
            .cache_type_v
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| cache(env, "AG1_LLAMA_CACHE_V"))
    );
    args.extend(["--no-webui".into(), "--jinja".into()]);
    if let Some(mode) = profile
        .load_mode
        .as_deref()
        .or_else(|| env.get("AG1_LLAMA_LOAD_MODE").map(|v| v.trim()))
        .filter(|v| !v.is_empty())
    {
        pair!("--load-mode", mode);
    }
    pair!("--parallel", 1);
    pair!(
        "--fit-target",
        profile.fit_target.clone().unwrap_or_else(|| env_number(
            env,
            "AG1_LLAMA_FIT_TARGET",
            64,
            512
        )
        .to_string())
    );
    pair!("--batch-size", batch);
    pair!("--ubatch-size", ubatch);
    pair!("--threads", decode);
    pair!("--threads-batch", threads_batch);
    pair!(
        "--cpu-range",
        format!("0-{}", decode.min(input.logical_cores) - 1)
    );
    pair!("--cpu-strict", 1);
    pair!(
        "--cpu-range-batch",
        format!("0-{}", threads_batch.min(input.logical_cores) - 1)
    );
    pair!("--cpu-strict-batch", 1);
    if profile.priority != Some(Value::Bool(false)) {
        pair!(
            "--prio",
            profile
                .priority
                .as_ref()
                .and_then(Value::as_i64)
                .unwrap_or(2)
        );
    }
    pair!("--poll", 100);
    if profile.slot_similarity_flag.as_deref() != Some("none") {
        pair!(
            if profile.slot_similarity_flag.as_deref() == Some("prompt") {
                "--slot-prompt-similarity"
            } else {
                "--slot-cache-key-similarity"
            },
            0
        );
    }
    if let Some(path) = &projector {
        pair!("--mmproj", path);
        args.push("--mmproj-offload".into());
    } else {
        args.push("--no-mmproj".into());
    }
    if let Some(spec) = &spec {
        pair!("--spec-type", spec);
    }
    pair!("--fit", profile.fit.as_deref().unwrap_or("off"));
    pair!(
        "--n-gpu-layers",
        profile
            .n_gpu_layers
            .as_ref()
            .map(|v| v
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string()))
            .unwrap_or("99".into())
    );
    if profile.kv_unified != Some(false) {
        args.push("--kv-unified".into());
    }
    if profile.kv_offload == Some(false) {
        args.push("--no-kv-offload".into());
    }
    if let Some(moe) = moe {
        pair!("--n-cpu-moe", moe);
    }
    args.extend(profile.extra_args);
    let library = if profile.server_path.is_some() {
        profile.lib_path
    } else {
        env_library(&input.environment)
    };
    let mut environment = input.environment;
    environment.extend(profile.environment);
    if let Some(library) = library {
        let inherited = environment.get("LD_LIBRARY_PATH").filter(|v| !v.is_empty());
        let path = inherited
            .map(|v| format!("{library}:{v}"))
            .unwrap_or(library);
        environment.insert("LD_LIBRARY_PATH".into(), path);
    }
    if command.contains('\0')
        || args.iter().any(|v| v.contains('\0'))
        || environment
            .iter()
            .any(|(k, v)| k.is_empty() || k.contains(['\0', '=']) || v.contains('\0'))
    {
        return Err(DomainError::InvalidInput("launch string".into()));
    }
    let profile_sha256 = hash(&input.profile_bytes);
    let mut plan = LaunchPlan {
        command,
        args,
        environment,
        model_id: input.model_id,
        model_path: input.model_path,
        projector,
        requested_context: context,
        max_tokens: profile.max_tokens.unwrap_or(65536),
        profile_sha256,
        plan_sha256: String::new(),
        sources,
        materials: input.materials,
    };
    plan.plan_sha256 = plan.fingerprint()?;
    Ok(plan)
}
impl LaunchPlan {
    pub(crate) fn fingerprint(&self) -> Result<String, DomainError> {
        Ok(hash(
            &serde_json::to_vec(&(
                &self.command,
                &self.args,
                &self.environment,
                &self.profile_sha256,
                &self.materials,
                &self.model_id,
                &self.model_path,
                &self.projector,
                self.requested_context,
                self.max_tokens,
                &self.sources,
            ))
            .map_err(|e| DomainError::Io(e.to_string()))?,
        ))
    }
}
fn env_library(environment: &BTreeMap<String, String>) -> Option<String> {
    environment.get("AG1_LLAMA_LIB_PATH").cloned()
}
