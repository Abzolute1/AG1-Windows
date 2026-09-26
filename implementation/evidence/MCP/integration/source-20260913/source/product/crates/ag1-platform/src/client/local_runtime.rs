//! Explicit pinned local-runtime composition. A process observation never
//! substitutes for the new supervised child handle or the user runtime lease.
use super::*;
use crate::{
    cutover::{Registry, configuration::ConfigurationCapture},
    llama::{Material, custody::RuntimeBundle, lifecycle::Observation, native::NativeRuntimeHost},
    provider::http::GenerationRuntime,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    pub capture: ConfigurationCapture,
    pub process_host: Material,
    /// Strict measurement environments can exclude every external GPU compute
    /// client. Desktop users can allow concurrent applications without granting
    /// control over their processes or relaxing model endpoint ownership.
    #[serde(
        default = "exclusive_gpu_default",
        skip_serializing_if = "is_exclusive"
    )]
    pub exclusive_gpu: bool,
}
fn exclusive_gpu_default() -> bool {
    true
}
fn is_exclusive(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_gpu_is_preserved_by_default_and_desktop_opt_out_is_explicit() {
        let original = json!({
            "capture":{"path":"/private/capture.json","sha256":"capture","plan_sha256":{}},
            "process_host":{"path":"/private/host","sha256":"host","kind":"process-host"}
        });
        let strict: Runtime = serde_json::from_value(original.clone()).unwrap();
        assert!(strict.exclusive_gpu);
        assert_eq!(serde_json::to_value(&strict).unwrap(), original);
        let mut desktop = original;
        desktop["exclusive_gpu"] = json!(false);
        let shared: Runtime = serde_json::from_value(desktop.clone()).unwrap();
        assert!(!shared.exclusive_gpu);
        assert_eq!(serde_json::to_value(shared).unwrap(), desktop);
        desktop["exclusive_gpu"] = json!("false");
        assert!(serde_json::from_value::<Runtime>(desktop).is_err());
    }
}
type Observer = Box<dyn FnMut(&crate::llama::LaunchPlan) -> Result<Observation> + Send>;
type NativeBundle = RuntimeBundle<NativeRuntimeHost<Observer>>;
impl Runtime {
    pub(super) async fn open(
        &self,
        config: &Config,
        state: &Path,
        registry: &Registry,
    ) -> Result<Arc<dyn GenerationRuntime>> {
        if config.fixture {
            return Err(DomainError::InvalidInput(
                "Fixture configuration cannot launch a tuned runtime".into(),
            ));
        }
        let setup = self.clone();
        let models = config.models.clone();
        let state = state.to_path_buf();
        let lease = registry.runtime_lease()?;
        let handle = tokio::runtime::Handle::current();
        let bundle = tokio::task::spawn_blocking(move || -> Result<NativeBundle> {
            // Opening a workspace needs its catalog, not every model's weights.
            // RuntimeBundle::ensure verifies only the selected plan before use.
            let plans = setup.capture.verify_launch_plans()?;
            let mut verifier = crate::llama::native::MaterialVerifier::default();
            let origin =
                crate::llama::observation::origin(plans.first().ok_or(DomainError::NotFound)?)?;
            for plan in &plans {
                let model = models
                    .iter()
                    .find(|m| m["provider"] == "llamacpp" && m["id"] == plan.model_id)
                    .ok_or(DomainError::Conflict)?;
                if crate::provider::http::endpoint_origin(
                    model["baseUrl"].as_str().ok_or(DomainError::Conflict)?,
                )? != origin
                    || model["contextWindow"].as_u64() != Some(plan.requested_context)
                    || model["maxTokens"].as_u64() != Some(plan.max_tokens)
                {
                    return Err(DomainError::Conflict);
                }
            }
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
            crate::cutover::private_dir(&directory)?;
            let observer: Observer = Box::new(move |plan| {
                let origin = crate::llama::observation::origin(plan)?;
                handle.block_on(async {
                    let http =
                        ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(2))?;
                    crate::llama::observation::observe(&http, &origin).await
                })
            });
            let host = NativeRuntimeHost::with_verifier(
                Path::new(&setup.process_host.path),
                &directory,
                observer,
                verifier,
            )?;
            let host = if setup.exclusive_gpu {
                host.require_exclusive_gpu()
            } else {
                host
            };
            RuntimeBundle::new(host, plans, lease, Duration::from_secs(900))
        })
        .await
        .map_err(|_| DomainError::DurabilityUnknown)??;
        Ok(Arc::new(bundle))
    }
}
