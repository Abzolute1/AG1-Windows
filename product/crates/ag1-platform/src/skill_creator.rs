//! Approval-bound resource helper. The UI host may register this audited command;
//! it is not automatically added to model-visible or read-only tool sets.
use crate::{file_tools::FileExecution, workspace::Workspace};
use ag1_core::{
    DomainError, FileRevision,
    resources::skills::{self, Draft},
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Registry, Scope, ValidatedCall},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
pub struct SkillCreator {
    workspace: Arc<Workspace>,
    base: String,
    incarnation: u64,
}
pub struct PreparedSkill {
    workspace: Arc<Workspace>,
    scope: Scope,
    call: ValidatedCall,
    draft: Draft,
    directory: String,
    target: String,
}
pub struct SkillCreated {
    pub revision: FileRevision,
    pub outcome: AdapterOutcome,
}
impl PreparedSkill {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    pub fn content(&self) -> &str {
        self.draft.content()
    }
}
impl SkillCreator {
    pub fn new(workspace: Arc<Workspace>, base: &str) -> Result<Self, DomainError> {
        // The host supplies either .ag1/skills under a project or skills under a
        // registered user resource root. Model arguments cannot change this base.
        if base.is_empty()
            || base.len() > 4096
            || std::path::Path::new(base)
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(DomainError::InvalidInput("skill helper base".into()));
        }
        workspace.verify_root()?;
        Ok(Self {
            workspace,
            base: base.into(),
            incarnation: SERIAL.fetch_add(1, Ordering::Relaxed),
        })
    }
    pub fn register(registry: &mut Registry) -> Result<(), DomainError> {
        registry.register(json!({"name":"createSkill","description":"Create a reusable skill in the host-selected resource scope.","parameters":{"type":"object","properties":{"name":{"type":"string"},"description":{"type":"string"},"disableModelInvocation":{"type":"boolean"}},"required":["name","description"],"additionalProperties":false}}),Origin::TrustedHost)
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
    ) -> Result<PreparedSkill, DomainError> {
        if call.origin() != Origin::TrustedHost
            || call.name() != "createSkill"
            || scope.workspace != self.workspace.id().as_str()
        {
            return Err(DomainError::PermissionDenied);
        }
        let name = call.args()["name"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("skill name".into()))?;
        let description = call.args()["description"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("skill description".into()))?;
        let draft = skills::draft(
            name,
            description,
            call.args()["disableModelInvocation"] == true,
        )?;
        let directory = format!("{}/{}", self.base, draft.name());
        if self.workspace.exists(&directory)? {
            return Err(DomainError::InvalidInput(format!(
                "Skill \"{name}\" already exists at {}",
                self.workspace.root().join(&directory).display()
            )));
        }
        let target = format!(
            "{:x}",
            Sha256::digest(
                json!([
                    "skill-directory-v1",
                    self.workspace.id().as_str(),
                    self.incarnation,
                    directory,
                    call.args_hash()
                ])
                .to_string()
                .as_bytes()
            )
        );
        Ok(PreparedSkill {
            workspace: self.workspace.clone(),
            scope: scope.clone(),
            call: call.clone(),
            draft,
            directory,
            target,
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        prepared: PreparedSkill,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<SkillCreated, DomainError> {
        if !Arc::ptr_eq(&self.workspace, &prepared.workspace) || prepared.scope != *execution.scope
        {
            return Err(DomainError::PermissionDenied);
        }
        // Recompute the service incarnation binding before consuming a one-use
        // approval, so a helper replacement cannot inherit old pending work.
        let current = format!(
            "{:x}",
            Sha256::digest(
                json!([
                    "skill-directory-v1",
                    self.workspace.id().as_str(),
                    self.incarnation,
                    prepared.directory,
                    prepared.call.args_hash()
                ])
                .to_string()
                .as_bytes()
            )
        );
        if current != prepared.target {
            return Err(DomainError::Conflict);
        }
        let call = authorization.consume(
            execution.registry,
            ExecutionFence {
                scope: execution.scope,
                policy_revision: execution.policy_revision,
                target_revision: &prepared.target,
                now: (execution.now)(),
            },
        )?;
        if call.operation() != prepared.call.operation()
            || call.call_id() != prepared.call.call_id()
            || call.args_hash() != prepared.call.args_hash()
            || call.origin() != Origin::TrustedHost
        {
            return Err(DomainError::PermissionDenied);
        }
        let check = || {
            if execution.cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= execution.deadline {
                Err(DomainError::TimedOut)
            } else {
                Ok(())
            }
        };
        let revision = self.workspace.create_directory_document_checked(
            &prepared.directory,
            "SKILL.md",
            prepared.draft.content().as_bytes(),
            check,
        )?;
        let path = self
            .workspace
            .root()
            .join(&prepared.directory)
            .join("SKILL.md")
            .to_string_lossy()
            .into_owned();
        let result: Value = json!({"content":[{"type":"text","text":format!("Created skill: {path}")}],"details":{"success":true,"path":path,"sourceSha256":revision.sha256}});
        Ok(SkillCreated {
            revision,
            outcome: AdapterOutcome {
                result,
                effect: Effect::Committed,
                artifact_page: false,
            },
        })
    }
}
