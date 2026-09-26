use ag1_core::DomainError;
use ag1_core::tools::{policy, schema::Schema};
use serde_json::{Value, json};
#[test]
fn responses_composite_call_ids_preserve_provider_identity_without_relaxing_host_ids() {
    use ag1_core::tools::registry::Registry;
    let mut registry = Registry::builtins().unwrap();
    let names = vec!["read".to_owned()];
    registry.activate(&names, &names.iter().cloned().collect()).unwrap();
    let args = json!({"path":"file.txt"});
    let id = "call_yWtmPNZFlRja363D4fBS4O4u|fc_024485519b43757b016aa36b20ff5487d2806dc351f3fb570a";
    assert_eq!(registry.prepare("operation", id, "read", &args, true).unwrap().call_id(), id);
    assert!(registry.prepare(id, "call", "read", &args, true).is_err());
    for invalid in ["|item", "call|", "call|item|extra", "call|../item", "call|item\n"] {
        assert!(registry.prepare("operation", invalid, "read", &args, true).is_err());
    }
}
#[test]
fn host_folder_policy_extends_only_explicit_paths_and_preserves_planning() {
    let mut p:policy::Policy=serde_json::from_value(json!({"mode":"autonomous","cwd":"/project"})).unwrap();
    let granted=vec!["/reference".to_owned()];
    let command=json!({"command":"cat /reference/notes.txt"});
    assert!(!policy::evaluate("bash",&command,&p,true).unwrap().allow);
    assert!(policy::evaluate_in_folders("bash",&command,&p,true,&granted).unwrap().allow);
    assert!(!policy::evaluate_in_folders("bash",&json!({"command":"cat /reference-escape/notes.txt"}),&p,true,&granted).unwrap().allow);
    assert!(!policy::evaluate_in_folders("bash",&json!({"command":"cat /reference/notes.txt /ungranted/secret"}),&p,true,&granted).unwrap().allow);
    let spaced=vec!["/reference folder".to_owned()];
    assert!(policy::evaluate_in_folders("bash",&json!({"command":"cat '/reference folder/notes.txt'"}),&p,true,&spaced).unwrap().allow);
    assert!(!policy::evaluate_in_folders("bash",&json!({"command":"cat '/reference folder'-escape/notes.txt"}),&p,true,&spaced).unwrap().allow);
    p.mode=policy::Mode::Planning;
    assert!(!policy::evaluate_in_folders("bash",&json!({"command":"printf changed > /reference/notes.txt"}),&p,true,&granted).unwrap().allow);
}
#[test]
fn missing_shell_commands_receive_immediate_recovery_and_success_resets_the_loop() {
    use ag1_core::tools::recovery::{FailureTracker, SHELL_RECOVERY};
    let mut tracker = FailureTracker::default();
    let missing = |command: &str| {
        format!("/bin/bash: {command}: command not found\nCommand exited with code 127")
    };
    assert_eq!(
        tracker.observe("bash", true, &missing("ack")).unwrap(),
        None
    );
    let second = tracker
        .observe("bash", true, &missing("phase-close"))
        .unwrap()
        .unwrap();
    assert_eq!(second, SHELL_RECOVERY);
    assert!(second.contains("Do not invent lifecycle/completion shell commands"));
    assert!(second.contains("provide the final answer now"));
    assert_eq!(
        tracker.observe("bash", true, "permission denied").unwrap(),
        None
    );
    assert_eq!(
        tracker.observe("bash", true, &missing("end")).unwrap(),
        Some(SHELL_RECOVERY)
    );
    assert_eq!(
        tracker.observe("read", true, "command not found").unwrap(),
        None
    );
    assert_eq!(
        tracker.observe("read", true, "command not found").unwrap(),
        None
    );
    assert_eq!(tracker.observe("bash", false, "ok").unwrap(), None);
    assert_eq!(
        tracker.observe("bash", true, &missing("ack")).unwrap(),
        None
    );
    tracker.reset();
    assert_eq!(
        tracker.observe("bash", true, "COMMAND NOT FOUND").unwrap(),
        None
    );
    assert_eq!(
        tracker.observe("bash", true, "COMMAND NOT FOUND").unwrap(),
        Some(SHELL_RECOVERY)
    );
    tracker.reset();
    for n in 0..256 {
        tracker
            .observe(&format!("tool{n}"), true, "tool not found")
            .unwrap();
    }
    assert!(matches!(
        tracker.observe("overflow", true, "not found"),
        Err(DomainError::LimitExceeded(_))
    ));
    tracker.observe("tool0", false, "success").unwrap();
    assert_eq!(
        tracker.observe("overflow", true, "not found").unwrap(),
        None
    );
    assert!(tracker.observe("bad\nname", true, "not found").is_err());
}
#[test]
fn every_pinned_definition_is_supported_and_rejects_invalid_required_values() {
    let rows: Value = serde_json::from_str(include_str!("../resources/tool-catalog.json")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 27);
    for row in rows.as_array().unwrap() {
        let schema =
            Schema::parse(&row["parameters"]).unwrap_or_else(|e| panic!("{}: {e:?}", row["name"]));
        assert!(schema.validate(&json!(null)).is_err(), "{}", row["name"]);
        assert!(
            schema.validate(&json!("malformed")).is_err(),
            "{}",
            row["name"]
        );
        if row["parameters"]["required"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            assert!(schema.validate(&json!({})).is_err(), "{}", row["name"]);
        }
    }
    assert!(Schema::parse(&json!({"type":"object","unknownValidationRule":true})).is_err());
    assert!(Schema::parse(&json!({"type":"string","pattern":"["})).is_err());
}
#[test]
fn modes_approvals_shell_guards_and_headless_selection_match_reference() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/policy-oracle.json"
    ))
    .unwrap();
    for row in oracle["decisions"].as_array().unwrap() {
        let i = &row["input"];
        let p: policy::Policy = serde_json::from_value(i["policy"].clone()).unwrap();
        let result = policy::evaluate(
            i["tool"].as_str().unwrap(),
            &i["args"],
            &p,
            i["untrusted"] == true,
        )
        .unwrap();
        assert_eq!(json!(result), row["output"], "{i}");
        assert_eq!(
            json!(result.approval_request.as_ref().map(policy::dialog)),
            row["dialog"]
        );
    }
    for row in oracle["planning"].as_array().unwrap() {
        assert_eq!(
            json!(policy::planning_shell(row["command"].as_str().unwrap())),
            row["allowed"],
            "{}",
            row["command"]
        );
    }
    for row in oracle["headless"].as_array().unwrap() {
        let active: Vec<String> = serde_json::from_value(row["input"]["active"].clone()).unwrap();
        let available: Vec<String> =
            serde_json::from_value(row["input"]["available"].clone()).unwrap();
        assert_eq!(json!(policy::headless(&active, &available)), row["output"]);
    }
    let available = [
        "read", "write", "edit", "bash", "task", "explore", "job", "memory", "webFetch",
    ]
    .map(str::to_string);
    assert_eq!(policy::read_only_profile(&available), ["read", "webFetch"]);
}

#[test]
fn exact_edits_fuzzy_opt_in_numbered_diffs_and_utf8_truncation_match_sdk() {
    use ag1_core::tools::{diff, edit, truncate};
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/edit-oracle.json")).unwrap();
    for row in oracle["edits"].as_array().unwrap() {
        let i = &row["input"];
        let edits: Vec<edit::Edit> = serde_json::from_value(i["edits"].clone()).unwrap();
        let out = match edit::apply(
            i["content"].as_str().unwrap(),
            &edits,
            "file.txt",
            i["fuzzy"] == true,
        ) {
            Ok(result) => {
                json!({"diff":diff::generate(&result.base_content,&result.new_content,4).unwrap(),"result":result})
            }
            Err(ag1_core::DomainError::InvalidInput(error)) => json!({"error":error}),
            Err(e) => panic!("{e:?}"),
        };
        assert_eq!(out, row["output"], "{i}");
    }
    for row in oracle["diffs"].as_array().unwrap() {
        let i = &row["input"];
        assert_eq!(
            json!(
                diff::generate(
                    i["old"].as_str().unwrap(),
                    i["new"].as_str().unwrap(),
                    i["context"].as_u64().unwrap_or(4) as usize
                )
                .unwrap()
            ),
            row["output"],
            "{i}"
        );
    }
    for row in oracle["truncation"].as_array().unwrap() {
        let i = &row["input"];
        let limits: truncate::Limits = serde_json::from_value(i["limits"].clone()).unwrap();
        for (key, tail) in [("head", false), ("tail", true)] {
            assert_eq!(
                json!(truncate::truncate(
                    i["content"].as_str().unwrap(),
                    limits,
                    tail
                )),
                row[key],
                "{key}: {i}"
            );
        }
    }
    for row in oracle["text"].as_array().unwrap() {
        let text = row["content"].as_str().unwrap();
        assert_eq!(edit::normalize_lf(text), row["lf"]);
        assert_eq!(edit::line_ending(text), row["ending"]);
        assert_eq!(edit::fuzzy(text), row["fuzzy"]);
    }
}

#[test]
fn legacy_argument_preparation_and_scalar_coercion_match_actual_sdk_validator() {
    use ag1_core::tools::{arguments, registry::Registry};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/arguments-oracle.json"
    ))
    .unwrap();
    let mut registry = Registry::builtins().unwrap();
    let names: Vec<_> = registry
        .definitions()
        .map(|d| d.metadata["name"].as_str().unwrap().to_string())
        .collect();
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    for row in oracle["cases"].as_array().unwrap() {
        let i = &row["input"];
        let name = i["name"].as_str().unwrap();
        assert_eq!(arguments::prepare(name, &i["args"]), row["prepared"], "{i}");
        let result = registry.prepare("operation", "call", name, &i["args"], true);
        // The SDK inferred deletion from an omitted replacement. Native edits
        // require an explicit string, including an explicit empty deletion.
        if i == &json!({"name":"editPreview","args":{"path":"fixture","oldText":"fixture"}}) {
            assert!(matches!(result, Err(DomainError::InvalidInput(ref error)) if error.contains("requires newText as a string")));
            continue;
        }
        assert_eq!(result.is_ok(), row["output"]["valid"], "{i}: {result:?}");
        if let Ok(call) = result {
            assert_eq!(call.args(), &row["output"]["args"], "{i}");
        }
    }
}

#[allow(dead_code)]
#[path = "support/compiler_fixture.rs"]
mod tool_fixture;
impl ag1_core::tools::result::ResultArtifacts for tool_fixture::FixtureStore {
    fn read_call(
        &self,
        id: &str,
        offset: u64,
        max_bytes: u32,
    ) -> Result<Value, ag1_core::DomainError> {
        ag1_core::context::ContextArtifactPort::get(self, id)?;
        Ok(json!({"action":"read","id":id,"offset":offset,"maxBytes":max_bytes}))
    }
}
#[test]
fn aggregate_result_cap_retains_all_original_blocks_unicode_and_images() {
    use ag1_core::{
        context::compiler::EXACT,
        tools::result::{self, ResultBudget},
    };
    for cap in [128, 400, 64000] {
        for value in ["a", "β", "🦙"] {
            let image = json!({"type":"image","mimeType":"image/png","data":"AA=="});
            let raw = json!({"content":[{"type":"text","text":value.repeat(200)},image,{"type":"text","text":value.repeat(200)},{"type":"text","text":value.repeat(200)}],"details":{"future":{"keep":true}}});
            let mut store = tool_fixture::FixtureStore::default();
            let result = result::project(
                &mut store,
                "bash",
                "call",
                &raw,
                ResultBudget {
                    text_units: cap,
                    ..Default::default()
                },
                false,
            )
            .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&store.texts[&result.original.id]).unwrap(),
                raw
            );
            assert!(result.projected_text_units <= cap);
            assert!(
                result.result["content"]
                    .as_array()
                    .unwrap()
                    .contains(&image)
            );
            if let Some(artifact) = result.text_artifacts.first() {
                let text = result.result["content"][0]["text"].as_str().unwrap();
                assert!(text.contains(EXACT), "{text}");
                assert!(text.contains(&artifact.id));
                let original = raw["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(store.texts[&artifact.id], original);
            } else {
                assert_eq!(result.result, raw);
            }
        }
    }
    let raw =
        json!({"content":[{"type":"text","text":"x".repeat(5000)}],"details":{"action":"read"}});
    let mut store = tool_fixture::FixtureStore::default();
    let page = result::project(
        &mut store,
        "artifact",
        "page",
        &raw,
        ResultBudget {
            text_units: 400,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert_eq!(page.result, raw);
    assert!(page.exempt_artifact_page);
    assert!(page.text_artifacts.is_empty());
    store.fail = true;
    assert!(
        result::project(
            &mut store,
            "bash",
            "fail",
            &raw,
            ResultBudget::default(),
            false
        )
        .is_err()
    );
}

#[test]
fn approvals_are_bound_to_arguments_epochs_revisions_expiry_and_consumed_once() {
    use ag1_core::tools::{
        approval::{Authorizer, Preflight},
        registry::{Origin, Registry, Scope},
    };
    let mut registry = Registry::builtins().unwrap();
    let names = vec!["write".to_string(), "read".to_string()];
    let available = names.iter().cloned().collect();
    registry.activate(&names, &available).unwrap();
    let p: policy::Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":"/fixture"})).unwrap();
    let scope = Scope {
        workspace: "workspace".into(),
        session: "session".into(),
        epoch: 2,
        revision: 3,
    };
    let args = json!({"path":"src/a","content":"exact approved bytes"});
    let mut book = Authorizer::new("fixture-incarnation").unwrap();
    let call = registry
        .prepare("operation", "call", "write", &args, true)
        .unwrap();
    let Preflight::Approval(request) = book
        .preflight(
            call,
            ag1_core::tools::approval::PreflightContext {
                scope: &scope,
                policy: &p,
                policy_revision: 1,
                target_revision: "file-sha",
                now: 1000,
                ttl: 1000,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!("write requires approval")
    };
    assert_eq!(book.pending().len(), 1);
    for field in [
        "argumentHash",
        "workspace",
        "session",
        "epoch",
        "revision",
        "registryRevision",
        "policyRevision",
        "targetRevision",
        "expiresMs",
    ] {
        let mut forged = json!(request.binding);
        if forged[field].is_string() {
            forged[field] = "changed".into();
        } else {
            forged[field] = 99999.into();
        }
        assert!(
            book.answer(
                &serde_json::from_value(forged).unwrap(),
                true,
                &registry,
                ag1_core::tools::approval::ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: "file-sha",
                    now: 1001
                }
            )
            .is_err()
        );
    }
    let mut changed = scope.clone();
    changed.epoch += 1;
    assert!(
        book.answer(
            &request.binding,
            true,
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &changed,
                policy_revision: 1,
                target_revision: "file-sha",
                now: 1001
            }
        )
        .is_err()
    );
    assert!(
        book.answer(
            &request.binding,
            true,
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: "different-file",
                now: 1001
            }
        )
        .is_err()
    );
    assert!(
        book.answer(
            &request.binding,
            true,
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: "file-sha",
                now: 2000
            }
        )
        .is_err()
    );
    let ticket = book
        .answer(
            &request.binding,
            true,
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: "file-sha",
                now: 1001,
            },
        )
        .unwrap()
        .unwrap();
    assert!(
        book.answer(
            &request.binding,
            true,
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: "file-sha",
                now: 1001
            }
        )
        .is_err()
    );
    let call = ticket
        .consume(
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: "file-sha",
                now: 1002,
            },
        )
        .unwrap();
    assert_eq!(call.args(), &args);
    assert!(
        registry
            .prepare("truncated", "call", "write", &args, false)
            .is_err()
    );
    assert!(
        registry
            .prepare("unknown", "call", "not-registered", &args, true)
            .is_err()
    );
    assert!(
        registry
            .register(
                json!({"name":"read","description":"untrusted replacement","parameters":{}}),
                Origin::Extension
            )
            .is_err()
    );
    let call = registry
        .prepare("reload", "call", "write", &args, true)
        .unwrap();
    let Preflight::Approval(request) = book
        .preflight(
            call,
            ag1_core::tools::approval::PreflightContext {
                scope: &scope,
                policy: &p,
                policy_revision: 1,
                target_revision: "sha",
                now: 1000,
                ttl: 1000,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    registry.activate(&["read".into()], &available).unwrap();
    assert!(
        book.answer(
            &request.binding,
            true,
            &registry,
            ag1_core::tools::approval::ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: "sha",
                now: 1001
            }
        )
        .is_err()
    );
    book.cancel("reload");
    assert!(book.pending().is_empty());
}

#[test]
fn structured_git_diff_preserves_reference_hunks_and_labels_global_truncation() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/git-oracle.json")).unwrap();
    let mut count = 0;
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        // The SDK shell loses the whitespace-containing commit path; the native argv
        // boundary corrects it. Missing refs are adapter errors, not parser input.
        if [6, 13].contains(&i) {
            continue;
        }
        let mut expected = row["output"].clone();
        if i == 8 {
            expected["details"]["truncated"] = json!(true);
            let text = expected["content"][0]["text"]
                .as_str()
                .unwrap()
                .replace("\"truncated\": false", "\"truncated\": true");
            expected["content"][0]["text"] = json!(text);
        }
        assert_eq!(
            ag1_core::tools::git_diff::project(
                row["raw"].as_str().unwrap(),
                row["input"]["maxChanges"].as_u64().unwrap_or(500) as usize
            )
            .unwrap(),
            expected,
            "Git case {i}"
        );
        count += 1;
    }
    assert_eq!(count, 12);
}

// The immutable SDK captures predate native target ownership and the separation
// of source coverage from task completion. Pin only these deliberate wording
// changes; all other result fields and transcript bytes still match the capture.
fn native_guidance_expectation(value: &Value) -> Value {
    const CHANGES: &[(&str, &str)] = &[
        ("Continue pricing research only if model, input price, output price, or currency is missing. Prefer an official pricing page for any missing price.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Continue release research only for missing version, date, change summary, or migration notes. Migration notes are required only when the user asks about upgrading or breaking changes.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Coverage complete for a brief current-events answer.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Coverage complete for a concise docs/API answer.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Coverage complete for a pricing answer. Reuse this source unless the user requested corroboration.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Coverage complete for a release/changelog answer.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Coverage complete for an error-cause-and-fix answer.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),

        ("Only continue docs research for the missing API slot. If signature, parameter, and behavior are present, answer now; examples or version notes are optional unless requested.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("Only continue web research for the missing coverage items, not by repeating the same broad query.", "Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims."),
        ("This authoritative current-events source is likely sufficient for a brief summary. Answer from it now unless the user explicitly asked for corroboration or a key fact is still missing.", "This extract is from an authoritative source; verify that its facts address the active task."),
        ("If the user asked for the strongest source, this fetch is enough. Reuse this source directly instead of fetching another article first.", "Source classification and coverage describe this extract, not completion of the active task."),
        ("This authoritative docs page is likely sufficient for a brief API or parameter answer. Do not refetch the same URL unless the requested concept is still missing.", "This extract is from an authoritative documentation page; its applicability depends on the active task and version."),
        ("This source summary is likely sufficient to answer or cite without refetching.", "This source extract is available as evidence for the active task."),
        ("Refused: do not start a static file server to take verification screenshots. For static HTML/CSS/JS, pass the file path (e.g. index.html). Do not start a Python, Node, or PHP server just to take a screenshot. Use screenshot_url or verify_ui with the HTML path. Start a server only for apps that actually need one (Vite, Next, an API).", "Refused: do not start a static file server just to take screenshots. Browser verification requires a ready AG1-owned web target started through preview start or the host Runtime UI; bash/job servers and local HTML paths do not grant browser access. Use read to preview local images, including simple vector SVG. Use the editor Preview for static HTML. Start a server only for apps that actually need one (Vite, Next, an API)."),
    ];
    match value {
        Value::String(text) => Value::String(text.split('\n').map(|line| {
            CHANGES.iter().find(|(old, _)| *old == line).map_or(line, |(_, new)| *new)
        }).collect::<Vec<_>>().join("\n")),
        Value::Array(rows) => Value::Array(rows.iter().map(native_guidance_expectation).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(key, value)| (key.clone(), native_guidance_expectation(value))).collect()),
        _ => value.clone(),
    }
}

#[test]
fn shell_guards_outputs_and_templates_match_the_executable_reference() {
    use ag1_core::tools::shell;
    assert_eq!(shell::quote("printf 'hello'"), "'printf '\\''hello'\\'''");
    assert_eq!(
        shell::sandbox_template(
            "sandbox --cwd {cwd} -- bash -lc {command}",
            "printf 'hello'",
            "/tmp/with space"
        )
        .unwrap(),
        "sandbox --cwd '/tmp/with space' -- bash -lc 'printf '\\''hello'\\'''"
    );
    assert_eq!(
        shell::sandbox_template("sandbox --cwd {cwd}", "npm test", "/repo").unwrap(),
        "sandbox --cwd '/repo' 'npm test'"
    );
    for command in [
        "sleep 120",
        "sleep 180 && echo done",
        "sleep 60; echo done",
        "sleep -- 90",
    ] {
        assert!(
            shell::wait_guard(command)
                .unwrap()
                .to_lowercase()
                .contains("job wait"),
            "{command}"
        );
    }
    for command in [
        "sleep 2",
        "sleep 0.5",
        "ls -la",
        "node --test test/*.test.js",
    ] {
        assert!(shell::wait_guard(command).is_none(), "{command}");
    }
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/shell-oracle.json")).unwrap();
    for (i, row) in oracle["guards"].as_array().unwrap().iter().enumerate() {
        let command = row["command"].as_str().unwrap();
        assert_eq!(
            json!(shell::static_guard(command)),
            native_guidance_expectation(&row["static"]),
            "static {i}"
        );
        assert_eq!(
            json!(shell::archive_guard(command)),
            row["archive"],
            "archive {i}"
        );
        assert_eq!(json!(shell::wait_guard(command)), row["wait"], "wait {i}");
        assert_eq!(
            json!(shell::install_guard(command)),
            row["install"],
            "install {i}"
        );
        assert_eq!(
            json!(shell::write_targets(command)),
            row["targets"],
            "targets {i}"
        );
    }
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let bytes: Vec<u8> = if let Some(bytes) = input["bytes"].as_array() {
            bytes
                .iter()
                .flat_map(|v| {
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|b| b.as_u64().unwrap() as u8)
                })
                .collect()
        } else {
            input["chunks"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|s| s.as_str().unwrap().bytes())
                .collect()
        };
        let exit = match input["error"].as_str() {
            Some("aborted") => shell::Exit::Cancelled,
            Some("timeout:0.1") => shell::Exit::Timeout(0.1),
            _ => match input.get("exit") {
                Some(Value::Null) => shell::Exit::Signal,
                Some(v) => shell::Exit::Code(v.as_i64().unwrap() as i32),
                None => shell::Exit::Code(0),
            },
        };
        let result = shell::render(input["command"].as_str().unwrap(), &bytes, exit, None).unwrap();
        if let Some(error) = row["output"]["error"].as_str() {
            assert_eq!(result["isError"], true);
            assert_eq!(result["content"][0]["text"], error, "shell {i}");
        } else {
            assert_eq!(result, row["output"]["result"], "shell {i}");
        }
    }
    for row in oracle["templates"].as_array().unwrap() {
        assert_eq!(
            shell::sandbox_template(
                row["template"].as_str().unwrap(),
                row["command"].as_str().unwrap(),
                row["cwd"].as_str().unwrap()
            )
            .unwrap(),
            row["output"].as_str().unwrap()
        );
    }
    assert_eq!(oracle["guards"].as_array().unwrap().len(), 34);
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 13);
}

#[test]
fn job_status_formatting_matches_reference_and_keeps_unknown_outcomes_explicit() {
    use ag1_core::tools::jobs::{Record, Status};
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/job-oracle.json")).unwrap();
    for row in oracle["formats"].as_array().unwrap() {
        let status = match row["status"].as_str().unwrap() {
            "running" => Status::Running,
            "exited" => Status::Exited,
            _ => Status::Unknown,
        };
        let record = Record {
            id: "fixture".into(),
            operation: "op".into(),
            call_id: "call".into(),
            command: row["command"].as_str().unwrap().into(),
            owner: "owner".into(),
            epoch: 1,
            started_at: 8000,
            status,
            pid: None,
            exit_code: Some(3),
            finished_at: None,
            result: None,
            notified: false,
        };
        let expected = row["output"].as_str().unwrap();
        let expected = if status == Status::Unknown {
            expected.replace("exited 3", "outcome unknown")
        } else {
            expected.into()
        };
        assert_eq!(record.line(10000), expected);
    }
    assert_eq!(oracle["formats"].as_array().unwrap().len(), 9);
}

#[test]
fn streamed_text_windows_preserve_sdk_results_across_utf8_chunk_boundaries() {
    use ag1_core::tools::read::TextWindow;
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/read-oracle.json")).unwrap();
    let mut count = 0;
    for row in oracle["rows"].as_array().unwrap() {
        for chunk_size in [1, 2, 3, 7, 65536] {
            let mut window = TextWindow::new(&row["input"]["args"]).unwrap();
            for chunk in row["input"]["text"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks(chunk_size)
            {
                window.push(chunk).unwrap();
                assert!(window.resident_bytes() <= 512 * 1024);
            }
            let actual = window.finish();
            if let Some(error) = row["output"]["error"].as_str() {
                assert!(matches!(actual,Err(DomainError::InvalidInput(ref s)) if s==error));
            } else {
                let mut expected = row["output"]["result"].clone();
                expected["details"]
                    .as_object_mut()
                    .unwrap()
                    .remove("inspectionEvidence");
                assert!(actual.unwrap() == expected, "stream boundary {chunk_size}");
            }
            count += 1;
        }
    }
    assert_eq!(count, 75);
}

fn research_case_value(row: &serde_json::Value) -> serde_json::Value {
    use ag1_core::tools::research as r;
    use serde_json::json;
    let name = row["name"].as_str().unwrap();
    let a = &row["args"];
    let s = |i: usize| a[i].as_str().unwrap_or_default();
    let n = |i: usize| a[i].as_u64().unwrap_or_default() as usize;
    match name {
        "parseDuckDuckGoResultsForTests" => {
            json!(ag1_core::tools::websearch::parse_duckduckgo(s(0), n(1)).unwrap())
        }
        "detectResearchQueryIntent" => json!(r::intent(s(0))),
        "isCodeLookupQuery" => json!(r::code_lookup(s(0))),
        "shouldUseRichResearchFetch" => json!(r::rich_fetch(s(0), s(1))),
        "deriveResearchCompressionProfile" => json!(r::profile(
            a[0]["contextWindow"].as_f64().unwrap_or_default(),
            a[0]["percent"].as_f64().unwrap_or_default()
        )),
        "getEffectiveFetchCompressionProfile" => json!(r::effective_profile(
            serde_json::from_value(a[0].clone()).unwrap(),
            a[1]["richFetch"].as_bool().unwrap_or(false)
        )),
        "truncateResearchSnippet" => json!(r::snippet(s(0), n(1))),
        "limitPreviewLines" => json!(r::limit_preview(s(0), n(1), n(2))),
        "extractResearchPrimaryHtml" => json!(r::primary_html(s(0))),
        "extractResearchDocumentTitle" => json!(r::title(s(0))),
        "extractResearchMetaDescription" => json!(r::meta_description(s(0))),
        "buildResearchPreview" => json!(r::preview(
            s(0),
            n(1),
            n(2),
            &serde_json::from_value::<Vec<String>>(a[3].clone()).unwrap_or_default()
        )),
        "extractResearchFacts" => json!(r::facts(
            s(0),
            n(1),
            n(2),
            &serde_json::from_value::<Vec<String>>(a[3].clone()).unwrap_or_default()
        )),
        "assessResearchCoverage" => json!(r::coverage(
            s(0),
            a[1]["intent"].as_str(),
            a[1]["coverageKind"].as_str()
        )),
        "assessCurrentEventsCoverage" => {
            let mut v = r::coverage(s(0), None, Some("current_events")).unwrap();
            v.kind = None;
            json!(v)
        }
        "assessResearchFetchContent" => {
            json!(r::assess(&serde_json::from_value(a[0].clone()).unwrap()))
        }
        "formatFetchTranscriptContent" => json!(r::fetch_transcript(
            &serde_json::from_value(a[0].clone()).unwrap()
        )),
        "formatSearchTranscriptContent" => json!(r::search_transcript(
            s(0),
            &serde_json::from_value::<Vec<r::SearchResult>>(a[1].clone()).unwrap(),
            &serde_json::from_value(a[2].clone()).unwrap(),
            &serde_json::from_value(if a[3].is_null() {
                json!({})
            } else {
                a[3].clone()
            })
            .unwrap()
        )),
        _ => panic!("unselected reference research case {name}"),
    }
}

#[test]
fn research_intent_compression_extraction_coverage_and_transcripts_match_reference() {
    use serde_json::Value;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/research-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 307);
    for (index, row) in rows.iter().enumerate() {
        let name = row["name"].as_str().unwrap();
        let value = research_case_value(row);
        assert_eq!(
            value, if name == "formatFetchTranscriptContent" { native_guidance_expectation(&row["result"]) } else { row["result"].clone() },
            "research case {index} {name}: {}",
            row["args"]
        );
    }
}

#[test]
fn selected_original_research_cases_match_sdk_facts_coverage_compression_and_guidance() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-research-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 27);
    for row in oracle["rows"].as_array().unwrap() {
        for call in row["calls"].as_array().unwrap() {
            assert_eq!(
                research_case_value(call),
                if call["name"] == "formatFetchTranscriptContent" { native_guidance_expectation(&call["result"]) } else { call["result"].clone() },
                "{}: {}",
                row["name"],
                call["name"]
            );
        }
    }
}

#[test]
fn complete_webfetch_projection_matches_reference_html_binary_and_unicode_cases() {
    use ag1_core::tools::research as r;
    use serde_json::Value;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/webfetch-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for (index, row) in rows.iter().take(10).enumerate() {
        let input = &row["input"];
        let bytes = if let Some(encoded) = input["bodyBase64"].as_str() {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).unwrap()
        } else {
            input["body"].as_str().unwrap().as_bytes().to_vec()
        };
        let content_type = if input.get("headers").is_some() {
            input["headers"]["content-type"]
                .as_str()
                .unwrap_or("text/plain;charset=UTF-8")
        } else {
            "text/html"
        };
        let hint = input
            .get("hint")
            .map(|v| serde_json::from_value::<r::UrlHint>(v.clone()).unwrap());
        let projection = r::fetch_projection(
            r::FetchedDocument {
                url: input["url"].as_str().unwrap(),
                final_url: input["url"].as_str().unwrap(),
                status_code: 200,
                content_type: Some(content_type),
                cache_hit: false,
                bytes: &bytes,
            },
            r::profile(
                input["usage"]["contextWindow"].as_f64().unwrap_or_default(),
                input["usage"]["percent"].as_f64().unwrap_or_default(),
            ),
            hint.as_ref(),
        )
        .unwrap();
        let mut expected = row["output"].clone();
        expected["content"] = native_guidance_expectation(&expected["content"]);
        assert_eq!(
            projection.result, expected,
            "webfetch projection {index}"
        );
    }
}

#[test]
fn websearch_plans_ranking_fallbacks_hints_and_html_parser_match_reference() {
    use ag1_core::tools::{research, websearch as w};
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/websearch-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 42);
    let parser = oracle["parse"].as_array().unwrap();
    assert_eq!(parser.len(), 18);
    for (index, row) in parser.iter().enumerate() {
        assert_eq!(
            json!(
                w::parse_duckduckgo(
                    row["html"].as_str().unwrap(),
                    row["max"].as_u64().unwrap() as usize
                )
                .unwrap()
            ),
            row["result"],
            "DDG parser {index}"
        );
    }
    for (index, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let details = &row["output"]["details"];
        let max = input["maxResults"].as_u64().unwrap_or(3).min(4) as usize;
        let source: Vec<w::ResultRow> = serde_json::from_value(input["results"].clone()).unwrap();
        let html = source
            .iter()
            .map(|r| {
                format!(
                    "<a class=\"result__a\" href=\"{}\">{}</a><a class=\"result__snippet\">{}</a>",
                    r.url, r.title, r.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let results = w::parse_duckduckgo(&html, max).unwrap();
        let response = w::ProviderResponse {
            results,
            attempts: serde_json::from_value(details["searchAttempts"].clone()).unwrap(),
            provider: if details["searchProvider"] == "official-fallback" {
                details["searchAttempts"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()["provider"]
                    .as_str()
                    .unwrap()
                    .into()
            } else {
                details["searchProvider"].as_str().unwrap().into()
            },
            provider_mode: input["provider"].as_str().unwrap().into(),
            provider_order: serde_json::from_value(details["searchProviderOrder"].clone()).unwrap(),
            cache_hit: false,
        };
        let outcomes = input
            .get("outcomes")
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .unwrap_or_default();
        let projected = w::project(
            input["query"].as_str().unwrap(),
            2026,
            max,
            response,
            research::profile(
                input["usage"]["contextWindow"].as_f64().unwrap_or_default(),
                0.,
            ),
            &outcomes,
        )
        .unwrap();
        assert_eq!(
            projected.result, row["output"],
            "search projection {index}: {}",
            input["query"]
        );
        let hints:Vec<_>=projected.hints.into_iter().map(|(url,h)|{let mut hint=json!({"intent":h.intent,"richFetch":h.rich_fetch,"authoritative":h.authoritative});if h.intent=="docs_reference"{hint["focusTerms"]=json!(h.focus_terms);}
if let Some(kind)=h.coverage_kind{hint["coverageKind"]=json!(kind);}json!({"url":url,"hint":hint})}).collect();
        assert_eq!(json!(hints), row["hints"], "search hints {index}");
    }
}

#[test]
fn selected_original_websearch_cases_match_sdk_queries_ranking_results_and_hints() {
    use ag1_core::tools::{research, websearch as w};
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-websearch-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 32);
    for row in oracle["rows"].as_array().unwrap() {
        let mut previous_results = vec![];
        for call in row["calls"].as_array().unwrap() {
            let details = &call["output"]["details"];
            let max = call["args"]["maxResults"].as_u64().unwrap_or(3).min(4) as usize;
            let requests = call["requests"].as_array().unwrap();
            let results = if let Some(last) = requests.last() {
                if let Some(data) = last.get("json") {
                    w::parse_brave(data, max).unwrap().0
                } else {
                    w::parse_duckduckgo(last["text"].as_str().unwrap(), max).unwrap()
                }
            } else {
                assert_eq!(details["cacheHit"], true);
                previous_results.clone()
            };
            previous_results = results.clone();
            let provider = if details["searchProvider"] == "official-fallback" {
                details["searchAttempts"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()["provider"]
                    .as_str()
                    .unwrap()
            } else {
                details["searchProvider"].as_str().unwrap()
            };
            let projected = w::project(
                call["args"]["query"].as_str().unwrap(),
                2026,
                max,
                w::ProviderResponse {
                    results,
                    attempts: serde_json::from_value(details["searchAttempts"].clone()).unwrap(),
                    provider: provider.into(),
                    provider_mode: details["searchProviderMode"].as_str().unwrap().into(),
                    provider_order: serde_json::from_value(details["searchProviderOrder"].clone())
                        .unwrap(),
                    cache_hit: details["cacheHit"] == true,
                },
                research::profile(
                    call["usage"]["contextWindow"].as_f64().unwrap_or_default(),
                    call["usage"]["percent"].as_f64().unwrap_or_default(),
                ),
                &serde_json::from_value(call["outcomes"].clone()).unwrap(),
            )
            .unwrap();
            assert_eq!(projected.result, call["output"], "{}", row["name"]);
            if row["name"]
                .as_str()
                .unwrap()
                .contains("records pricing coverage hints")
            {
                assert!(!projected.hints.is_empty());
                assert!(
                    projected
                        .hints
                        .iter()
                        .any(|(_, hint)| json!(hint)["coverageKind"] == "pricing")
                );
            }
        }
    }
}

#[test]
fn selected_web_presentations_match_sdk_compact_and_expanded_text_without_client_framing() {
    use ag1_core::tools::web_presentation;
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-web-presentation-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 4);
    for row in oracle["rows"].as_array().unwrap() {
        assert_eq!(row["calls"].as_array().unwrap().len(), 2);
        for call in row["calls"].as_array().unwrap() {
            let actual = web_presentation::render(
                call["tool"].as_str().unwrap(),
                &call["args"],
                &call["result"],
                call["expanded"].as_bool().unwrap(),
            )
            .unwrap();
            assert_eq!(json!(actual), call["presentation"], "{}", row["name"]);
        }
    }
    let result = json!({"details":{"title":"Title\u{1b}]52;c;bad\u{7}","preview":"Safe\u{1b}[31mtext\u{1b}[0m\u{0}","status":"placeholder"}});
    let actual = web_presentation::render(
        "webFetch",
        &json!({"url":"https://fixture.invalid/"}),
        &result,
        true,
    )
    .unwrap();
    assert!(!actual.summary.contains('\u{1b}'));
    assert!(!actual.summary.contains("bad"));
    assert!(actual.body.contains("Safetext"));
    assert!(!actual.body.contains('\u{0}'));
    assert!(
        web_presentation::render(
            "webFetch",
            &json!({}),
            &json!({"details":{"preview":"x".repeat(131073)}}),
            true
        )
        .is_err()
    );
    assert!(web_presentation::render("unknown", &json!({}), &json!({}), false).is_err());
}

#[test]
fn active_registry_defaults_sdk_provenance_and_prompt_refresh_preserve_native_capability_gates() {
    use ag1_core::{
        context::prompt::PromptInput,
        tools::{registry::Registry, targets},
    };
    let mut registry = Registry::builtins().unwrap();
    targets::register_visual_tools(&mut registry).unwrap();
    registry.register_sdk(json!({"name":"sdk_tool","description":"Tool registered through createAgentSession","parameters":{"type":"object"},"sourceInfo":{"source":"builtin"}})).unwrap();
    registry.register_sdk(json!({"name":"hidden_tool","description":"Description should appear in available tools","parameters":{"type":"object"},"promptGuidelines":["  Use hidden_tool for its declared task.  ","Use hidden_tool for its declared task."]})).unwrap();
    assert!(registry.active_names().is_empty());
    let mut available: std::collections::BTreeSet<_> = registry
        .definitions()
        .map(|d| d.metadata["name"].as_str().unwrap().to_owned())
        .collect();
    available.remove("screenshot_url");
    available.remove("interact_then_screenshot");
    available.remove("verify_ui");
    available.remove("bash");
    registry.activate_defaults(&available).unwrap();
    let names = registry.active_names();
    for name in [
        "webSearch",
        "webFetch",
        "artifact",
        "episode",
        "sdk_tool",
        "hidden_tool",
    ] {
        assert!(names.contains(&name.into()));
    }
    for name in [
        "bash",
        "screenshot_url",
        "interact_then_screenshot",
        "verify_ui",
        "editApply",
    ] {
        assert!(!names.contains(&name.into()));
    }
    let public = registry.public_tools();
    let read = public.iter().find(|d| d["name"] == "read").unwrap();
    assert_eq!(
        read["sourceInfo"],
        json!({"path":"<builtin:read>","source":"builtin","scope":"temporary","origin":"top-level"})
    );
    let sdk = public.iter().find(|d| d["name"] == "sdk_tool").unwrap();
    assert_eq!(
        sdk["sourceInfo"],
        json!({"path":"<sdk:sdk_tool>","source":"sdk","scope":"temporary","origin":"top-level"})
    );
    let base = PromptInput {
        date: "2026-09-05".into(),
        cwd: "/fixture".into(),
        ..PromptInput::default()
    };
    let prompt = registry.build_prompt(&base).unwrap();
    for text in [
        "hidden_tool",
        "Description should appear in available tools",
        "- webSearch: Search the web for current information, documentation, or error messages",
        "- webFetch: Fetch and read a URL directly",
        "For online research, use webSearch for candidate sources and webFetch for the best URL",
        "Prefer authoritative sources such as official docs, vendor docs, specs, and release notes",
        "Keep source URLs available when online information affects the answer",
        "recover externalized raw evidence: unknown id → artifact",
        "bracket substantial long-task phases and preserve exact trajectories",
    ] {
        assert!(prompt.contains(text), "{text}");
    }
    assert_eq!(
        prompt
            .matches("Use hidden_tool for its declared task.")
            .count(),
        1
    );
    assert!(base.selected_tools.is_none());
    assert!(base.tool_snippets.is_empty());
    let target = targets::Binding {
        workspace: "fixture".into(),
        target: "fixture".into(),
        revision: "1".into(),
        capabilities: [
            targets::Capability::Screenshot,
            targets::Capability::Interact,
        ]
        .into(),
        vision: targets::VisionAccess {
            caller: true,
            judge: false,
        },
    };
    available.extend(target.available_tools());
    registry.activate_defaults(&available).unwrap();
    for name in ["screenshot_url", "interact_then_screenshot"] {
        assert!(registry.active_names().contains(&name.into()));
        registry
            .prepare(
                "visual",
                name,
                name,
                &if name == "screenshot_url" {
                    json!({"url":"index.html"})
                } else {
                    json!({"url":"index.html","actions":[]})
                },
                true,
            )
            .unwrap();
        let visible = registry
            .public_tools()
            .into_iter()
            .find(|d| d["name"] == name)
            .unwrap();
        assert_eq!(
            visible["sourceInfo"],
            json!({"path":format!("<visual:{name}>"),"source":"visual","scope":"temporary","origin":"top-level"})
        );
    }
    registry.activate(&["read".into()], &available).unwrap();
    let next = registry.build_prompt(&base).unwrap();
    assert!(!next.contains("hidden_tool"));
    assert!(!next.contains("Description should appear in available tools"));
}

#[test]
fn target_schemas_are_pinned_inactive_and_capabilities_gate_visual_visibility() {
    use ag1_core::tools::{
        registry::Registry,
        targets::{self, Binding, Capability, Request, VisionAccess},
    };
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/target-catalog-oracle.json"
    ))
    .unwrap();
    let mut registry = Registry::builtins().unwrap();
    targets::register_visual_tools(&mut registry).unwrap();
    assert_eq!(registry.definitions().count(), 38);
    let facades: Vec<_> = registry.definitions().filter_map(|d| {
        let name = d.metadata["name"].as_str().unwrap();
        ag1_core::tools::surface::is_facade(name).then_some(name)
    }).collect();
    assert_eq!(facades, ["capability", "evidence", "inspect", "releaseEvidence", "retainEvidence", "ui", "work"]);
    assert!(registry.active_names().is_empty());
    for row in oracle["catalog"].as_array().unwrap() {
        assert_eq!(
            &registry
                .definitions()
                .find(|d| d.metadata["name"] == row["name"])
                .unwrap()
                .metadata,
            row
        );
    }
    let revision = registry.revision();
    assert!(targets::register_visual_tools(&mut registry).is_err());
    assert_eq!(registry.revision(), revision);
    let mut binding = Binding {
        workspace: "workspace".into(),
        target: "target".into(),
        revision: "revision-1".into(),
        capabilities: [
            Capability::Check,
            Capability::Screenshot,
            Capability::Interact,
        ]
        .into(),
        vision: VisionAccess::default(),
    };
    assert_eq!(binding.available_tools(), ["check".into()].into());
    binding.vision.judge = true;
    assert_eq!(
        binding.available_tools(),
        ["check".into(), "verify_ui".into()].into()
    );
    binding.vision.caller = true;
    let available = binding.available_tools();
    registry
        .activate(&available.iter().cloned().collect::<Vec<_>>(), &available)
        .unwrap();
    let args = json!({"url":"index.html","actions":[{"type":"fill","selector":"#password","value":"private-fixture"}],"claim":"button is visible"});
    let call = registry
        .prepare("operation", "call", "verify_ui", &args, true)
        .unwrap();
    let request = Request::from_call(&call).unwrap();
    binding.admits(&request).unwrap();
    assert!(!request.display().to_string().contains("private-fixture"));
    assert_eq!(request.arguments(), &args);
    binding.capabilities.remove(&Capability::Interact);
    assert!(binding.admits(&request).is_err());
    for args in [
        json!({"url":""}),
        json!({"url":"index.html","viewport":{"width":0,"height":900}}),
        json!({"url":"index.html","viewport":{"width":2.5,"height":900}}),
        json!({"url":"index.html","timeoutMs":300001}),
    ] {
        let call = registry
            .prepare("invalid", "call", "screenshot_url", &args, true)
            .unwrap();
        assert!(Request::from_call(&call).is_err());
    }
}

#[test]
fn exact_selected_safety_cases_match_reference_decisions_and_approval_dialogs() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/safety-cases-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 50);
    let mut pure_cases = 0;
    let mut decisions = 0;
    let mut filesystem_cases = 0;
    for row in oracle["rows"].as_array().unwrap() {
        if !row["symlinks"].as_array().unwrap().is_empty() {
            assert_eq!(
                row["name"],
                "tool safety policy blocks file mutations that traverse a symlink outside the workspace"
            );
            filesystem_cases += 1;
            continue; // Exercised through FileService's P03 capability in platform/tool_contract.
        }
        pure_cases += 1;
        for call in row["calls"].as_array().unwrap() {
            let input = &call["input"];
            let p: policy::Policy = serde_json::from_value(input["policy"].clone()).unwrap();
            let decision = policy::evaluate(
                input["tool"].as_str().unwrap(),
                &input["args"],
                &p,
                input["untrusted"] == true,
            )
            .unwrap();
            assert_eq!(json!(decision), call["output"], "{}: {input}", row["name"]);
            assert_eq!(
                json!(decision.approval_request.as_ref().map(policy::dialog)),
                call["dialog"],
                "{}",
                row["name"]
            );
            decisions += 1;
        }
    }
    assert_eq!((pure_cases, decisions, filesystem_cases), (49, 96, 1));
}

#[test]
fn selected_result_externalization_preserves_small_blocks_thresholds_exact_diffs_and_unicode() {
    use ag1_core::tools::result::{self, ResultBudget};
    let ack = json!({"type":"text","text":"Successfully wrote 28,209 bytes to room.ts"});
    let summary = json!({"type":"text","text":"607 lines added"});
    let diff = format!("@@ room.ts @@\n{}", "+ scene source\n".repeat(2080));
    let image = json!({"type":"image","data":"abc","mimeType":"image/png"});
    let huge_diff = (1..=8000)
        .map(|n| format!("+{n} generated line"))
        .collect::<Vec<_>>()
        .join("\n");
    let raw_text = format!("BEGIN\n{}END", "middle\n".repeat(2000));
    let unicode = format!("HEAD\n{}\nTAIL", "🧪漢字".repeat(4000));
    let aggregate = ["A".repeat(10000), "B".repeat(10000), "C".repeat(10000)];
    let cases = [
        (
            "write",
            None,
            json!([ack,summary,{"type":"text","text":diff}]),
            diff.clone(),
            2usize,
        ),
        (
            "bash",
            Some(100),
            json!([{"type":"text","text":raw_text}]),
            raw_text.clone(),
            0,
        ),
        (
            "custom",
            Some(50),
            json!([{"type":"text","text":"keep"},{"type":"text","text":"x".repeat(100)},image]),
            "x".repeat(100),
            1,
        ),
        (
            "bash",
            None,
            json!([{"type":"text","text":"log line\n".repeat(4000)}]),
            "log line\n".repeat(4000),
            0,
        ),
        (
            "custom",
            None,
            json!([{"type":"text","text":aggregate[0]},{"type":"text","text":aggregate[1]},{"type":"text","text":aggregate[2]}]),
            aggregate.join("\n"),
            0,
        ),
        (
            "write",
            None,
            json!([{"type":"text","text":huge_diff}]),
            huge_diff.clone(),
            0,
        ),
        (
            "bash",
            Some(100),
            json!([{"type":"text","text":unicode}]),
            unicode.clone(),
            0,
        ),
    ];
    for (tool, threshold, content, exact, preview_index) in cases {
        let mut store = tool_fixture::FixtureStore::default();
        let raw = json!({"content":content,"details":{"firstChangedLine":1,"diffSummary":{"addedLines":8000},"exitCode":1}});
        let projected = result::project(
            &mut store,
            tool,
            "selected-call",
            &raw,
            ResultBudget {
                threshold_bytes: threshold,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(projected.text_artifacts.len(), 1);
        let evidence = &projected.text_artifacts[0];
        assert_eq!(store.texts[&evidence.id], exact);
        assert_eq!(
            serde_json::from_str::<Value>(&store.texts[&projected.original.id]).unwrap(),
            raw
        );
        let text = projected.result["content"][preview_index]["text"]
            .as_str()
            .unwrap();
        let envelope: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(
            envelope["externalizedToolResult"]["nextCall"]["action"],
            "read"
        );
        assert_eq!(
            envelope["externalizedToolResult"]["artifactId"],
            evidence.id
        );
        assert!(text.contains("full_result: exact artifact, paged by UTF-8 byte offset"));
        assert!(!text.contains('�'));
        assert_eq!(projected.result["details"]["firstChangedLine"], 1);
        assert_eq!(projected.result["details"]["exitCode"], 1);
        assert_eq!(
            projected.result["details"]["diffSummary"]["addedLines"],
            8000
        );
        if tool == "write" {
            assert_eq!(evidence.kind, "diff");
            assert!(text.contains("exact artifact bytes"));
            let resident = projected.result["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(resident.len() < 4000);
        }
        if preview_index == 2 {
            assert_eq!(projected.result["content"][0], ack);
            assert_eq!(projected.result["content"][1], summary);
        }
        if preview_index == 1 {
            assert_eq!(
                projected.result["content"][0],
                json!({"type":"text","text":"keep"})
            );
            assert_eq!(projected.result["content"][2], image);
        }
        if exact == raw_text {
            assert!(text.contains("BEGIN") && text.contains("END"));
        }
        if exact == unicode {
            assert!(text.contains("HEAD") && text.contains("TAIL"));
        }
    }
    for (tool, text) in [
        ("read", "small result".into()),
        ("read", "change\n".repeat(6000)),
        ("edit", "change\n".repeat(6000)),
        ("diff", "change\n".repeat(6000)),
    ] {
        let mut store = tool_fixture::FixtureStore::default();
        let raw = json!({"content":[{"type":"text","text":text}]});
        let result = result::project(
            &mut store,
            tool,
            "resident",
            &raw,
            ResultBudget::default(),
            false,
        )
        .unwrap();
        assert_eq!(result.result, raw);
        assert!(result.text_artifacts.is_empty());
        // P05 deliberately retains the original result for custody even when no
        // text is externalized; this differs from the standalone SDK helper.
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.records[0]["kind"], "tool-original-result");
    }
}

#[test]
fn many_large_result_blocks_share_one_budget_including_every_preview_frame() {
    use ag1_core::tools::result::{self, ResultBudget};
    for cap in [128, 400, 64000] {
        let raw = json!({"content":(0..12).map(|i|json!({"type":"text","text":format!("BLOCK-{i}\n{}\nEND-{i}","🧪漢字".repeat(4000))})).collect::<Vec<_>>()});
        let mut store = tool_fixture::FixtureStore::default();
        let output = result::project(
            &mut store,
            "custom",
            "multiple",
            &raw,
            ResultBudget {
                text_units: cap,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        let actual: usize = output.result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["text"].as_str())
            .map(|s| s.encode_utf16().count())
            .sum();
        assert_eq!(output.projected_text_units, actual);
        assert!(actual <= cap);
        assert_eq!(
            serde_json::from_str::<Value>(&store.texts[&output.original.id]).unwrap(),
            raw
        );
        if cap == 64000 {
            assert_eq!(output.text_artifacts.len(), 12);
            assert_eq!(output.result["content"].as_array().unwrap().len(), 12);
            for (i, artifact) in output.text_artifacts.iter().enumerate() {
                assert_eq!(
                    store.texts[&artifact.id],
                    raw["content"][i]["text"].as_str().unwrap()
                );
            }
        } else {
            assert_eq!(output.text_artifacts.len(), 1);
        }
    }
}

#[test]
fn path_expansion_preserves_sdk_home_prefix_and_unicode_space_semantics() {
    use ag1_core::tools::path;
    for (input, expected) in [
        ("~", "/fixture/home"),
        ("~/Documents/file.txt", "/fixture/home/Documents/file.txt"),
        ("file\u{a0}name.txt", "file name.txt"),
        ("@~/Documents/file.txt", "/fixture/home/Documents/file.txt"),
        ("/absolute/path/file.txt", "/absolute/path/file.txt"),
        ("relative/file.txt", "relative/file.txt"),
    ] {
        assert_eq!(
            path::expand(input, Some("/fixture/home")).unwrap(),
            expected
        );
    }
    assert!(path::expand("~", None).is_err());
    assert!(path::expand("@", Some("/fixture/home")).is_err());
    assert!(path::expand("nul\0path", None).is_err());
}
#[test]
fn shell_write_hints_ignore_quoted_programs_and_escaped_operators() {
    use ag1_core::tools::shell;

    // A real P14 model check passed JavaScript to node -e. Its arrow belongs to
    // the quoted program, and Bash does not create a file named "{".
    for command in [
        r#"node -e "(async () => { console.log('ok'); })()""#,
        r#"node -e '(() => { console.log("ok"); })()'"#,
        r#"printf '%s' 'value > imaginary.txt'"#,
        r#"printf '%s' \> imaginary.txt"#,
        r#"printf '%s' "escaped quote: \" > imaginary.txt""#,
        r#"printf '%s' ${unset_value:-a>phantom.txt}"#,
        r#"printf '%s' "$(printf '%s' "a > phantom.txt")""#,
        r#"printf ok > ${unset_value:-a>phantom.txt}"#,
        r#"printf ok > "$(printf '%s' "a > phantom.txt")""#,
        r#"printf '%s' $'escaped quote: \' > phantom.txt suffix'"#,
    ] {
        assert!(shell::write_targets(command).is_empty(), "{command}");
    }
    // Quoted program text must not hide the real outer-shell destination.
    assert_eq!(
        shell::write_targets(r#"node -e "(() => { console.log('ok'); })()" > actual.txt"#),
        ["actual.txt"]
    );
    assert_eq!(
        shell::write_targets(r#"printf '%s' 'value > imaginary.txt' >> 'actual.txt'"#),
        ["actual.txt"]
    );
}

#[test]
fn github_list_preview_preserves_metadata_and_recovers_exact_local_evidence() {
    use ag1_core::tools::result::{self,ResultBudget};
    let data=json!({"status":200,"outcome":"read","rate_resource":"search","rate_remaining":29,"rate_limit":30,"rate_reset":1234,"next":"search/repositories?q=example&page=2", "data":{"total_count":100,"items":(0..30).map(|n|json!({"id":n,"full_name":format!("owned/repo-{n}"),"description":"private fixture description🦙".repeat(1000)})).collect::<Vec<_>>()}});
    let text=data.to_string();let raw=json!({"content":[{"type":"text","text":text}],"details":{"github":data}});
    for cap in [1500,6000] {
        let mut store=tool_fixture::FixtureStore::default();
        let projected=result::project(&mut store,"github","owned-call",&raw,ResultBudget{text_units:cap,..Default::default()},false).unwrap();
        let preview:Value=serde_json::from_str(projected.result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(preview["rate_remaining"],29);assert_eq!(preview["rate_resource"],"search");assert_eq!(preview["next"],data["next"]);
        assert_eq!(preview["full_result"]["github_requests"],0);assert_eq!(preview["summary_only"],true);
        assert_eq!(preview["items"][0]["full_name"],"owned/repo-0");assert!(projected.projected_text_units<=cap);
        let id=preview["full_result"]["artifact_id"].as_str().unwrap();assert_eq!(store.texts[id],text);
        assert_eq!(serde_json::from_str::<Value>(&store.texts[&projected.original.id]).unwrap(),raw);
        assert!(preview["full_result"]["readCall"].is_object());
        assert!(projected.result["content"][0]["text"].as_str().unwrap().contains(ag1_core::context::compiler::EXACT));
    }
}

#[test]
fn deeper_edit_preview_and_apply_share_matching_and_reject_changed_source() {
    use ag1_core::tools::file::transform;
    let source = b"alpha\nbeta\n";
    let mut args = json!({"path":"fixture.txt","mode":"preview","edits":[{"oldText":"alpha","newText":"beta"},{"oldText":"beta","newText":"gamma"}]});
    let preview = transform("edit", &args, Some(source)).unwrap();
    assert!(preview.bytes.is_none());
    args["mode"] = json!("apply");
    args["expectedSha256"] = preview.result["details"]["sourceSha256"].clone();
    assert_eq!(transform("edit", &args, Some(source)).unwrap().bytes.unwrap(), b"beta\ngamma\n");
    assert!(transform("edit", &args, Some(b"alpha\nbeta\nnew work\n")).is_err());
    args["mode"] = json!("preview");
    let p = serde_json::from_value(json!({"mode":"planning","cwd":"/fixture"})).unwrap();
    assert!(policy::evaluate("edit", &args, &p, true).unwrap().allow);
    args["mode"] = json!("apply");
    assert!(!policy::evaluate("edit", &args, &p, true).unwrap().allow);
}
