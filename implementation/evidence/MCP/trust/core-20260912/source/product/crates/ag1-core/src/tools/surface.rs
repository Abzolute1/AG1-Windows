//! Preferred model interface. Routes are host-owned, canonical calls retain policy,
//! adapter and receipt identity; the transcript retains the model's original name.
use super::registry::{Origin, Registry};
use crate::DomainError;
use serde_json::{Value, json};

pub const FACADES: &[&str] = &[
    "inspect",
    "ui",
    "work",
    "evidence",
    "retainEvidence",
    "releaseEvidence",
    "capability",
];
pub const ADVANCED: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "repo",
    "sourceContext",
    "artifact",
    "taskState",
    "runtimeObservation",
    "todo",
    "episode",
    "editApply",
    "editPreview",
    "screenshot_url",
    "interact_then_screenshot",
    "verify_ui",
];
pub fn is_facade(name: &str) -> bool {
    FACADES.contains(&name)
}
pub fn dependencies(name: &str) -> &'static [&'static str] {
    match name {
        "inspect" => &["read", "grep", "find", "ls"],
        "ui" => &["read", "screenshot_url", "interact_then_screenshot", "verify_ui"],
        "work" => &["taskState", "runtimeObservation"],
        "evidence" | "retainEvidence" | "releaseEvidence" => &["artifact"],
        "capability" => &[],
        _ => &[],
    }
}
pub fn available(names: &[String]) -> Vec<String> {
    FACADES
        .iter()
        .filter(|name| {
            dependencies(name)
                .iter()
                .all(|dep| names.iter().any(|n| n == dep))
        })
        .map(|n| n.to_string())
        .collect()
}
/// Called only for a new default selection. Explicit/restored selections remain exact.
pub fn preferred(tools: Vec<Value>) -> Vec<Value> {
    let names: Vec<_> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    tools
        .iter()
        .filter(|t| {
            let name = t["name"].as_str().unwrap_or("");
            if !ADVANCED.contains(&name) {
                return true;
            }
            let replacement = match name {
                "read" | "grep" | "find" | "ls" => "inspect",
                "taskState" | "runtimeObservation" => "work",
                "artifact" => "evidence",
                "screenshot_url" | "interact_then_screenshot" | "verify_ui" => "ui",
                _ => "capability",
            };
            !names.contains(&replacement)
        })
        .cloned()
        .collect()
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn schema(registry: &Registry, name: &str) -> Value {
    registry
        .definitions()
        .find(|d| d.metadata["name"] == name)
        .unwrap()
        .metadata["parameters"]
        .clone()
}
pub fn register(registry: &mut Registry) -> Result<(), DomainError> {
    let artifact = schema(registry, "artifact");
    let mut evidence_props = json!({});
    for key in ["query", "id", "cursor", "maxBytes", "kind"] {
        evidence_props[key] = artifact["properties"][key].clone();
    }
    evidence_props["maxBytes"]["minimum"] = json!(2048);
    evidence_props["action"] = json!({"type":"string","const":"retrieve","description":"Optional compatibility field in returned nextCall."});
    let mut evidence = object(evidence_props, &[]);
    evidence["anyOf"] = json!([{"required":["query"]},{"required":["id"]},{"required":["cursor"]}]);
    let lease = object(
        json!({"id":{"type":"string","minLength":1},"reason":{"type":"string","minLength":1,"maxLength":240},"verificationScope":{"type":"string","minLength":1,"maxLength":128},"ttlMinutes":{"type":"integer","minimum":1,"maximum":1440}}),
        &["id", "reason", "verificationScope"],
    );
    let mut task = schema(registry, "taskState");
    task["properties"]["action"]["anyOf"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"string","const":"observe"}));
    let mut observation = schema(registry, "runtimeObservation");
    observation["properties"]
        .as_object_mut()
        .unwrap()
        .remove("taskObjective");
    observation["properties"]
        .as_object_mut()
        .unwrap()
        .remove("updateTaskState");
    task["properties"]["observation"] = observation;
    task["allOf"] = json!([{"anyOf":[{"properties":{"action":{"const":"observe"}},"required":["observation"]},{"properties":{"action":{"not":{"const":"observe"}}},"not":{"required":["observation"]}}]}]);
    let read = schema(registry, "read");
    let mut inspect = read.clone();
    inspect["additionalProperties"] = json!(false);
    inspect["required"] = json!([]);
    inspect["properties"]["query"] = json!({"type":"string","minLength":1,"maxLength":512,"description":"Literal text to locate; matching lines include surrounding source context."});
    inspect["properties"]["glob"] = json!({"type":"string","minLength":1,"description":"Filename glob, e.g. **/*.rs. Use without query or path."});
    inspect["oneOf"] = json!([{"required":["path"],"not":{"anyOf":[{"required":["query"]},{"required":["glob"]}]}},{"required":["query"],"not":{"anyOf":[{"required":["glob"]},{"required":["offset"]}]}},{"required":["glob"],"not":{"anyOf":[{"required":["path"]},{"required":["query"]},{"required":["offset"]}]}}]);
    let targets: Vec<Value> =
        serde_json::from_str(include_str!("../../resources/target-tool-catalog.json"))
            .map_err(|_| DomainError::Conflict)?;
    let mut ui = targets.iter().find(|t| t["name"] == "verify_ui").unwrap()["parameters"].clone();
    ui["required"] = json!([]);
    ui["properties"]["path"] = json!({"type":"string","minLength":1,"description":"Local image file to view, e.g. pelican-bike.svg. SVG is rasterized automatically. No server is needed."});
    ui["oneOf"] = json!([{"required":["path"],"not":{"anyOf":[{"required":["url"]},{"required":["actions"]},{"required":["claim"]}]}},{"required":["url"],"not":{"required":["path"]}}]);
    ui["additionalProperties"] = json!(false);
    ui["properties"]["url"]["description"] = json!(
        "Ready AG1-owned URL returned by preview status/start. Arbitrary URLs and local paths are refused."
    );
    let capability = object(
        json!({"action":{"type":"string","enum":["list","schema","run"]},"name":{"type":"string","minLength":1,"maxLength":128},"arguments":{"type":"object","additionalProperties":true}}),
        &["action"],
    );
    for (name, description, parameters) in [
        (
            "inspect",
            "Inspect current source: path reads a file; whole-file SVG/image reads include a visible image preview automatically; query returns literal matches with surrounding lines, optionally inside path; glob locates filenames. Start here for file inspection. For retained historical output use evidence.",
            inspect,
        ),
        (
            "ui",
            "View images with {path:\"pelican-bike.svg\"}: SVG/PNG/JPEG/GIF/WebP render directly into image content you can inspect. After creating or editing an image, call ui with its path, examine the returned image and iterate. No server, URL, conversion command or preview startup is needed for local images. For running web apps instead, provide an AG1-owned ready url: actions interact, claim verifies. A capture alone does not prove a claim.",
            ui,
        ),
        (
            "work",
            "Current task owner: show/reset/set/record/update/remove manage intent and claims. observe takes a runtime observation and the harness binds it to the current task and updates task state atomically. No duplicate todo, episode or task records are needed. Tool receipts remain evidence; memory is only for knowledge useful beyond this task.",
            task,
        ),
        (
            "evidence",
            "Retrieve exact retained evidence by query, id, or returned cursor, with provenance and bounded context. Pass a returned nextCall directly to this tool. The harness handles lookup and continuation; no network request is made. maxBytes bounds the entire result.",
            evidence,
        ),
        (
            "retainEvidence",
            "Retain decisive evidence until a successful check with the same verificationScope, or TTL expiry. Already retrieved evidence need not be read again. The harness binds the lease to this session and releases it only after a matching later native verifier.",
            lease,
        ),
        (
            "releaseEvidence",
            "Release retained evidence explicitly when it is no longer needed. Matching successful checks normally release scoped leases automatically.",
            object(json!({"id":{"type":"string","minLength":1}}), &["id"]),
        ),
        (
            "capability",
            "Specialist controls, used only when the preferred tools are insufficient. list returns available advanced tools and actual host permissions; schema returns the selected tool's full input schema and effects; run validates arguments and invokes that tool through its normal native permission checks. Never treats metadata as authorization.",
            capability,
        ),
    ] {
        registry.register(
            json!({"name":name,"description":description,"parameters":parameters}),
            Origin::BuiltIn,
        )?;
    }
    Ok(())
}
/// Pure routing, also used by the journal owner to independently validate derived records.
pub fn route(
    name: &str,
    args: &Value,
    branch: &[Value],
) -> Result<Option<(String, Value)>, DomainError> {
    let invalid = || {
        DomainError::InvalidInput(
            "Invalid preferred tool request; inspect its schema with capability".into(),
        )
    };
    let mut a = args.clone();
    let target = match name {
        "inspect" => {
            if let Some(query) = args.get("query") {
                a = json!({"pattern":query,"literal":true,"context":3});
                if let Some(path) = args.get("path") {
                    a["path"] = path.clone();
                }
                if let Some(limit) = args.get("limit") {
                    a["limit"] = limit.clone();
                }
                "grep"
            } else if let Some(glob) = args.get("glob") {
                a = json!({"pattern":glob});
                if let Some(limit) = args.get("limit") {
                    a["limit"] = limit.clone();
                }
                "find"
            } else if args["path"].is_string() {
                "read"
            } else {
                return Err(invalid());
            }
        }
        "ui" => {
            if let Some(path) = args.get("path") {
                if !path.is_string() || ["url", "actions", "claim"].iter().any(|k| args.get(*k).is_some()) {
                    return Err(invalid());
                }
                a = json!({"path":path});
                "read"
            } else if args.get("claim").is_some() {
                "verify_ui"
            } else if args.get("actions").is_some() {
                "interact_then_screenshot"
            } else {
                "screenshot_url"
            }
        }
        "work" => {
            if args["action"] == "observe" {
                a = args["observation"].clone();
                if !a.is_object() {
                    return Err(invalid());
                }
                if let Some(task) = crate::runtime::context::latest_task(branch) {
                    a["taskObjective"] = task.value()["objective"].clone();
                    a["updateTaskState"] = json!(true);
                } else {
                    a["updateTaskState"] = json!(false);
                }
                "runtimeObservation"
            } else {
                "taskState"
            }
        }
        "evidence" => {
            a["action"] = json!("retrieve");
            "artifact"
        }
        "retainEvidence" => {
            a["action"] = json!("lease");
            a["releaseOnVerification"] = json!(true);
            "artifact"
        }
        "releaseEvidence" => {
            a["action"] = json!("release");
            "artifact"
        }
        "capability" if args["action"] == "run" => {
            let target = args["name"]
                .as_str()
                .filter(|n| ADVANCED.contains(n))
                .ok_or_else(invalid)?;
            a = args["arguments"].clone();
            if !a.is_object() {
                return Err(invalid());
            }
            // Canonical operations only; capability cannot recursively invoke itself.
            target
        }
        _ => return Ok(None),
    };
    Ok(Some((target.into(), a)))
}
/// Conservative, descriptive metadata. Runtime policy and prepared targets decide execution.
pub fn effects(name: &str, args: Option<&Value>) -> Value {
    let state = matches!(
        name,
        "work"
            | "taskState"
            | "runtimeObservation"
            | "todo"
            | "episode"
            | "memory"
            | "retainEvidence"
            | "releaseEvidence"
            | "artifact"
            | "sourceContext"
    );
    let write = matches!(
        name,
        "write" | "edit" | "editApply" | "append" | "NotebookEdit" | "notebook_edit"
    ) && !(name == "edit" && args.is_some_and(|a| a["mode"] == "preview"));
    let command = matches!(name, "bash" | "job" | "check" | "preview");
    let external = matches!(name, "github" | "webSearch" | "webFetch");
    let visual = matches!(
        name,
        "ui" | "screenshot_url" | "verify_ui" | "interact_then_screenshot"
    );
    let read = matches!(
        name,
        "inspect" | "read" | "grep" | "find" | "ls" | "repo" | "diff" | "editPreview" | "evidence"
    );
    let unknown = !(state || write || command || external || visual || read);
    json!({"workspaceWrites":if write {"yes"}else if command||visual||unknown||name=="github" {"depends_on_request"}else{"no"},
        "sessionStateWrites":if state {"depends_on_request"}else if unknown {"unknown"}else{"no"},
        "externalService":if external {"yes"}else if command||visual||unknown {"depends_on_request"}else{"no"},
        "executesCommands":if command {"yes"}else if unknown {"unknown"}else{"no"},
        "approval":"host_policy_and_prepared_target","descriptiveOnly":true})
}

/// Preserve the existing CV classification for native facade results without
/// changing their provider-visible name or any context budget/threshold.
pub fn message_tool(message: &Value) -> &str {
    let name = message["toolName"].as_str().unwrap_or("");
    if let Some(canonical) = message["details"]["nativeTool"].as_str()
        && (dependencies(name).contains(&canonical)
            || name == "capability" && ADVANCED.contains(&canonical))
    {
        return canonical;
    }
    name
}
