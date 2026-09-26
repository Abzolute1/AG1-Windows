//! Model-scoped reasoning controls. A reasoning boolean is not an effort menu.
//! Discovery is read-only and never starts a server, wakes a router or generates.
use serde_json::{Value, json};
use std::time::Duration;
const LEVELS: [&str; 8] = [
    "off", "on", "minimal", "low", "medium", "high", "xhigh", "max",
];
fn mode(id: &str) -> Value {
    json!({"id":id,"label":match id {"off"=>"Off","on"=>"On","minimal"=>"Minimal","low"=>"Low","medium"=>"Medium","high"=>"High","xhigh"=>"Extra high","max"=>"Maximum",_=>"Unknown"},"level":if id=="on"{"high"}else{id}})
}
fn profile(ids: Vec<&str>, default: &str, source: &str) -> Value {
    let mut seen = std::collections::BTreeSet::new();
    let ids: Vec<_> = ids
        .into_iter()
        .filter(|id| LEVELS.contains(id) && seen.insert(*id))
        .collect();
    if ids.is_empty() {
        return unknown();
    }
    let default = if ids.contains(&default) {
        default
    } else {
        ids[0]
    };
    json!({"kind":if ids.iter().all(|id| ["on","off"].contains(id)){"toggle"}else{"levels"},"modes":ids.iter().map(|id|mode(id)).collect::<Vec<_>>(),"default":default,"source":source})
}
pub fn unknown() -> Value {
    json!({"kind":"unknown","modes":[],"source":"unavailable"})
}
pub fn declared(model: &Value) -> Value {
    // Provider integrations publish normalized capabilities alongside the model.
    // No model-name heuristics and no synthetic levels from `reasoning: true`.
    if let Some(modes) = model["thinking"]["modes"].as_array() {
        let ids: Vec<_> = modes
            .iter()
            .filter_map(|m| m.as_str().or_else(|| m["id"].as_str()))
            .collect();
        let mut result = profile(
            ids,
            model["thinking"]["default"].as_str().unwrap_or("on"),
            "provider",
        );
        if let Some(apply) = model["thinking"].get("apply") {
            result["apply"] = apply.clone();
        }
        if let Some(normalized) = result["modes"].as_array_mut() {
            for mode in normalized {
                if let Some(native) = modes.iter().find(|m| m["id"] == mode["id"])
                    .and_then(|m| m.get("nativeValue")) {
                    mode["nativeValue"] = native.clone();
                }
            }
        }
        return result;
    }
    if model["reasoning"] == false {
        return json!({"kind":"unsupported","modes":[],"source":"provider"});
    }
    unknown()
}
pub fn from_props(model: &Value, models: &Value, props: &Value) -> Value {
    // Single-model endpoints only. Router introspection must not load another
    // model simply because the user browsed its settings.
    let Some(available) = models["data"].as_array() else {
        return unknown();
    };
    if available.len() != 1 || available[0]["id"] != model["id"] || props["is_sleeping"] == true {
        return unknown();
    }
    let Some(template) = props["chat_template"].as_str() else {
        return unknown();
    };
    let Some(p) = ag1_core::model::template_thinking(template) else {
        return unknown();
    };
    // The existing parser infers an off mode; expose it only if the template
    // actually has a switch or explicitly mentions off/none with effort.
    let can_disable = p.apply.iter().any(|v| v == "enable_thinking")
        || template
            .lines()
            .filter(|l| l.contains("reasoning_effort"))
            .any(|l| {
                ["'off'", "\"off\"", "'none'", "\"none\""]
                    .iter()
                    .any(|s| l.contains(s))
            });
    let ids: Vec<_> = p
        .modes
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| *id != "off" || can_disable)
        .collect();
    let mut result = profile(ids, &p.default_id, "llama-server");
    // Preserve native casing and `none` instead of silently changing a
    // template's enum spellings while presenting canonical UI labels.
    if result["kind"] == "levels" {
        let literals = regex::Regex::new(r#"["']([A-Za-z]+)["']"#).unwrap();
        for mode in result["modes"].as_array_mut().unwrap() {
            for line in template
                .lines()
                .filter(|line| line.contains("reasoning_effort"))
            {
                for capture in literals.captures_iter(line) {
                    let native = &capture[1];
                    let canonical = if native.eq_ignore_ascii_case("none") {
                        "off".into()
                    } else {
                        native.to_ascii_lowercase()
                    };
                    if mode["id"] == canonical {
                        mode["nativeValue"] = json!(native);
                    }
                }
            }
        }
    }
    result
}
async fn read(client: &reqwest::Client, url: reqwest::Url) -> Option<Value> {
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 262144 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).ok()
}
pub async fn discover(model: &Value) -> Value {
    let supplied = declared(model);
    let local = ["llamacpp", "llama.cpp"].contains(&model["provider"].as_str().unwrap_or(""));
    // Explicit, model-bound capabilities work before loading a local model.
    // A live template probe is only a fallback for missing metadata; it must
    // not replace a declared menu with aliases inferred from template text.
    if !local || supplied["kind"] != "unknown" {
        return supplied;
    }
    // Do not send credentials to discovery. Authenticated endpoints can supply
    // a declared profile through their provider integration instead.
    let Ok(mut url) = reqwest::Url::parse(model["baseUrl"].as_str().unwrap_or("")) else {
        return supplied;
    };
    if !url.username().is_empty() || url.password().is_some() {
        return supplied;
    }
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(1200))
        .build()
    else {
        return supplied;
    };
    let prefix = url
        .path()
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_owned();
    url.set_query(None);
    url.set_fragment(None);
    url.set_path(&format!("{prefix}/v1/models"));
    let Some(models) = read(&client, url.clone()).await else {
        return supplied;
    };
    if models["data"]
        .as_array()
        .is_none_or(|a| a.len() != 1 || a[0]["id"] != model["id"])
    {
        return supplied;
    }
    url.set_path(&format!("{prefix}/props"));
    let Some(props) = read(&client, url).await else {
        return supplied;
    };
    let found = from_props(model, &models, &props);
    if found["kind"] == "unknown" {
        supplied
    } else {
        found
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn declared_local_controls_work_before_loading_and_preserve_native_mapping() {
        let toggle = json!({"provider":"llamacpp","baseUrl":"http://127.0.0.1:1/v1","reasoning":true,"thinking":{"modes":["off","on"],"default":"on"}});
        assert_eq!(discover(&toggle).await["kind"], "toggle");
        let levels = json!({"provider":"llamacpp","baseUrl":"http://127.0.0.1:1/v1","thinking":{"apply":"reasoning_effort","modes":["off","low","medium",{"id":"xhigh","nativeValue":"xHigh"}],"default":"xhigh"}});
        let result = discover(&levels).await;
        assert_eq!(result["apply"], "reasoning_effort");
        assert_eq!(result["modes"][3]["nativeValue"], "xHigh");
        assert_eq!(result["modes"].as_array().unwrap().len(), 4);
    }
    #[test]
    fn boolean_does_not_invent_effort_levels() {
        assert_eq!(declared(&json!({"reasoning":true}))["kind"], "unknown");
    }
    #[test]
    fn templates_are_bound_to_exact_model_and_do_not_invent_budgets() {
        let model = json!({"id":"35B"});
        let catalog = json!({"data":[{"id":"35B"}]});
        let props = json!({"chat_template":"{% if enable_thinking %}think{% endif %}"});
        assert_eq!(
            from_props(&model, &catalog, &props)["modes"],
            json!([mode("off"), mode("on")])
        );
        assert_eq!(
            from_props(&json!({"id":"27B"}), &catalog, &props)["kind"],
            "unknown"
        );
        let props = json!({"chat_template":"{% if reasoning_effort in ['low', 'medium', 'high', 'xhigh'] %}think{% endif %}"});
        let p = from_props(&model, &catalog, &props);
        assert_eq!(p["modes"].as_array().unwrap().len(), 4);
        assert_eq!(p["modes"][3]["id"], "xhigh");
        assert!(
            !p["modes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["id"] == "off")
        );
    }
    #[test]
    fn provider_profiles_preserve_only_declared_levels() {
        let p = declared(&json!({"thinking":{"modes":["low","high","xhigh"],"default":"high"}}));
        assert_eq!(p["modes"].as_array().unwrap().len(), 3);
        assert_eq!(p["default"], "high");
    }
}
