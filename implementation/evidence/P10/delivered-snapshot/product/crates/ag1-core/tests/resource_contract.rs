use ag1_core::{DomainError, resources::frontmatter};
use serde_json::{Value, json};
#[test]
fn frontmatter_preserves_reference_scalars_bodies_aliases_and_unknown_fields_with_bounded_failures()
{
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-functions-oracle.json"
    ))
    .unwrap();
    let mut selected = 0;
    for (index, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        if row["name"] != "parseFrontmatter" && row["name"] != "cyclicFrontmatter" {
            continue;
        }
        selected += 1;
        let result = frontmatter::parse(row["args"][0].as_str().unwrap());
        if row["name"] == "cyclicFrontmatter" {
            assert_eq!(row["result"]["value"], "TypeError");
            assert!(
                matches!(result, Err(DomainError::InvalidInput(_))),
                "cyclic aliases must fail before materialization"
            );
        } else if row["result"].get("error").is_some() {
            assert!(
                matches!(result,Err(DomainError::InvalidInput(ref reason)) if reason.contains("duplicate mapping key")),
                "reference duplicate-key error {index}"
            );
        } else {
            assert_eq!(
                json!(result.unwrap()),
                row["result"]["value"],
                "frontmatter {index}"
            );
        }
    }
    assert_eq!(selected, 18);
}
#[test]
fn yaml_input_depth_alias_expansion_and_duplicate_keys_are_bounded() {
    assert!(matches!(
        frontmatter::yaml(&"x".repeat(256 * 1024 + 1)),
        Err(DomainError::LimitExceeded(_))
    ));
    let nested = format!("{}value{}", "[".repeat(33), "]".repeat(33));
    assert!(matches!(
        frontmatter::yaml(&nested),
        Err(DomainError::LimitExceeded(_))
    ));
    let mut aliases = String::from("a0: &a0 [one, two, three, four]\n");
    for i in 1..14 {
        aliases.push_str(&format!(
            "a{i}: &a{i} [*a{}, *a{}, *a{}, *a{}]\n",
            i - 1,
            i - 1,
            i - 1,
            i - 1
        ));
    }
    assert!(matches!(
        frontmatter::yaml(&aliases),
        Err(DomainError::LimitExceeded(_))
    ));
    assert!(matches!(
        frontmatter::yaml("key: one\nkey: two"),
        Err(DomainError::InvalidInput(_))
    ));
    assert!(frontmatter::yaml("---\nfirst\n...\n---\nsecond").is_err());
    assert!(matches!(
        frontmatter::parse(&"x".repeat(1024 * 1024 + 1)),
        Err(DomainError::LimitExceeded(_))
    ));
    // The source's loose opening delimiter can split a UTF-16 surrogate; native
    // parsing still must not panic on a valid UTF-8 resource document.
    assert!(
        std::panic::catch_unwind(|| frontmatter::parse("---🦙\nname: fixture\n---\nbody")).is_ok()
    );
}

#[test]
fn template_argument_substitution_and_skill_activation_match_all_reference_functions() {
    use ag1_core::resources::{prompts, skills};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-functions-oracle.json"
    ))
    .unwrap();
    let mut selected = 0;
    for (index, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let args = &row["args"];
        let name = row["name"].as_str().unwrap();
        let value = match name {
            "parseCommandArgs" => json!(prompts::command_args(args[0].as_str().unwrap()).unwrap()),
            "substituteArgs" => json!(
                prompts::substitute(
                    args[0].as_str().unwrap(),
                    &serde_json::from_value::<Vec<String>>(args[1].clone()).unwrap()
                )
                .unwrap()
            ),
            "expandPromptTemplate" => json!(
                prompts::expand(
                    args[0].as_str().unwrap(),
                    &serde_json::from_value::<Vec<prompts::Template>>(args[1].clone()).unwrap()
                )
                .unwrap()
            ),
            "validateSkillName" => json!(skills::validate_name(args[0].as_str().unwrap())),
            "matchSkillForPrompt" => {
                let skills = serde_json::from_value::<Vec<skills::Skill>>(args[1].clone()).unwrap();
                json!(skills::match_prompt(args[0].as_str().unwrap(), &skills).unwrap())
            }
            "formatSkillsForPrompt" => json!(
                skills::format(
                    &serde_json::from_value::<Vec<skills::Skill>>(args[0].clone()).unwrap()
                )
                .unwrap()
            ),
            "parseFrontmatter" | "cyclicFrontmatter" => continue,
            _ => panic!("unselected resource function {name}"),
        };
        selected += 1;
        assert_eq!(
            value, row["result"]["value"],
            "resource function {index}: {name}"
        );
    }
    assert_eq!(selected, 61);
}
#[test]
fn template_growth_and_argument_cardinality_fail_with_explicit_limits() {
    use ag1_core::resources::prompts;
    assert!(matches!(
        prompts::command_args(&"word ".repeat(257)),
        Err(DomainError::LimitExceeded(_))
    ));
    assert!(matches!(
        prompts::substitute("$@", &vec![String::new(); 257]),
        Err(DomainError::LimitExceeded(_))
    ));
    let repeated = "$ARGUMENTS".repeat(3000);
    let args = vec!["$`".into()];
    assert!(matches!(
        prompts::substitute(&repeated, &args),
        Err(DomainError::LimitExceeded(_))
    ));
}

#[test]
fn theme_schema_variable_resolution_and_optional_fallbacks_match_reference() {
    use ag1_core::resources::themes;
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/theme-oracle.json")).unwrap();
    assert_eq!(json!(themes::REQUIRED), oracle["required"]);
    for row in oracle["rows"].as_array().unwrap() {
        let result = themes::parse(&row["input"].to_string());
        assert_eq!(
            result.is_ok(),
            row["output"]["accepted"].as_bool().unwrap(),
            "{}",
            row["name"]
        );
        if let Ok(theme) = result {
            assert_eq!(theme.name, row["output"]["name"]);
            fn ansi(value: &Value, bg: bool) -> String {
                let n = if bg { 48 } else { 38 };
                if let Some(numeric) = value.as_f64() {
                    return format!("\x1b[{n};5;{numeric}m");
                }
                let value = value.as_str().unwrap();
                if value.is_empty() {
                    return format!("\x1b[{}m", if bg { 49 } else { 39 });
                }
                let rgb: Vec<_> = (1..7)
                    .step_by(2)
                    .map(|i| {
                        let hex = &value[i..i + 2];
                        u8::from_str_radix(hex, 16)
                            .unwrap_or_else(|_| u8::from_str_radix(&hex[..1], 16).unwrap())
                    })
                    .collect();
                format!("\x1b[{n};2;{};{};{}m", rgb[0], rgb[1], rgb[2])
            }
            assert_eq!(
                ansi(&theme.colors["accent"], false),
                row["output"]["accent"]
            );
            assert_eq!(
                ansi(&theme.colors["toolDiffLineNum"], false),
                row["output"]["line"]
            );
            assert_eq!(
                ansi(&theme.colors["toolDiffAddedBg"], true),
                row["output"]["added"]
            );
        }
    }
    let mut value = oracle["rows"][0]["input"].clone();
    value["vars"] = json!({});
    for i in 0..130 {
        value["vars"][format!("var{i}")] = json!(format!("var{}", i + 1));
    }
    value["colors"]["accent"] = json!("var0");
    assert!(matches!(
        themes::parse(&value.to_string()),
        Err(DomainError::LimitExceeded(_))
    ));
}

#[test]
fn package_source_identity_and_npm_command_arguments_match_reference() {
    use ag1_core::resources::packages;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/package-source-oracle.json"
    ))
    .unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let source = row["source"].as_str().unwrap();
        let parsed = packages::parse(source);
        if let Some(error) = row.get("error") {
            assert_eq!(
                parsed,
                Err(DomainError::InvalidInput(error.as_str().unwrap().into())),
                "package source {i}"
            );
            continue;
        }
        assert_eq!(
            json!(parsed.unwrap()),
            row["parsed"],
            "package source {i}: {source}"
        );
        assert_eq!(
            json!(packages::git(source).unwrap()),
            row["git"],
            "git source {i}"
        );
    }
    for row in oracle["commands"].as_array().unwrap() {
        let values: Option<Vec<String>> =
            serde_json::from_value(row["configured"].clone()).unwrap();
        let result = packages::npm_command(values.as_deref());
        if let Some(error) = row.get("error") {
            assert_eq!(
                result,
                Err(DomainError::InvalidInput(error.as_str().unwrap().into()))
            );
        } else {
            assert_eq!(json!(result.unwrap()), row["result"]);
        }
    }
}

#[test]
fn selected_original_package_sources_match_sdk_parsing_and_identity_assertions() {
    use ag1_core::resources::packages;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-package-sources-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 19);
    for row in oracle["rows"].as_array().unwrap() {
        for call in row["calls"].as_array().unwrap() {
            let source = call["source"].as_str().unwrap();
            let parsed = packages::parse(source).unwrap();
            let actual = match call["method"].as_str().unwrap() {
                "parse" => json!(parsed),
                "identity" => {
                    assert!(!matches!(parsed, packages::Source::Local { .. }));
                    json!(parsed.identity(""))
                }
                other => panic!("unexpected captured method {other}"),
            };
            assert_eq!(actual, call["output"], "{}: {source}", row["name"]);
        }
    }
}

#[test]
fn catalog_bounds_retained_alias_expansions_and_rejects_typed_metadata_instead_of_reinterpreting_it()
 {
    use ag1_core::resources::{
        catalog::{self, CapturedResource, Kind, Metadata, ResolvedResource, Trust},
        prompts, skills,
    };
    use sha2::{Digest, Sha256};
    let mut captures = vec![];
    for i in 0..300 {
        let name = format!("fixture-{i}");
        let content = format!(
            "---\nname: {name}\ndescription: Fixture.\ntriggers: [&phrase {}, {}]\n---\nBody",
            "x".repeat(512),
            vec!["*phrase"; 255].join(", ")
        );
        let path = format!("/fixture/skills/{name}/SKILL.md");
        let sha256 = format!("{:x}", Sha256::digest(content.as_bytes()));
        captures.push(CapturedResource {
            resource: ResolvedResource {
                path: path.clone(),
                enabled: true,
                metadata: Metadata {
                    source: "auto".into(),
                    scope: "project".into(),
                    origin: "top-level".into(),
                    base_dir: None,
                },
            },
            kind: Kind::Skill,
            canonical_path: path,
            bytes: content.into_bytes(),
            sha256,
            trust: Trust::UntrustedData,
        });
    }
    assert!(captures.iter().map(|c| c.bytes.len()).sum::<usize>() < 1024 * 1024);
    assert!(
        matches!(catalog::compile(&captures),Err(DomainError::LimitExceeded(ref reason)) if reason=="retained resource catalog")
    );
    for field in ["name: [wrong]", "description: true"] {
        let content = format!("---\n{field}\n---\nBody");
        assert!(
            skills::load("/fixture/SKILL.md", &content, None)
                .skill
                .is_none()
        );
    }
    assert!(matches!(
        prompts::template(
            "/fixture/prompt.md",
            "---\ndescription: [wrong]\n---\nBody",
            None
        ),
        Err(DomainError::InvalidInput(_))
    ));
}

#[test]
fn extension_trust_matches_reference_precedence_and_closes_canonical_alias_bypass() {
    use ag1_core::resources::trust;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/extension-trust-oracle.json"
    ))
    .unwrap();
    let project = oracle["projectConfigDir"].as_str().unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let config = &row["config"];
        let paths = row["paths"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["canonical"].as_array().unwrap())
            .map(|(p, c)| trust::ExtensionPath {
                path: p.as_str().unwrap().into(),
                canonical: c.as_str().unwrap().into(),
            })
            .collect::<Vec<_>>();
        let user = if config["user"] == true {
            vec![project.into()]
        } else {
            vec![]
        };
        let selected = trust::select(
            &paths,
            trust::Policy {
                project_config_dir: project,
                canonical_project_dir: project,
                explicit: config["explicit"].as_bool(),
                environment: config["env"].as_str(),
                user_trusted_projects: &user,
            },
        )
        .unwrap();
        if config["alias"] == true {
            assert_eq!(
                row["output"]["loaded"].as_array().unwrap().len(),
                1,
                "reference alias bypass must remain captured"
            );
            assert!(selected.allowed.is_empty());
            assert_eq!(
                selected.pending.unwrap().extension_paths,
                vec![paths[0].path.clone()]
            );
        } else {
            let mut output = json!(selected);
            let map = output.as_object_mut().unwrap();
            let allowed = map.remove("allowed").unwrap();
            map.insert("loaded".into(), allowed);
            assert_eq!(output, row["output"], "extension trust case {i}");
        }
    }
}

#[test]
fn selected_skill_prompt_formatting_and_activation_match_sdk_cases() {
    use ag1_core::resources::skills::{self, Skill};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-skills-oracle.json"
    ))
    .unwrap();
    let mut checked = 0;
    for row in oracle["rows"].as_array().unwrap() {
        let output = match row["kind"].as_str().unwrap() {
            "format" => json!(
                skills::format(
                    &serde_json::from_value::<Vec<Skill>>(row["skills"].clone()).unwrap()
                )
                .unwrap()
            ),
            "match" => json!(
                skills::match_prompt(
                    row["prompt"].as_str().unwrap(),
                    &serde_json::from_value::<Vec<Skill>>(row["skills"].clone()).unwrap()
                )
                .unwrap()
            ),
            _ => continue,
        };
        checked += 1;
        assert_eq!(output, row["output"], "{row}");
    }
    assert_eq!(checked, 10);
}
