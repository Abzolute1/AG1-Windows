//! P09 target tool boundary. Concrete compiler/browser/native targets arrive in
//! P11; they cannot replace registry validation, approval fences or durable intent.
use crate::resource_fs::ScanControl;
use ag1_core::{
    DomainError,
    storage::StoragePort,
    tools::{
        approval::AuthorizedCall,
        executor::{
            self, AdapterOutcome, Execution, ExecutionArtifacts, Hooks, Outcome, ToolEvent,
        },
        registry::{Registry, Scope, ValidatedCall},
        targets::{Binding, Request},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Native capability owner. Preparation is read-only: resolve the target, captured
/// source/configuration revisions, toolchain, file/URL grants and available vision
/// path. It must not launch a process, navigate a browser or request inference.
/// Execution uses P03/P06/P11 ownership primitives; artifacts use the supplied
/// operation port. A compiler pass alone is never visual/runtime proof.
pub trait Port {
    type Plan;
    fn prepare(
        &mut self,
        request: &Request,
        control: &ScanControl<'_>,
    ) -> Result<Self::Plan, DomainError>;
    fn binding(&self, plan: &Self::Plan) -> Binding;
    /// Host-produced review metadata such as resolved compiler argv. Form values
    /// and credentials must not be included in this public display.
    fn display(&self, plan: &Self::Plan) -> Value;
    fn revalidate(
        &mut self,
        plan: &Self::Plan,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError>;
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        plan: Self::Plan,
        request: &Request,
        control: &ScanControl<'_>,
        artifacts: &mut A,
    ) -> Result<AdapterOutcome, DomainError>;
}
pub struct Service<P: Port> {
    scope: Scope,
    incarnation: u64,
    port: P,
}
pub struct Prepared<P: Port> {
    call: ValidatedCall,
    scope: Scope,
    request: Request,
    plan: P::Plan,
    binding: Binding,
    revision: String,
    display: Value,
}
impl<P: Port> Prepared<P> {
    pub fn call(&self) -> &ValidatedCall {
        &self.call
    }
    pub fn target_revision(&self) -> &str {
        &self.revision
    }
    pub fn display(&self) -> &Value {
        &self.display
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
}
pub struct Run<'a, S, A> {
    pub store: &'a S,
    pub artifacts: &'a mut A,
    pub execution: Execution<'a>,
    pub control: &'a ScanControl<'a>,
}
impl<P: Port> Service<P> {
    pub fn new(scope: Scope, port: P) -> Self {
        Self {
            scope,
            incarnation: NEXT.fetch_add(1, Ordering::Relaxed),
            port,
        }
    }
    /// Only the native composition owner can update its target adapter. Any
    /// change must be reflected by Port::binding/revalidate before execution.
    pub fn port_mut(&mut self) -> &mut P {
        &mut self.port
    }
    fn revision(&self, binding: &Binding) -> String {
        format!(
            "target-{:x}",
            Sha256::digest(json!([self.incarnation, binding]).to_string().as_bytes())
        )
    }
    pub fn prepare(
        &mut self,
        registry: &Registry,
        call: ValidatedCall,
        scope: &Scope,
        control: &ScanControl<'_>,
    ) -> Result<Prepared<P>, DomainError> {
        control.check()?;
        if scope != &self.scope {
            return Err(DomainError::Conflict);
        }
        registry.validate_current(&call)?;
        let request = Request::from_call(&call)?;
        let plan = self.port.prepare(&request, control)?;
        let binding = self.port.binding(&plan);
        binding.admits(&request)?;
        if binding.workspace != scope.workspace {
            return Err(DomainError::PermissionDenied);
        }
        self.port.revalidate(&plan, control)?;
        let display = json!({"request":request.display(),"target":binding,"preparation":self.port.display(&plan)});
        ag1_core::resources::packages::validate_settings_document(&display)?;
        if display.to_string().len() > 65536 {
            return Err(DomainError::LimitExceeded(
                "target preparation display".into(),
            ));
        }
        control.check()?;
        Ok(Prepared {
            call,
            scope: scope.clone(),
            request,
            plan,
            revision: self.revision(&binding),
            binding,
            display,
        })
    }
    /// Mandatory durable lifecycle. The adapter is entered only after acknowledged
    /// intent. Recovery of a completed result never contacts the target; uncertain
    /// intent is retained without executing it again.
    pub fn execute<S: StoragePort, A: ExecutionArtifacts>(
        &mut self,
        prepared: Prepared<P>,
        authorization: AuthorizedCall,
        run: Run<'_, S, A>,
        hooks: &mut impl Hooks,
        on_event: impl FnMut(ToolEvent),
    ) -> Result<Outcome, DomainError> {
        let Run {
            store,
            artifacts,
            execution,
            control,
        } = run;
        if prepared.scope != self.scope
            || execution.fence.scope != &self.scope
            || prepared.revision != self.revision(&prepared.binding)
            || execution.fence.target_revision != prepared.revision
            || authorization.call().operation() != prepared.call.operation()
            || authorization.call().call_id() != prepared.call.call_id()
            || authorization.call().args_hash() != prepared.call.args_hash()
            || authorization.call().name() != prepared.call.name()
        {
            return Err(DomainError::Conflict);
        }
        let registry = execution.registry;
        let fence = execution.fence;
        let clock = execution.clock;
        executor::execute_with_artifacts(
            store,
            artifacts,
            authorization,
            execution,
            hooks,
            |authorization, artifacts| {
                control.check()?;
                authorization.consume(
                    registry,
                    ag1_core::tools::approval::ExecutionFence {
                        now: clock.now_ms(),
                        ..fence
                    },
                )?;
                self.port.revalidate(&prepared.plan, control)?;
                if self.port.binding(&prepared.plan) != prepared.binding {
                    return Err(DomainError::Conflict);
                }
                prepared.binding.admits(&prepared.request)?;
                self.port
                    .execute(prepared.plan, &prepared.request, control, artifacts)
            },
            on_event,
        )
    }
}
