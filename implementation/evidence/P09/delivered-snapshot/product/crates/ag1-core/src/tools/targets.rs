//! Capability-dispatched check and visual requests. P11 provides concrete targets;
//! schemas and model visibility never imply that a browser or compiler exists.
use super::registry::{Origin, Registry, ValidatedCall};
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Check,
    Screenshot,
    Interact,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisionAccess {
    pub caller: bool,
    pub judge: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub workspace: String,
    pub target: String,
    /// Configuration/source/runtime revision, supplied by the native target owner.
    pub revision: String,
    pub capabilities: BTreeSet<Capability>,
    pub vision: VisionAccess,
}
impl Binding {
    pub fn validate(&self) -> Result<(), DomainError> {
        for value in [&self.workspace, &self.target, &self.revision] {
            if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
                return Err(invalid("target binding"));
            }
        }
        Ok(())
    }
    pub fn available_tools(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        if self.capabilities.contains(&Capability::Check) {
            names.insert("check".into());
        }
        if self.capabilities.contains(&Capability::Screenshot) {
            if self.vision.caller {
                names.insert("screenshot_url".into());
                if self.capabilities.contains(&Capability::Interact) {
                    names.insert("interact_then_screenshot".into());
                }
            }
            if self.vision.caller || self.vision.judge {
                names.insert("verify_ui".into());
            }
        }
        names
    }
    pub fn admits(&self, request: &Request) -> Result<(), DomainError> {
        self.validate()?;
        if !self.available_tools().contains(&request.name)
            || (request.has_actions() && !self.capabilities.contains(&Capability::Interact))
        {
            return Err(DomainError::Unavailable("target tool capability".into()));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Request {
    name: String,
    arguments: Value,
}
impl Request {
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Original validated arguments. Adapter-specific normalization must happen in
    /// preparation and be bound to its opaque native plan before approval.
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
    pub fn has_actions(&self) -> bool {
        self.arguments["actions"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    }
    pub fn from_call(call: &ValidatedCall) -> Result<Self, DomainError> {
        if call.origin() != Origin::BuiltIn
            || ![
                "check",
                "screenshot_url",
                "interact_then_screenshot",
                "verify_ui",
            ]
            .contains(&call.name())
        {
            return Err(DomainError::PermissionDenied);
        }
        let args = call.args();
        if args.to_string().len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("target request".into()));
        }
        if call.name() == "check" {
            if let Some(files) = args.get("files") {
                let files = files.as_array().ok_or_else(|| invalid("check files"))?;
                if files.len() > 4096 {
                    return Err(DomainError::LimitExceeded("check file selection".into()));
                }
                for file in files {
                    text(file, 8192, false)?;
                }
            }
        } else {
            text(&args["url"], 8192, false)?;
            if call.name() == "verify_ui" {
                text(&args["claim"], 65536, false)?;
            }
            if let Some(viewport) = args.get("viewport") {
                for key in ["width", "height"] {
                    if viewport[key].as_u64().is_none_or(|n| n == 0 || n > 16384) {
                        return Err(invalid("viewport dimensions"));
                    }
                }
            }
            if let Some(timeout) = args.get("timeoutMs")
                && timeout.as_u64().is_none_or(|n| n == 0 || n > 300000)
            {
                return Err(invalid("visual timeout"));
            }
            if let Some(actions) = args.get("actions") {
                let actions = actions
                    .as_array()
                    .ok_or_else(|| invalid("visual actions"))?;
                if actions.len() > 256 {
                    return Err(DomainError::LimitExceeded("visual actions".into()));
                }
                let mut wait_ms = 0.;
                for action in actions {
                    match action["type"].as_str() {
                        Some("click") => {
                            text(&action["selector"], 8192, false)?;
                        }
                        Some("fill") => {
                            text(&action["selector"], 8192, false)?;
                            text(&action["value"], 65536, true)?;
                        }
                        Some("navigate") => {
                            text(&action["url"], 8192, false)?;
                        }
                        Some("wait") => {
                            let ms = action["ms"]
                                .as_f64()
                                .filter(|n| n.is_finite() && *n >= 0.)
                                .ok_or_else(|| invalid("visual wait"))?;
                            wait_ms += ms;
                            if wait_ms > 300000. {
                                return Err(DomainError::LimitExceeded("visual total wait".into()));
                            }
                        }
                        _ => return Err(invalid("visual action")),
                    }
                }
            }
        }
        Ok(Self {
            name: call.name().into(),
            arguments: args.clone(),
        })
    }
    /// Display metadata omits form contents. Original arguments remain private
    /// operation evidence under the executor's existing artifact ACL.
    pub fn display(&self) -> Value {
        let mut args = self.arguments.clone();
        if let Some(actions) = args["actions"].as_array_mut() {
            for action in actions {
                if action["type"] == "fill" {
                    action["value"] = json!("[redacted]");
                }
            }
        }
        json!({"tool":self.name,"arguments":args})
    }
}
fn text(value: &Value, max: usize, allow_empty: bool) -> Result<(), DomainError> {
    let value = value.as_str().ok_or_else(|| invalid("target text"))?;
    if value.len() > max {
        return Err(DomainError::LimitExceeded("target text".into()));
    }
    if value.contains('\0') || (!allow_empty && value.trim().is_empty()) {
        return Err(invalid("target text"));
    }
    Ok(())
}
fn invalid(message: &str) -> DomainError {
    DomainError::InvalidInput(message.into())
}
/// Register the three pinned visual schemas atomically, initially inactive. The
/// composition host selects only capabilities of an admitted current target.
pub fn register_visual_tools(registry: &mut Registry) -> Result<(), DomainError> {
    let definitions: Vec<Value> =
        serde_json::from_str(include_str!("../../resources/target-tool-catalog.json"))
            .map_err(|_| invalid("visual catalog"))?;
    let mut candidate = registry.clone();
    for definition in definitions {
        candidate.register(definition, Origin::BuiltIn)?;
    }
    *registry = candidate;
    Ok(())
}
