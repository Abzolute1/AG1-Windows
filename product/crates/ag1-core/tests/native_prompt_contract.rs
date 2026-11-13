use ag1_core::{
    context::prompt::{ContextFile, PromptInput, PromptSkill},
    tools::registry::Registry,
};
use serde_json::json;
fn input() -> PromptInput {
    PromptInput {
        date: "2026-09-09".into(),
        cwd: "/work/current-project".into(),
        workspace_snapshot: Some("src/\nCargo.toml".into()),
        ..Default::default()
    }
}
fn activate(registry: &mut Registry, names: &[&str]) {
    let names = names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
}
#[test]
fn local_image_guidance_uses_read_without_claiming_general_browser_access() {
    let mut registry = Registry::builtins().unwrap();
    ag1_core::tools::targets::register_visual_tools(&mut registry).unwrap();
    activate(
        &mut registry,
        &[
            "read",
            "screenshot_url",
            "interact_then_screenshot",
            "verify_ui",
        ],
    );
    let prompt = format!(
        "{} {}",
        registry
            .build_native_prompt(&input(), &json!({"id":"fixture","provider":"api"}))
            .unwrap(),
        json!(registry.native_tools())
    );
    assert!(prompt.contains("PNG preview automatically"));
    assert!(prompt.contains("Image results are displayed inline"));
    assert!(prompt.contains("Not a general website or local file screenshot tool"));
    assert!(!prompt.contains("screenshot_url (HTML path"));
    assert!(!prompt.contains("Inspect a page or local HTML directly"));
    assert!(
        ag1_core::tools::shell::static_guard("python -m http.server 8000")
            .unwrap()
            .contains("Use read to preview local images")
    );
    let tools = registry.native_tools();
    let read = tools.iter().find(|t| t["name"] == "read").unwrap();
    assert!(
        read["description"]
            .as_str()
            .unwrap()
            .contains("PNG preview automatically")
    );
    let screenshot = tools
        .iter()
        .find(|t| t["name"] == "screenshot_url")
        .unwrap();
    assert!(
        screenshot["parameters"]["properties"]["url"]["description"]
            .as_str()
            .unwrap()
            .contains("Not a local file")
    );
}
#[test]
fn default_prompt_is_current_bounded_and_tracks_active_tools_and_model() {
    let mut registry = Registry::builtins().unwrap();
    let names = registry
        .definitions()
        .map(|d| d.metadata["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let model = json!({"id":"Qwen3.6-35B-A3B-UD-Q4_K_XL","provider":"llamacpp"});
    let p = registry.build_native_prompt(&input(), &model).unwrap();
    assert!(
        p.len() < 12000,
        "Default guidance grew unexpectedly: {} bytes",
        p.len()
    );
    assert!(p.contains("Qwen3.6-35B-A3B-UD-Q4_K_XL"));
    assert!(p.contains("For ordinary conversation or explanations, answer directly"));
    assert_eq!(p.matches("Current working directory:").count(), 1);
    assert!(p.contains("/work/current-project"));
    assert!(!p.contains("Cruzible") && !p.contains("context-capture"));
    assert_eq!(p, registry.build_native_prompt(&input(), &model).unwrap());
    activate(&mut registry, &["read", "grep"]);
    let narrow = registry
        .build_native_prompt(&input(), &json!({"id":"another","provider":"api"}))
        .unwrap();
    assert!(narrow.contains("Available tools: grep, read"));
    assert!(
        !narrow.contains("- bash:") && !narrow.contains("- check:") && !narrow.contains("- task:")
    );
    assert!(!narrow.contains("Qwen3.6"));
    assert!(narrow.len() < p.len());
}
#[test]
fn custom_project_append_skill_and_extension_instructions_are_preserved() {
    let mut registry = Registry::builtins().unwrap();
    registry.register_sdk(json!({"name":"special_tool","description":"Host tool","parameters":{"type":"object","properties":{}},"promptGuidelines":["Preserve this exact extension contract: alpha => beta."]})).unwrap();
    activate(&mut registry, &["read", "special_tool"]);
    let mut input = input();
    input.append_system_prompt = Some("User's appended instruction remains verbatim.".into());
    input.context_files.push(ContextFile {
        path: "/work/current-project/AGENTS.md".into(),
        content: "Use our project-specific verification procedure.".into(),
    });
    input.skills.push(PromptSkill {
        name: "my-skill".into(),
        description: "My custom skill".into(),
        file_path: "/skills/my-skill/SKILL.md".into(),
        disable_model_invocation: false,
    });
    let model = json!({"id":"model","provider":"provider"});
    let p = registry.build_native_prompt(&input, &model).unwrap();
    for rule in [
        "User's appended instruction remains verbatim.",
        "Use our project-specific verification procedure.",
        "Preserve this exact extension contract: alpha => beta.",
        "/skills/my-skill/SKILL.md",
    ] {
        assert!(p.contains(rule), "Missing {rule}");
    }
    input.custom_prompt = Some("My replacement system prompt. Keep it as written.".into());
    assert_eq!(
        registry.build_native_prompt(&input, &model).unwrap(),
        registry.build_prompt(&input).unwrap()
    );
}
#[test]
fn evidence_recovery_and_verification_rules_remain_explicit() {
    let mut registry = Registry::builtins().unwrap();
    activate(
        &mut registry,
        &[
            "artifact",
            "sourceContext",
            "taskState",
            "episode",
            "runtimeObservation",
        ],
    );
    let p = format!(
        "{} {}",
        registry.build_native_prompt(&input(), &json!({})).unwrap(),
        json!(registry.native_tools())
    );
    for rule in [
        "readCall/nextCall exactly",
        "action=retrieve",
        "omitted diff ranges",
        "Lease decisive evidence",
        "Saved capsules are historical",
        "Label assumptions",
        "Compilation is not runtime proof",
        "indexes, not evidence",
        "respect",
    ] {
        assert!(
            p.to_lowercase().contains(&rule.to_lowercase()),
            "Missing {rule}"
        );
    }
    assert!(p.contains("missing identity is unknown"));
}

#[test]
fn native_interface_keeps_contracts_and_removes_conflicting_or_duplicate_instructions() {
    let mut registry = Registry::builtins().unwrap();
    ag1_core::tools::targets::register_visual_tools(&mut registry).unwrap();
    activate(
        &mut registry,
        &[
            "read",
            "edit",
            "editApply",
            "editPreview",
            "todo",
            "explore",
            "runtimeObservation",
            "artifact",
            "screenshot_url",
        ],
    );
    let original = registry.public_tools();
    let native = registry.native_tools();
    for tool in &native {
        let public = original.iter().find(|t| t["name"] == tool["name"]).unwrap();
        let mut parameters = tool["parameters"].clone();
        // Effect annotations describe operations without changing argument validation.
        assert!(
            parameters["$comment"]
                .as_str()
                .unwrap()
                .contains("ag1Effects")
        );
        parameters.as_object_mut().unwrap().remove("$comment");
        if tool["name"] == "artifact" {
            parameters["properties"]["releaseOnVerification"]["description"] =
                public["parameters"]["properties"]["releaseOnVerification"]["description"].clone();
        }
        if matches!(tool["name"].as_str().unwrap(), "check" | "artifact") {
            parameters["properties"]
                .as_object_mut()
                .unwrap()
                .remove("verificationScope");
        }
        if matches!(
            tool["name"].as_str().unwrap(),
            "screenshot_url" | "interact_then_screenshot" | "verify_ui"
        ) {
            parameters["properties"]["url"]["description"] =
                public["parameters"]["properties"]["url"]["description"].clone();
        }
        assert_eq!(
            parameters, public["parameters"],
            "Argument contract changed for {}",
            tool["name"]
        );
    }
    let system = registry
        .build_native_prompt(&input(), &json!({"id":"fixture"}))
        .unwrap();
    let description = |name: &str| {
        native.iter().find(|t| t["name"] == name).unwrap()["description"]
            .as_str()
            .unwrap()
    };
    assert_eq!(
        description("screenshot_url")
            .matches("Inspect only the ready URL")
            .count(),
        1
    );
    assert!(description("screenshot_url").contains("preview status/start"));
    assert!(!system.contains("PNG preview automatically"));
    assert!(description("read").contains("PNG preview automatically"));
    assert!(!description("todo").contains("before completing it in a later update"));
    assert!(description("todo").contains("Existing pending items may be completed directly"));
    assert!(description("edit").contains("original file"));
    assert!(description("editApply").contains("sequential"));
    assert!(description("editPreview").contains("no apply token"));
    assert!(description("explore").contains("do not exchange live findings"));
    assert!(description("runtimeObservation").contains("taskStateEntryId"));
    assert!(description("artifact").contains("releaseOnVerification=false"));
    assert!(description("artifact").contains("same explicit verificationScope"));
    assert!(system.contains("Record a fact once"));
}

#[test]
fn only_the_exact_legacy_desktop_default_migrates_to_current_guidance() {
    let legacy = include_str!("../../../../fixtures/context/desktop-legacy-prompt.txt");
    let mut registry = Registry::builtins().unwrap();
    activate(&mut registry, &["read", "todo"]);
    let model = json!({"id":"gpt-5.6-luna","provider":"openai-codex"});
    let mut configured = input();
    configured.custom_prompt = Some(legacy.into());
    let migrated = registry.build_native_prompt(&configured, &model).unwrap();
    assert_eq!(
        migrated,
        registry.build_native_prompt(&input(), &model).unwrap()
    );
    assert!(migrated.contains("gpt-5.6-luna"));
    assert!(!migrated.contains("never repeat"));
    configured.custom_prompt = Some(format!("{legacy}\nMy deliberate custom rule."));
    assert_eq!(
        registry.build_native_prompt(&configured, &model).unwrap(),
        registry.build_prompt(&configured).unwrap()
    );
    // Measured rendered text, excluding unchanged argument schemas and source metadata.
    let names = registry.active_names();
    let descriptions: usize = registry
        .native_tools()
        .iter()
        .filter(|t| names.iter().any(|n| t["name"] == *n))
        .map(|t| t["description"].as_str().unwrap().len())
        .sum();
    eprintln!(
        "legacy_system_bytes={} current_system_bytes={} current_selected_description_bytes={descriptions}",
        legacy.len(),
        migrated.len()
    );
}
