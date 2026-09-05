use ag1_core::{
    DomainError,
    resources::extension_providers::{HookBinding, Hooks, Registry, Snapshot},
};
use serde_json::{Value, json};
#[derive(Default)]
struct FixtureHooks {
    credential: bool,
    calls: Vec<Value>,
    fail: bool,
}
impl Hooks for FixtureHooks {
    fn modify_models(
        &mut self,
        binding: &HookBinding,
        models: &[Value],
    ) -> Result<Option<Vec<Value>>, DomainError> {
        if self.fail {
            return Err(DomainError::Unavailable("fixture hook failure".into()));
        }
        if !self.credential || binding.provider != "fixture-a" {
            return Ok(None);
        }
        self.calls.push(json!({"name":binding.provider,"access":"fixture-access","ids":models.iter().map(|m| &m["id"]).collect::<Vec<_>>()}));
        Ok(Some(
            models
                .iter()
                .map(|m| {
                    let mut m = m.clone();
                    if m["provider"] == binding.provider {
                        m["name"] = json!(format!("{} via OAuth", m["name"].as_str().unwrap()));
                    }
                    m
                })
                .collect(),
        ))
    }
}
#[test]
fn dynamic_provider_catalog_matches_sdk_replacement_reordering_headers_and_oauth_steps() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/extension-providers-oracle.json"
    ))
    .unwrap();
    let baseline: Snapshot = serde_json::from_value(oracle["baseline"].clone()).unwrap();
    let mut registry = Registry::new(baseline).unwrap();
    let mut hooks = FixtureHooks::default();
    let mut earlier_stream = None;
    for (index, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        if let Some(value) = input["oauthCredential"].as_bool() {
            hooks.credential = value;
        }
        let name = input["name"].as_str().unwrap();
        let result = if input["action"] == "unregister" {
            registry.unregister(registry.generation(), name, &mut hooks)
        } else {
            registry
                .register(
                    registry.generation(),
                    "/fixture/extension.ts",
                    name,
                    input["config"].clone(),
                    &mut hooks,
                )
                .map(|_| ())
        };
        match result {
            Ok(()) => assert!(row.get("error").is_none(), "step {index}"),
            Err(DomainError::InvalidInput(error)) => {
                assert_eq!(json!(error), row["error"], "step {index}")
            }
            Err(error) => panic!("step {index}: {error:?}"),
        }
        let snapshot = registry.snapshot();
        let actual = json!({"models":snapshot.models,"requestConfigs":snapshot.request_configs,
            "modelHeaders":snapshot.model_headers,"registered":registry.registrations().iter().map(|r| &r.binding.provider).collect::<Vec<_>>(),
            "streamOwners":snapshot.stream_bindings.iter().map(|(api,binding)| (api.clone(), json!(binding.provider))).collect::<serde_json::Map<String,Value>>(),
            "hookCalls":hooks.calls});
        assert_eq!(actual, row["state"], "step {index}: {input}");
        if index == 4 {
            earlier_stream = snapshot.stream_bindings.get("fixture-shared-api").cloned();
        }
        if (5..=7).contains(&index) {
            assert_eq!(
                snapshot.stream_bindings.get("fixture-shared-api"),
                earlier_stream.as_ref(),
                "settings replacement must preserve original hook version"
            );
        }
    }
}
#[test]
fn failed_provider_hooks_stale_generations_and_oversize_configs_never_publish() {
    let mut registry = Registry::new(Snapshot::default()).unwrap();
    let mut hooks = FixtureHooks {
        fail: true,
        ..FixtureHooks::default()
    };
    let config = json!({"baseUrl":"https://fixture.invalid","oauth":{},"api":"fixture-api","hooks":{"oauth":{"modifyModels":true}},"models":[{"id":"fixture"}]});
    assert!(matches!(
        registry.register(
            0,
            "/fixture/extension.ts",
            "fixture-a",
            config.clone(),
            &mut hooks
        ),
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(registry.generation(), 0);
    assert!(registry.snapshot().models.is_empty());
    assert!(registry.registrations().is_empty());
    hooks.fail = false;
    registry
        .register(
            0,
            "/fixture/extension.ts",
            "fixture-a",
            config.clone(),
            &mut hooks,
        )
        .unwrap();
    let before = serde_json::to_value(registry.snapshot()).unwrap();
    assert!(matches!(
        registry.register(0, "/fixture/extension.ts", "fixture-a", config, &mut hooks),
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        registry.register(
            1,
            "/fixture/extension.ts",
            "fixture-a",
            json!({"extra":"x".repeat(1024*1024)}),
            &mut hooks
        ),
        Err(DomainError::InvalidInput(_)) | Err(DomainError::LimitExceeded(_))
    ));
    assert_eq!(serde_json::to_value(registry.snapshot()).unwrap(), before);
    assert_eq!(registry.generation(), 1);
}

#[test]
fn credential_refresh_rebuilds_sdk_model_states_and_keeps_prior_catalog_on_failure() {
    struct RefreshHooks {
        available: bool,
        fail: bool,
        calls: usize,
    }
    impl Hooks for RefreshHooks {
        fn modify_models(
            &mut self,
            _: &HookBinding,
            models: &[Value],
        ) -> Result<Option<Vec<Value>>, DomainError> {
            if !self.available {
                return Ok(None);
            }
            if self.fail {
                return Err(DomainError::Unavailable("fixture failure".into()));
            }
            self.calls += 1;
            Ok(Some(
                models
                    .iter()
                    .map(|model| {
                        let mut model = model.clone();
                        if model["provider"] == "fixture" {
                            model["name"] = json!(format!(
                                "{} for fixture-account",
                                model["name"].as_str().unwrap()
                            ));
                        }
                        model
                    })
                    .collect(),
            ))
        }
    }
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/provider-refresh-oracle.json"
    ))
    .unwrap();
    let mut registry = Registry::new(Snapshot::default()).unwrap();
    let mut hooks = RefreshHooks {
        available: false,
        fail: false,
        calls: 0,
    };
    let binding = registry
        .register(
            0,
            "/fixture/extension.ts",
            "fixture",
            oracle["config"].clone(),
            &mut hooks,
        )
        .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        if row["action"] != "register" {
            hooks.available = row["action"] != "logout";
            registry.refresh(registry.generation(), &mut hooks).unwrap();
        }
        assert_eq!(json!(registry.snapshot().models), row["models"]);
        assert_eq!(json!(hooks.calls), row["calls"]);
        assert_eq!(registry.snapshot().oauth_bindings["fixture"], binding);
    }
    let generation = registry.generation();
    let before = serde_json::to_value(registry.snapshot()).unwrap();
    hooks.available = true;
    hooks.fail = true;
    assert!(registry.refresh(generation, &mut hooks).is_err());
    assert_eq!(registry.generation(), generation);
    assert_eq!(serde_json::to_value(registry.snapshot()).unwrap(), before);
    assert!(matches!(
        registry.refresh(generation - 1, &mut hooks),
        Err(DomainError::Conflict)
    ));
    assert_eq!(registry.snapshot().oauth_bindings["fixture"], binding);
}
