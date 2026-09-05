use ag1_core::model::*;
use serde_json::{Value, json};
#[test]
fn capability_thresholds_and_native_thinking_modes_match_pinned_sdk() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/providers/capabilities.json"
    ))
    .unwrap();
    assert_eq!(oracle["profiles"].as_array().unwrap().len(), 96);
    for row in oracle["profiles"].as_array().unwrap() {
        let profile = infer_capabilities(
            row["context"].as_u64(),
            row["local"].as_bool().unwrap(),
            true,
            true,
        );
        assert_eq!(json!(profile), row["profile"]);
        assert_eq!(
            generation_max_tokens(row["context"].as_u64(), row["max"].as_u64()),
            row["generation_max_tokens"].as_u64().unwrap()
        );
        assert_eq!(
            profile.serialize_tools(),
            row["serialize_tools"].as_bool().unwrap()
        );
        assert_eq!(
            profile.volume_rails(),
            row["volume_rails"].as_bool().unwrap()
        );
    }
    assert_eq!(oracle["thinking"].as_array().unwrap().len(), 42);
    for row in oracle["thinking"].as_array().unwrap() {
        let declared: Option<DeclaredThinking> =
            serde_json::from_value(row["declared"].clone()).unwrap();
        let result = resolve_thinking(
            row["reasoning"].as_bool().unwrap(),
            declared.as_ref(),
            row["template"].as_str(),
            false,
        );
        assert_eq!(json!(result), row["resolved"], "{}", row);
    }
}
fn scope(model: &str, runtime: &str) -> ModelScope {
    ModelScope {
        provider: "llamacpp".into(),
        endpoint: "http://127.0.0.1:18080".into(),
        model: model.into(),
        runtime_fingerprint: runtime.into(),
    }
}
fn vision(verdict: VisionVerdict, source: &str) -> VisionObservation {
    VisionObservation {
        verdict,
        source: Some(source.into()),
        reason: None,
    }
}
#[test]
fn thinking_fallback_is_reprobeable_and_model_switch_cannot_reuse_old_runtime_capabilities() {
    let mut cache = CapabilityCache::default();
    let current = scope("model-a", "runtime-a");
    cache.put_thinking(current.clone(), resolve_thinking(true, None, None, true));
    assert!(cache.thinking(&current).is_none());
    let profile = resolve_thinking(true, None, Some("enable_thinking"), true);
    assert_eq!(profile.default_id, "high");
    assert_eq!(
        profile
            .modes
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["off", "minimal", "low", "medium", "high", "xhigh"]
    );
    assert!(profile.apply.contains(&"thinking_budget_tokens".into()));
    cache.put_thinking(current.clone(), profile);
    assert!(cache.thinking(&current).is_some());
    assert!(cache.thinking(&scope("model-b", "runtime-a")).is_none());
    assert!(cache.thinking(&scope("model-a", "runtime-b")).is_none());
}
#[test]
fn vision_live_props_canary_conflicts_and_asymmetric_cache_expiry_are_preserved() {
    use VisionVerdict::*;
    for verdict in [Yes, No] {
        let props = vision(verdict, "llamacpp");
        assert!(!needs_vision_canary(Some(&props), true));
        assert_eq!(
            resolve_vision(
                Some(props.clone()),
                Some(vision(if verdict == Yes { No } else { Yes }, "canary"))
            ),
            props
        );
    }
    let declared = vision(Yes, "lmstudio");
    assert!(needs_vision_canary(Some(&declared), true));
    assert!(!needs_vision_canary(Some(&declared), false));
    assert_eq!(
        resolve_vision(Some(declared.clone()), Some(vision(No, "canary"))).verdict,
        No
    );
    assert_eq!(
        resolve_vision(Some(declared), Some(vision(Unknown, "canary"))).verdict,
        Yes
    );
    assert_eq!(resolve_vision(None, None).verdict, Unknown);
    let mut cache = CapabilityCache::default();
    let key = scope("model-a", "runtime-a");
    cache.put_vision(key.clone(), vision(Unknown, "canary"), 100);
    assert!(cache.vision(&key, 100, 86400000).is_none());
    cache.put_vision(key.clone(), vision(No, "canary"), 100);
    assert!(cache.vision(&key, 900100, 86400000).is_some());
    assert!(cache.vision(&key, 900101, 86400000).is_none());
    cache.put_vision(key.clone(), vision(Yes, "canary"), 100);
    assert!(cache.vision(&key, 900101, 86400000).is_some());
    assert!(cache.vision(&key, 86400101, 86400000).is_none());
    assert!(
        cache
            .vision(&scope("model-a", "new-runtime"), 100, 86400000)
            .is_none()
    );
    assert!(cache.vision(&key, 99, 86400000).is_none());
}
