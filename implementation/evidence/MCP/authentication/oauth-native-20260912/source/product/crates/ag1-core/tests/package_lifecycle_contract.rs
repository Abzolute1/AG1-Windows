use ag1_core::{
    DomainError,
    resources::{
        package_lifecycle::{self, Manager, Port, Run, Scope},
        packages::{self, Source},
    },
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
struct Fixture {
    input: Value,
    files: BTreeMap<String, String>,
    directories: BTreeSet<String>,
    calls: Vec<Value>,
    progress: Vec<Value>,
}
#[test]
fn configured_update_selection_matches_sdk_order_collisions_and_suggestions() {
    use package_lifecycle::ConfiguredPackage;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/package-selection-oracle.json"
    ))
    .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        let input = &row["input"];
        let manager = Manager::new(package_lifecycle::Config {
            cwd: "/fixture/project".into(),
            agent_dir: "/fixture/agent".into(),
            home: "/fixture/home".into(),
            temporary_dir: "/fixture/tmp".into(),
            npm: packages::npm_command(None).unwrap(),
            offline: input["offline"] == true,
        })
        .unwrap();
        let entries = [("user", Scope::User), ("project", Scope::Project)]
            .into_iter()
            .flat_map(|(key, scope)| {
                input[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(move |entry| ConfiguredPackage {
                        source: entry
                            .as_str()
                            .unwrap_or_else(|| entry["source"].as_str().unwrap())
                            .into(),
                        scope,
                    })
            })
            .collect::<Vec<_>>();
        let result = if input["action"] == "update" {
            manager
                .configured_updates(&entries, input["source"].as_str())
                .map(|v| json!(v))
        } else {
            manager.update_check_candidates(&entries).map(|v| {
                json!(v.into_iter().map(|entry| {
                let (kind, name) = match packages::parse(&entry.source).unwrap() {
                    Source::Npm { name, .. } => ("npm", name),
                    Source::Git { host, path, .. } => ("git", format!("{host}/{path}")),
                    _ => panic!("local package in query plan"),
                };
                json!({"source":entry.source,"scope":entry.scope,"type":kind,"displayName":name})
            }).collect::<Vec<_>>())
            })
        };
        let actual = match result {
            Ok(value) => json!({"value":value}),
            Err(DomainError::InvalidInput(message)) => json!({"error":message}),
            Err(error) => panic!("unexpected selection error {error:?}"),
        };
        assert_eq!(actual, row["output"], "{input}");
    }
}

#[test]
fn resolution_checks_installed_pins_without_querying_registry_or_mutating() {
    let mut manager = Manager::new(package_lifecycle::Config {
        cwd: "/fixture/project".into(),
        agent_dir: "/fixture/agent".into(),
        home: "/fixture/home".into(),
        temporary_dir: "/fixture/tmp".into(),
        npm: packages::npm_command(None).unwrap(),
        offline: false,
    })
    .unwrap();
    let mut fixture = Fixture::new(&json!({}));
    let installed = "/fixture/project/.ag1/npm/node_modules/fixture";
    fixture.create_directory(installed).unwrap();
    fixture.files.insert(
        format!("{installed}/package.json"),
        "{\"version\":\"1.0.0\"}".into(),
    );
    for (source, expected) in [
        ("npm:fixture", false),
        ("npm:fixture@1.0.0", false),
        ("npm:fixture@2.0.0", true),
        ("npm:fixture@^1", true),
        ("npm:missing", true),
        ("./missing", false),
    ] {
        assert_eq!(
            manager
                .needs_install(
                    &mut fixture,
                    &packages::parse(source).unwrap(),
                    Scope::Project
                )
                .unwrap(),
            expected,
            "{source}"
        );
    }
    assert!(fixture.calls.is_empty());
    assert!(fixture.progress.is_empty());
    assert_eq!(fixture.files.len(), 1);
    assert_eq!(
        manager.update_check_candidates(&vec![
            package_lifecycle::ConfiguredPackage {
                source: "npm:fixture".into(),
                scope: Scope::Project,
            };
            4097
        ]),
        Err(DomainError::LimitExceeded("configured packages".into()))
    );
}
impl Fixture {
    fn new(input: &Value) -> Self {
        let mut fixture = Self {
            input: input.clone(),
            files: BTreeMap::new(),
            directories: BTreeSet::new(),
            calls: vec![],
            progress: vec![],
        };
        for path in [
            "/fixture/project",
            "/fixture/agent",
            "/fixture/npm-global/node_modules",
        ] {
            fixture.create_directory(path).unwrap();
        }
        if input["localPresent"] == true {
            fixture.create_directory("/fixture/project/local").unwrap();
        }
        fixture
    }
}
impl Port for Fixture {
    fn checkpoint(&mut self) -> Result<(), DomainError> {
        Ok(())
    }
    fn exists(&mut self, path: &str) -> Result<bool, DomainError> {
        Ok(self.files.contains_key(path) || self.directories.contains(path))
    }
    fn read_json(&mut self, path: &str) -> Result<Option<Value>, DomainError> {
        self.files
            .get(path)
            .map(|s| {
                serde_json::from_str(s)
                    .map_err(|_| DomainError::InvalidInput("fixture JSON".into()))
            })
            .transpose()
    }
    fn create_directory(&mut self, path: &str) -> Result<(), DomainError> {
        let mut path = std::path::Path::new(path);
        loop {
            self.directories.insert(path.to_str().unwrap().into());
            let Some(parent) = path.parent() else { break };
            path = parent;
        }
        Ok(())
    }
    fn write_absent(&mut self, path: &str, bytes: &[u8]) -> Result<(), DomainError> {
        if self.exists(path)? {
            return Err(DomainError::Conflict);
        }
        self.files
            .insert(path.into(), String::from_utf8(bytes.to_vec()).unwrap());
        Ok(())
    }
    fn remove_owned_tree(&mut self, path: &str, _: Option<&str>) -> Result<(), DomainError> {
        self.files
            .retain(|p, _| p != path && !p.starts_with(&format!("{path}/")));
        self.directories
            .retain(|p| p != path && !p.starts_with(&format!("{path}/")));
        Ok(())
    }
    fn run(&mut self, run: Run) -> Result<String, DomainError> {
        self.calls.push(json!(run));
        if self.input["cancelAt"]
            .as_str()
            .is_some_and(|name| run.args.first().is_some_and(|a| a == name))
        {
            return Err(if self.input["timeout"] == true {
                DomainError::TimedOut
            } else {
                DomainError::Cancelled
            });
        }
        if run.kind == "sync" {
            return Ok("/fixture/npm-global/node_modules".into());
        }
        if run.kind == "run" {
            if self.input["failCommand"] == true {
                return Err(DomainError::Io("fixture command failed".into()));
            }
            if run.command == "git" && run.args.first().is_some_and(|s| s == "clone") {
                let target = run.args.last().unwrap();
                assert!(target.starts_with("/fixture/"));
                self.create_directory(target)?;
                self.files.insert(
                    format!("{target}/package.json"),
                    "{\"name\":\"fixture\",\"version\":\"1.0.0\"}".into(),
                );
            }
            return Ok(String::new());
        }
        if run.args == ["rev-parse", "--abbrev-ref", "@{upstream}"] {
            if self.input["noUpstream"] == true {
                return Err(DomainError::Io("no upstream".into()));
            }
            return Ok("origin/main".into());
        }
        if run.args.first().is_some_and(|s| s == "symbolic-ref") {
            return Ok("refs/remotes/origin/main".into());
        }
        if run.args == ["rev-parse", "HEAD"] {
            return Ok("a".repeat(40));
        }
        if run.args.first().is_some_and(|s| s == "rev-parse") {
            return Ok(if self.input["sameHead"] == true {
                "a"
            } else {
                "b"
            }
            .repeat(40));
        }
        panic!("unexpected fixture command");
    }
    fn progress(&mut self, event: Value) -> Result<(), DomainError> {
        self.progress.push(event);
        Ok(())
    }
}

#[test]
fn cancellation_during_git_fallback_does_not_continue_to_fetch_or_reset() {
    for command in ["remote", "symbolic-ref"] {
        for timeout in [false, true] {
            let mut manager = Manager::new(package_lifecycle::Config {
                cwd: "/fixture/project".into(),
                agent_dir: "/fixture/agent".into(),
                home: "/fixture/home".into(),
                temporary_dir: "/fixture/tmp".into(),
                npm: packages::npm_command(None).unwrap(),
                offline: false,
            })
            .unwrap();
            let source = "git:github.com/acme/repo";
            let mut fixture =
                Fixture::new(&json!({"noUpstream":true,"cancelAt":command,"timeout":timeout}));
            let path = manager
                .installed_path(
                    &mut fixture,
                    &packages::parse(source).unwrap(),
                    Scope::Project,
                )
                .unwrap();
            fixture.create_directory(&path).unwrap();
            assert_eq!(
                manager.update_source(&mut fixture, source, Scope::Project),
                Err(if timeout {
                    DomainError::TimedOut
                } else {
                    DomainError::Cancelled
                })
            );
            assert_eq!(fixture.calls.last().unwrap()["args"][0], command);
            assert!(
                !fixture
                    .calls
                    .iter()
                    .any(|v| v["args"][0] == "fetch" || v["args"][0] == "reset")
            );
            assert_eq!(fixture.progress.last().unwrap()["type"], "error");
        }
    }
}
#[test]
fn lifecycle_state_and_port_calls_match_all_captured_sdk_install_update_remove_cases() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/package-lifecycle-oracle.json"
    ))
    .unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let command: Option<Vec<String>> = input
            .get("command")
            .map(|v| serde_json::from_value(v.clone()).unwrap());
        let mut manager = Manager::new(package_lifecycle::Config {
            cwd: "/fixture/project".into(),
            agent_dir: "/fixture/agent".into(),
            home: "/fixture/home".into(),
            temporary_dir: "/fixture/tmp".into(),
            npm: packages::npm_command(command.as_deref()).unwrap(),
            offline: input["offline"] == true,
        })
        .unwrap();
        let source = input["source"].as_str().unwrap();
        let parsed = packages::parse(source).unwrap();
        let scope: Scope = serde_json::from_value(input["scope"].clone()).unwrap();
        let mut fixture = Fixture::new(input);
        if input["installed"] == true || input["action"] == "pinnedMatch" {
            let target = manager
                .installed_path(&mut fixture, &parsed, scope)
                .unwrap();
            fixture.create_directory(&target).unwrap();
            fixture.files.insert(format!("{target}/package.json"),json!({"name":"fixture","version":input.get("version").cloned().unwrap_or(json!("1.0.0"))}).to_string());
        }
        fixture.calls.clear();
        let result = match input["action"].as_str().unwrap() {
            "install" => manager
                .install(&mut fixture, source, scope)
                .map(|_| Value::Null),
            "remove" => manager
                .remove(&mut fixture, source, scope)
                .map(|_| Value::Null),
            "update" => manager
                .update_source(&mut fixture, source, scope)
                .map(|_| Value::Null),
            "temporaryRefresh" => manager
                .refresh_temporary(&mut fixture, source)
                .map(|_| Value::Null),
            "pinnedMatch" => {
                let path = manager
                    .installed_path(&mut fixture, &parsed, scope)
                    .unwrap();
                manager
                    .pinned_matches(&mut fixture, &parsed, &path)
                    .map(Value::Bool)
            }
            "rootCache" => {
                let a = manager.global_npm_root(&mut fixture).unwrap();
                let b = manager.global_npm_root(&mut fixture).unwrap();
                manager
                    .set_npm(
                        packages::npm_command(Some(&[
                            "/fixture/second-npm".into(),
                            "--fixture".into(),
                        ]))
                        .unwrap(),
                    )
                    .unwrap();
                let c = manager.global_npm_root(&mut fixture).unwrap();
                Ok(json!([a, b, c]))
            }
            other => panic!("unselected package fixture {other}"),
        };
        let output = match result {
            Ok(value) => json!({"value":value}),
            Err(DomainError::InvalidInput(error) | DomainError::Io(error)) => {
                json!({"error":error})
            }
            Err(error) => panic!("unexpected native error {error:?}"),
        };
        assert_eq!(output, row["output"], "package {i} outcome");
        assert_eq!(json!(fixture.calls), row["calls"], "package {i} commands");
        assert_eq!(
            json!(fixture.progress),
            row["progress"],
            "package {i} progress"
        );
        let files: BTreeMap<_, _> = fixture
            .files
            .into_iter()
            .map(|(p, bytes)| (p.strip_prefix("/fixture/").unwrap().to_string(), bytes))
            .collect();
        // JSON object member order is not semantically significant; native serde
        // preserves settings fields while the SDK insertion order differs.
        let reference = row["files"].as_object().unwrap();
        assert_eq!(files.len(), reference.len(), "package {i} file count");
        for (name, bytes) in files {
            let expected = reference[&name].as_str().unwrap();
            if name.ends_with("package.json") {
                assert_eq!(
                    serde_json::from_str::<Value>(&bytes).unwrap(),
                    serde_json::from_str::<Value>(expected).unwrap(),
                    "package {i} manifest"
                );
            } else {
                assert_eq!(bytes, expected, "package {i} file {name}");
            }
        }
        assert!(!matches!(parsed, Source::Local { .. }) || fixture.calls.is_empty());
    }
}

#[test]
fn unsafe_package_options_and_embedded_credentials_are_rejected_before_any_port_effect() {
    for source in [
        "npm:--help",
        "https://github.com/user/repo@--orphan",
        "npm:fixture@https://fixture-user:fixture-password@example.com/package.tgz",
        "npm:alias@npm:fixture@https://fixture-user:fixture-password@example.com/package.tgz",
        "https://fixture-user:fixture-password@github.com/user/repo",
    ] {
        let mut manager = Manager::new(package_lifecycle::Config {
            cwd: "/fixture/project".into(),
            agent_dir: "/fixture/agent".into(),
            home: "/fixture/home".into(),
            temporary_dir: "/fixture/tmp".into(),
            npm: packages::npm_command(None).unwrap(),
            offline: false,
        })
        .unwrap();
        let mut fixture = Fixture::new(&json!({}));
        let before = fixture.directories.clone();
        assert!(matches!(
            manager.install(&mut fixture, source, Scope::Project),
            Err(DomainError::InvalidInput(_))
        ));
        assert!(fixture.calls.is_empty());
        assert!(fixture.progress.is_empty());
        assert!(fixture.files.is_empty());
        assert_eq!(fixture.directories, before);
    }
}
