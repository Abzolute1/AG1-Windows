//! Windows composition for one pinned, owned llama.cpp runtime.
//!
//! The desktop supplies only public model metadata plus a sealed capture. This
//! module verifies that they describe the same local endpoint before it starts
//! the model process. It never adopts an already-running server as an owned one.

use crate::{
    cutover::{Registry, configuration::ConfigurationCapture, private_dir},
    llama::{Material, custody::RuntimeBundle, lifecycle::Observation, native::NativeRuntimeHost},
    provider::http::{GenerationRuntime, endpoint_origin},
};
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};

type Result<T> = std::result::Result<T, DomainError>;
type Observer = Box<dyn FnMut(&crate::llama::LaunchPlan) -> Result<Observation> + Send>;
type NativeBundle = RuntimeBundle<NativeRuntimeHost<Observer>>;

/// A sealed runtime capture plus the executable material that starts it.
///
/// This is intentionally separate from a model list entry. A model entry says
/// where requests go; this structure proves which exact process may own it.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsRuntimeConfig {
    // Each model has its own sealed capture because its GGUF file and launch
    // plan are different. They share one llama-server executable and port.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<ConfigurationCapture>,
    // Read the first native AG1W configuration format as well. New configs
    // write `captures`, but keeping this field avoids stranding an existing
    // one-model installation during the upgrade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<ConfigurationCapture>,
    pub process_host: Material,
}

impl WindowsRuntimeConfig {
    pub fn captures(&self) -> Vec<ConfigurationCapture> {
        if self.captures.is_empty() {
            self.capture.iter().cloned().collect()
        } else {
            self.captures.clone()
        }
    }

    /// Open the owned runtime lazily. No weights are loaded here; the first
    /// matching generation request calls `RuntimeBundle::prepare` and loads the
    /// selected verified plan.
    pub async fn open(&self, models: &[Value], state: &Path) -> Result<Arc<dyn GenerationRuntime>> {
        let setup = self.clone();
        let models = models.to_vec();
        let state = state.to_path_buf();
        let registry = Registry::for_user()?;
        let lease = registry.runtime_lease()?;

        let bundle = tokio::task::spawn_blocking(move || -> Result<NativeBundle> {
            // Reading a catalog must not hash every multi-gigabyte model. The
            // bundle checks the selected plan immediately before launch.
            let mut plans = Vec::new();
            for capture in setup.captures() {
                plans.extend(capture.verify_launch_plans()?);
            }
            let mut verifier = crate::llama::native::MaterialVerifier::default();
            let origin =
                crate::llama::observation::origin(plans.first().ok_or(DomainError::NotFound)?)?;

            for plan in &plans {
                if crate::llama::observation::origin(plan)? != origin {
                    return Err(DomainError::Conflict);
                }
                let model = models
                    .iter()
                    .find(|model| model["provider"] == "llamacpp" && model["id"] == plan.model_id)
                    .ok_or(DomainError::Conflict)?;
                if endpoint_origin(model["baseUrl"].as_str().ok_or(DomainError::Conflict)?)?
                    != origin
                    || model["contextWindow"].as_u64() != Some(plan.requested_context)
                    || model["maxTokens"].as_u64() != Some(plan.max_tokens)
                {
                    return Err(DomainError::Conflict);
                }
            }

            // A local URL without a matching sealed plan could point at someone
            // else's server. Refuse it instead of silently treating it as ours.
            for model in &models {
                let url =
                    reqwest::Url::parse(model["baseUrl"].as_str().ok_or(DomainError::Conflict)?)
                        .map_err(|_| DomainError::Conflict)?;
                let local = url.host_str().is_some_and(|host| {
                    host == "localhost"
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                });
                if local
                    && !plans.iter().any(|plan| {
                        model["provider"] == "llamacpp"
                            && model["id"] == plan.model_id
                            && url.origin().ascii_serialization() == origin
                    })
                {
                    return Err(DomainError::PermissionDenied);
                }
            }

            verifier.verify(std::slice::from_ref(&setup.process_host))?;
            let directory = state.join("owned-llama-runtime");
            private_dir(&directory)?;
            // This worker is outside Tauri's async runtime. Give its short
            // health probes their own runtime instead of re-entering the UI
            // runtime from a blocking thread.
            let observer: Observer = Box::new(crate::llama::observation::blocking_observer()?);
            let host = NativeRuntimeHost::with_verifier(
                Path::new(&setup.process_host.path),
                &directory,
                observer,
                verifier,
            )?;
            RuntimeBundle::new(host, plans, lease, Duration::from_secs(900))
        })
        .await
        .map_err(|_| DomainError::DurabilityUnknown)??;

        Ok(Arc::new(bundle))
    }
}
