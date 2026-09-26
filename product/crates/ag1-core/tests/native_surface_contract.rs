use ag1_core::tools::{
    registry::{Origin, Registry},
    surface,
};
use serde_json::{Value, json};
fn registry() -> Registry {
    let mut r = Registry::builtins().unwrap();
    let names: Vec<_> = r
        .definitions()
        .map(|d| d.metadata["name"].as_str().unwrap().to_owned())
        .collect();
    r.activate(&names, &names.iter().cloned().collect())
        .unwrap();
    r
}
#[test]
fn work_objective_correction_preserves_explicit_action_semantics() {
    let invalid = json!({"action":"update","objective":"Current repair"});
    let error = surface::route("work", &invalid, &[]).unwrap_err();
    assert!(format!("{error:?}").contains("Use action=set"));
    let args = json!({"action":"set","objective":"Current repair","nextAction":"Check result"});
    let (target, routed) = surface::route("work", &args, &[]).unwrap().unwrap();
    assert_eq!(target, "taskState");
    assert_eq!(routed, args);
    let plan = ag1_core::tools::task_state::prepare(&[], &routed, "2026-09-13T15:00:00Z", "unused").unwrap();
    assert_eq!(plan.next.unwrap().value()["objective"], "Current repair");
    let update = json!({"action":"update","id":"entry","statement":"corrected"});
    assert_eq!(surface::route("work", &update, &[]).unwrap().unwrap().1, update);
}
#[test]
fn preferred_surface_is_smaller_and_capability_gated_without_removing_legacy_definitions() {
    let r = registry();
    let all = r.native_tools();
    let preferred = surface::preferred(all.clone());
    assert!(preferred.len() < all.len());
    for name in [
        "inspect",
        "work",
        "evidence",
        "retainEvidence",
        "releaseEvidence",
        "capability",
    ] {
        assert!(preferred.iter().any(|t| t["name"] == name));
    }
    for name in [
        "read",
        "grep",
        "find",
        "sourceContext",
        "artifact",
        "taskState",
        "todo",
        "episode",
    ] {
        assert!(!preferred.iter().any(|t| t["name"] == name));
        assert!(all.iter().any(|t| t["name"] == name));
    }
    assert!(!surface::available(&["read".into()]).contains(&"inspect".into()));
    assert!(!surface::available(&["screenshot_url".into()]).contains(&"ui".into()));
    assert!(
        r.definitions()
            .filter(|d| surface::is_facade(d.metadata["name"].as_str().unwrap()))
            .all(|d| d.origin == Origin::BuiltIn)
    );
}
#[test]
fn typed_requests_route_to_exact_canonical_arguments_and_reject_ambiguous_inputs() {
    let r = registry();
    for (name, args, target) in [
        ("inspect", json!({"path":"file.txt"}), "read"),
        ("ui", json!({"path":"pelican-bike.svg"}), "read"),
        ("inspect", json!({"query":"literal.*","path":"src"}), "grep"),
        ("inspect", json!({"glob":"**/*.rs"}), "find"),
        (
            "ui",
            json!({"url":"http://127.0.0.1:1234"}),
            "screenshot_url",
        ),
        (
            "ui",
            json!({"url":"http://127.0.0.1:1234","actions":[]}),
            "interact_then_screenshot",
        ),
        (
            "ui",
            json!({"url":"http://127.0.0.1:1234","claim":"button is visible"}),
            "verify_ui",
        ),
        ("evidence", json!({"id":"art_123"}), "artifact"),
        ("work", json!({"action":"show"}), "taskState"),
    ] {
        let call = r.prepare("op", "call", name, &args, true).unwrap();
        let (actual, args) = surface::route(name, call.args(), &[]).unwrap().unwrap();
        assert_eq!(actual, target);
        if actual == "grep" {
            assert_eq!(args["literal"], true);
            assert_eq!(args["context"], 3);
        }
    }
    for args in [
        json!({}),
        json!({"path":"file","glob":"*.rs"}),
        json!({"query":"x","offset":2}),
        json!({"glob":"*.rs","query":"x"}),
    ] {
        assert!(r.prepare("op", "call", "inspect", &args, true).is_err());
    }
    assert!(
        r.prepare(
            "op",
            "call",
            "retainEvidence",
            &json!({"id":"x","reason":"needed"}),
            true
        )
        .is_err()
    );
    assert!(
        r.prepare("op", "call", "work", &json!({"action":"observe"}), true)
            .is_err()
    );
    assert!(
        surface::route(
            "capability",
            &json!({"action":"run","name":"bash","arguments":{"command":"touch forbidden"}}),
            &[]
        )
        .is_err()
    );
    assert!(
        surface::route(
            "capability",
            &json!({"action":"run","name":"capability","arguments":{}}),
            &[]
        )
        .is_err()
    );
}
#[test]
fn metadata_never_claims_command_or_github_effects_are_read_only() {
    for name in ["bash", "job", "check", "preview", "github", "capability"] {
        let e = surface::effects(name, None);
        assert_ne!(e["workspaceWrites"], "no");
        assert_eq!(e["descriptiveOnly"], true);
    }
    let r = registry();
    let evidence = r
        .native_tools()
        .into_iter()
        .find(|t| t["name"] == "evidence")
        .unwrap();
    assert!(evidence["effects"].is_object());
    let _: Vec<Value> = surface::preferred(r.native_tools());
}

#[test]
fn native_effect_annotations_survive_provider_schema_projection_and_aliases_keep_cv_identity() {
 let r=registry();
 for tool in r.native_tools() {let e:Value=serde_json::from_str(tool["parameters"]["$comment"].as_str().unwrap()).unwrap();assert_eq!(e["ag1Effects"],tool["effects"]);ag1_core::tools::schema::Schema::parse(&tool["parameters"]).unwrap();}
 for (name,canonical) in [("inspect","read"),("evidence","artifact"),("retainEvidence","artifact"),("work","runtimeObservation"),("ui","verify_ui"),("capability","sourceContext")] {
  let m=json!({"role":"toolResult","toolName":name,"details":{"nativeTool":canonical}});assert_eq!(surface::message_tool(&m),canonical);
 }
 assert_eq!(surface::message_tool(&json!({"toolName":"read","details":{"nativeTool":"bash"}})),"read");
 assert_eq!(surface::message_tool(&json!({"toolName":"inspect","details":{"nativeTool":"github"}})),"inspect");
}

#[test]
fn local_ui_images_reject_browser_actions_and_keep_read_classification() {
    let r = registry();
    for args in [json!({"path":"a.svg","url":"http://localhost"}),json!({"path":"a.svg","actions":[]}),json!({"path":"a.svg","claim":"looks good"})] {
        assert!(r.prepare("op","call","ui",&args,true).is_err());
    }
    assert_eq!(surface::message_tool(&json!({"toolName":"ui","details":{"nativeTool":"read"}})), "read");
}
