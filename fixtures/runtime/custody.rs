//! Disposable runtime host used by P13 tests. It never launches a model process.
use ag1_core::DomainError;
use ag1_platform::llama::{
    LaunchPlan, Material, ResolveInput,
    lifecycle::{ModelIdentity, Observation, RuntimeHost},
    native::MaterialVerifier,
    resolve_launch_plan,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub struct Audit {
    pub active: Option<String>,
    pub actions: Vec<(String, String)>,
    pub launches: Vec<(
        String,
        Vec<String>,
        std::collections::BTreeMap<String, String>,
    )>,
    pub loading: bool,
    pub external: bool,
    pub maximum_active: usize,
}
pub struct FakeHost {
    pub audit: Arc<Mutex<Audit>>,
    verifier: MaterialVerifier,
}
impl FakeHost {
    pub fn new(audit: Arc<Mutex<Audit>>) -> Self {
        Self {
            audit,
            verifier: MaterialVerifier::default(),
        }
    }
}
impl RuntimeHost for FakeHost {
    type Handle = String;
    fn verify_materials(&mut self, materials: &[Material]) -> Result<(), DomainError> {
        self.verifier.verify(materials)
    }
    fn observe(&mut self, plan: &LaunchPlan) -> Result<Observation, DomainError> {
        let a = self.audit.lock().unwrap();
        if a.external {
            return Ok(Observation::Occupied);
        }
        match a.active.as_deref() {
            None => Ok(Observation::Vacant),
            Some(id) if id == plan.model_id && !a.loading => {
                Ok(Observation::Healthy(ModelIdentity {
                    path: Some(plan.model_path.clone()),
                    alias: Some(plan.model_id.clone()),
                    model: None,
                    context_window: Some(plan.requested_context),
                    vision: false,
                }))
            }
            _ => Ok(Observation::Occupied),
        }
    }
    fn spawn(&mut self, plan: &LaunchPlan) -> Result<String, DomainError> {
        let mut a = self.audit.lock().unwrap();
        assert!(
            a.active.is_none() && !a.external,
            "No concurrent runtime or external adoption"
        );
        a.active = Some(plan.model_id.clone());
        a.maximum_active = 1;
        a.actions.push(("start".into(), plan.model_id.clone()));
        a.launches.push((
            plan.command.clone(),
            plan.args.clone(),
            plan.environment.clone(),
        ));
        Ok(plan.model_id.clone())
    }
    fn owned_alive(&mut self, handle: &String) -> Result<bool, DomainError> {
        Ok(self.audit.lock().unwrap().active.as_ref() == Some(handle))
    }
    fn stop(&mut self, handle: &mut String) -> Result<(), DomainError> {
        let mut a = self.audit.lock().unwrap();
        assert_eq!(a.active.as_ref(), Some(&*handle));
        a.actions.push(("stop".into(), handle.clone()));
        a.active = None;
        Ok(())
    }
}
pub fn plans(root: &Path, port: u16, ids: &[&str], context: u64) -> Vec<LaunchPlan> {
    ids.iter().map(|id|{
        let binary=root.join(format!("{id}-server"));let weight=root.join(format!("{id}.gguf"));
        std::fs::write(&binary,b"fixture binary bytes, not executable").unwrap();
        std::fs::write(&weight,b"fixture model bytes").unwrap();
        let materials=[(&binary,"executable"),(&weight,"weight")].into_iter().map(|(path,kind)|Material {path:path.display().to_string(),sha256:ag1_storage::digest(&std::fs::read(path).unwrap()),kind:kind.into()}).collect();
        resolve_launch_plan(ResolveInput {model_id:(*id).into(),model_path:weight.display().to_string(),profile_bytes:serde_json::to_vec(&serde_json::json!({"serverPath":binary,"contextWindow":context,"maxTokens":4096,"nCpuMoe":0,"priority":false,"extraArgs":["--fixture-ordered","3","1","2"]})).unwrap(),environment:[("AG1_LLAMA_CPP_PORT".into(),port.to_string())].into(),context_override:None,forced_moe:None,cached_moe:None,vram_mib:Some(16376),physical_cores:12,logical_cores:24,discovered_projector:None,materials}).unwrap()
    }).collect()
}
