#![cfg(target_os = "linux")]
use ag1_core::{
    DomainError,
    resources::{
        catalog::{self, CapturedResource, Kind, Metadata, ResolvedResource, Trust},
        prompts,
    },
};
use ag1_platform::{
    resource_fs::{Discovery, ResourceFs, ScanControl},
    workspace::WorkspaceService,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::symlink,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn control(cancel: &AtomicBool) -> ScanControl<'_> {
    ScanControl {
        cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
#[test]
fn long_literal_prompts_load_without_hiding_file_overrides_or_capability_errors() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("prompt.md"), "Prompt from file").unwrap();
    fs::write(outside.path().join("secret.md"), "Outside capability").unwrap();
    symlink(
        outside.path().join("secret.md"),
        dir.path().join("escape.md"),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let load = |input: &str| {
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        ResourceLoader::new(
            ResourceFs::new(vec![workspace]).unwrap(),
            Config {
                cwd: dir.path().into(),
                agent_dir: dir.path().into(),
                home: dir.path().into(),
                additional: Paths::default(),
                disabled: vec![],
                installed: vec![],
                system: Some(input.into()),
                append_system: Some(input.into()),
            },
        )
        .unwrap()
        .build(&control(&cancel))
    };
    for input in ["You are AG1 in a controlled long-horizon coding benchmark. Historical tool bodies may be immutable artifacts. Retrieve exact authoritative evidence before material edits, inspect current source, make the smallest correct change, and run the provided verifier. Never modify verifier files.".to_owned(), "é".repeat(200), "x".repeat(9000)] {
        let snapshot = load(&input).unwrap();
        assert_eq!(snapshot.system.as_deref(), Some(input.as_str()));
        assert_eq!(snapshot.append_system, vec![input]);
    }
    let snapshot = load("prompt.md").unwrap();
    assert_eq!(snapshot.system.as_deref(), Some("Prompt from file"));
    assert_eq!(snapshot.append_system, vec!["Prompt from file"]);
    assert!(matches!(
        load("escape.md"),
        Err(DomainError::PermissionDenied)
    ));
}
#[test]
fn selected_original_package_resource_cases_match_sdk_paths_filters_and_precedence() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    for (fixture, count) in [
        (
            include_str!("../../../../fixtures/tools/selected-package-resources-oracle.json"),
            33,
        ),
        (
            include_str!("../../../../fixtures/tools/selected-package-explicit-oracle.json"),
            12,
        ),
    ] {
        let oracle: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(oracle["rows"].as_array().unwrap().len(), count);
        for row in oracle["rows"].as_array().unwrap() {
            for call in row["calls"].as_array().unwrap() {
                let dir = tempfile::tempdir().unwrap();
                let input = &call["input"];
                let rebase = |value: &str| {
                    assert!(value == "/fixture" || value.starts_with("/fixture/"));
                    dir.path().join(
                        value
                            .strip_prefix("/fixture")
                            .unwrap()
                            .trim_start_matches('/'),
                    )
                };
                for (name, state) in input["files"].as_object().unwrap() {
                    let file = dir.path().join(name);
                    assert!(file.starts_with(dir.path()));
                    if state["directory"] == true {
                        fs::create_dir_all(file).unwrap();
                    } else {
                        fs::create_dir_all(file.parent().unwrap()).unwrap();
                        fs::write(
                            file,
                            state["text"]
                                .as_str()
                                .unwrap()
                                .replace("/fixture", dir.path().to_str().unwrap()),
                        )
                        .unwrap();
                    }
                }
                let cwd = rebase(input["cwd"].as_str().unwrap());
                let agent = rebase(input["agentDir"].as_str().unwrap());
                for (base, key) in [(&agent, "user"), (&cwd.join(".ag1"), "project")] {
                    fs::create_dir_all(base).unwrap();
                    fs::write(
                        base.join("settings.json"),
                        input[key]
                            .to_string()
                            .replace("/fixture", dir.path().to_str().unwrap()),
                    )
                    .unwrap();
                }
                let workspace = WorkspaceService::new()
                    .unwrap()
                    .register(dir.path())
                    .unwrap();
                let loader = ResourceLoader::new(
                    ResourceFs::new(vec![workspace]).unwrap(),
                    Config {
                        cwd,
                        agent_dir: agent,
                        home: rebase(input["home"].as_str().unwrap()),
                        additional: Paths::default(),
                        disabled: vec![],
                        installed: vec![],
                        system: None,
                        append_system: None,
                    },
                )
                .unwrap();
                let cancelled = AtomicBool::new(false);
                let paths = if let Some(sources) = input.get("sources") {
                    use ag1_core::resources::package_lifecycle::Scope;
                    let sources: Vec<Value> = serde_json::from_str(
                        &sources
                            .to_string()
                            .replace("/fixture", dir.path().to_str().unwrap()),
                    )
                    .unwrap();
                    let scope = if input["options"]["temporary"] == true {
                        Scope::Temporary
                    } else if input["options"]["local"] == true {
                        Scope::Project
                    } else {
                        Scope::User
                    };
                    let (paths, diagnostics) = loader
                        .resolve_sources(&sources, scope, &control(&cancelled))
                        .unwrap();
                    assert!(diagnostics.is_empty());
                    cancelled.store(true, std::sync::atomic::Ordering::Release);
                    assert!(matches!(
                        loader.resolve_sources(&sources, scope, &control(&cancelled)),
                        Err(DomainError::Cancelled)
                    ));
                    cancelled.store(false, std::sync::atomic::Ordering::Release);
                    assert!(matches!(
                        loader.resolve_sources(
                            &vec![json!("missing"); 257],
                            scope,
                            &control(&cancelled)
                        ),
                        Err(DomainError::LimitExceeded(_))
                    ));
                    paths
                } else {
                    loader.build(&control(&cancelled)).unwrap().paths
                };
                assert_eq!(
                    normalize(json!(paths), dir.path()),
                    call["output"],
                    "{}",
                    row["name"]
                );
                assert_eq!(loader.generation(), 0);
                assert!(loader.snapshot().is_none());
            }
        }
    }
}

#[test]
fn package_home_discovery_and_missing_sources_keep_host_scope_without_implicit_installs() {
    use ag1_core::resources::package_lifecycle::Scope;
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("scratch/nested");
    let agent = dir.path().join(".ag1/agent");
    let skill = dir.path().join(".agents/skills/home-skill/SKILL.md");
    for path in [&cwd, &agent, skill.parent().unwrap()] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(&skill, "---\nname: home-skill\ndescription: home\n---\n").unwrap();
    fs::create_dir_all(cwd.join(".ag1")).unwrap();
    fs::write(
        cwd.join(".ag1/settings.json"),
        r#"{"packages":["npm:missing-package","git:github.com/example/missing-repo"]}"#,
    )
    .unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let loader = ResourceLoader::new(
        ResourceFs::new(vec![workspace]).unwrap(),
        Config {
            cwd: cwd.clone(),
            agent_dir: agent,
            home: dir.path().into(),
            additional: Paths::default(),
            disabled: vec![],
            installed: vec![],
            system: None,
            append_system: None,
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let snapshot = loader.build(&control(&cancel)).unwrap();
    let matching: Vec<_> = snapshot
        .paths
        .skills
        .iter()
        .filter(|r| Path::new(&r.path) == skill)
        .collect();
    assert_eq!(matching.len(), 1);
    assert!(matching[0].enabled);
    assert_eq!(matching[0].metadata.scope, "user");
    assert_eq!(matching[0].metadata.source, "auto");
    for paths in [
        &snapshot.paths.extensions,
        &snapshot.paths.skills,
        &snapshot.paths.prompts,
        &snapshot.paths.themes,
    ] {
        assert!(paths.iter().all(|r| r.metadata.origin != "package"));
    }
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("not installed in a registered root"))
            .count(),
        2
    );
    let (paths, notes) = loader
        .resolve_sources(
            &[
                json!("npm:missing-package"),
                json!("git:github.com/example/missing-repo"),
            ],
            Scope::Temporary,
            &control(&cancel),
        )
        .unwrap();
    assert!(
        paths.extensions.is_empty()
            && paths.skills.is_empty()
            && paths.prompts.is_empty()
            && paths.themes.is_empty()
    );
    assert_eq!(notes.len(), 2);
    assert!(!cwd.join(".ag1/npm").exists());
    assert!(!cwd.join(".ag1/git").exists());
    assert_eq!(loader.generation(), 0);
}

#[test]
fn installed_unpinned_and_temporary_packages_resolve_without_registry_or_git_refresh() {
    use ag1_core::resources::package_lifecycle::Scope;
    use ag1_platform::resources::{Config, InstalledPackage, Paths, ResourceLoader};
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project");
    let agent = dir.path().join("agent");
    let npm = dir.path().join("installed/npm");
    let git = dir.path().join("installed/git");
    for path in [
        cwd.join(".ag1"),
        agent.clone(),
        npm.join("extensions"),
        git.join("extensions"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(
        cwd.join(".ag1/settings.json"),
        r#"{"packages":["npm:example"]}"#,
    )
    .unwrap();
    fs::write(
        npm.join("package.json"),
        r#"{"name":"example","version":"1.0.0"}"#,
    )
    .unwrap();
    let extension = "export default function() {};";
    fs::write(npm.join("extensions/index.ts"), extension).unwrap();
    fs::write(git.join("extensions/index.ts"), extension).unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let loader = ResourceLoader::new(
        ResourceFs::new(vec![workspace]).unwrap(),
        Config {
            cwd: cwd.clone(),
            agent_dir: agent,
            home: dir.path().join("home"),
            additional: Paths::default(),
            disabled: vec![],
            installed: vec![
                InstalledPackage {
                    source: "npm:example".into(),
                    scope: "project".into(),
                    identity: "npm:example".into(),
                    root: npm.clone(),
                },
                InstalledPackage {
                    source: "git:github.com/example/repo".into(),
                    scope: "temporary".into(),
                    identity: "git:github.com/example/repo".into(),
                    root: git.clone(),
                },
            ],
            system: None,
            append_system: None,
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let snapshot = loader.build(&control(&cancel)).unwrap();
    assert_eq!(snapshot.paths.extensions.len(), 1);
    assert_eq!(
        Path::new(&snapshot.paths.extensions[0].path),
        npm.join("extensions/index.ts")
    );
    assert!(snapshot.paths.extensions[0].enabled);
    assert_eq!(snapshot.paths.extensions[0].metadata.scope, "project");
    let (paths, diagnostics) = loader
        .resolve_sources(
            &[json!("git:github.com/example/repo")],
            Scope::Temporary,
            &control(&cancel),
        )
        .unwrap();
    assert!(diagnostics.is_empty());
    assert_eq!(paths.extensions.len(), 1);
    assert_eq!(
        Path::new(&paths.extensions[0].path),
        git.join("extensions/index.ts")
    );
    assert!(paths.extensions[0].enabled);
    assert_eq!(paths.extensions[0].metadata.scope, "temporary");
    assert_eq!(
        fs::read_to_string(npm.join("extensions/index.ts")).unwrap(),
        extension
    );
    assert_eq!(
        fs::read_to_string(git.join("extensions/index.ts")).unwrap(),
        extension
    );
    assert_eq!(
        fs::read_to_string(npm.join("package.json")).unwrap(),
        r#"{"name":"example","version":"1.0.0"}"#
    );
    assert!(!cwd.join(".ag1/npm").exists());
    assert!(!cwd.join(".ag1/git").exists());
    assert_eq!(loader.generation(), 0);
}

#[test]
fn installed_package_resources_are_withheld_until_configured_pin_matches() {
    use ag1_platform::resources::{Config, InstalledPackage, Paths, ResourceLoader};
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let package = dir.path().join("installed");
    for path in [
        project.join(".ag1"),
        package.join("prompts"),
        dir.path().join("agent"),
        dir.path().join("home"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(
        project.join(".ag1/settings.json"),
        r#"{"packages":["npm:fixture@2.0.0"]}"#,
    )
    .unwrap();
    fs::write(package.join("prompts/fixture.md"), "Package prompt.").unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let mut loader = ResourceLoader::new(
        ResourceFs::new(vec![workspace]).unwrap(),
        Config {
            cwd: project,
            agent_dir: dir.path().join("agent"),
            home: dir.path().join("home"),
            additional: Paths::default(),
            disabled: vec![],
            installed: vec![InstalledPackage {
                source: "npm:fixture@2.0.0".into(),
                scope: "project".into(),
                identity: "npm:fixture".into(),
                root: package.clone(),
            }],
            system: None,
            append_system: None,
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    for (version, expected) in [
        (Some("1.0.0"), false),
        (None, false),
        (Some("2.0.0"), true),
        (Some("3.0.0"), false),
    ] {
        fs::write(
            package.join("package.json"),
            json!({"name":"fixture","version":version,"ag1":{"prompts":["prompts"]}}).to_string(),
        )
        .unwrap();
        loader
            .reload(loader.generation(), &control(&cancel))
            .unwrap();
        let snapshot = loader.snapshot().unwrap();
        assert_eq!(
            snapshot.catalog.prompts.len(),
            usize::from(expected),
            "{version:?}"
        );
        assert_eq!(
            snapshot
                .diagnostics
                .iter()
                .any(|d| d.message.contains("does not match the configured version")),
            !expected
        );
    }
}
fn fixture() -> (tempfile::TempDir, ResourceFs, Value) {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-discovery-oracle.json"
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in oracle["files"].as_object().unwrap() {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            bytes
                .as_str()
                .unwrap()
                .replace("/fixture", dir.path().to_str().unwrap()),
        )
        .unwrap();
    }
    fs::create_dir_all(dir.path().join("project/.git")).unwrap();
    for link in oracle["symlinks"].as_array().unwrap() {
        symlink(
            dir.path().join(link["target"].as_str().unwrap()),
            dir.path().join(link["path"].as_str().unwrap()),
        )
        .unwrap();
    }
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let resources = ResourceFs::new(vec![workspace]).unwrap();
    (dir, resources, oracle)
}
fn normalize(value: Value, root: &Path) -> Value {
    serde_json::from_str(
        &serde_json::to_string(&value)
            .unwrap()
            .replace(root.to_str().unwrap(), "/fixture"),
    )
    .unwrap()
}
fn direct(
    resources: &ResourceFs,
    path: &Path,
    mode: Discovery,
    scope: &str,
    kind: Kind,
    cancel: &AtomicBool,
) -> Vec<CapturedResource> {
    resources
        .scan(path, mode, control(cancel))
        .unwrap()
        .paths
        .into_iter()
        .map(|path| {
            let metadata = Metadata {
                source: "local".into(),
                scope: scope.into(),
                origin: "top-level".into(),
                base_dir: Some(Path::new(&path).parent().unwrap().to_str().unwrap().into()),
            };
            resources
                .capture(
                    ResolvedResource {
                        path,
                        enabled: true,
                        metadata,
                    },
                    kind,
                    Trust::UntrustedData,
                    &control(cancel),
                )
                .unwrap()
        })
        .collect()
}
#[test]
fn filesystem_skills_and_direct_templates_match_captured_reference() {
    let (dir, resources, oracle) = fixture();
    let cancel = AtomicBool::new(false);
    for row in [0, 1] {
        let paths = if row == 1 {
            vec![
                dir.path()
                    .join("project/.ag1/skills")
                    .to_str()
                    .unwrap()
                    .into(),
                dir.path().join("agent/skills").to_str().unwrap().into(),
                dir.path().join("missing.md").to_str().unwrap().into(),
            ]
        } else {
            vec![]
        };
        let catalog = ag1_platform::resources::load_skills(
            &resources,
            ag1_platform::resources::StandaloneSkills {
                cwd: &dir.path().join("project"),
                agent_dir: &dir.path().join("agent"),
                home: &dir.path().join("home"),
                paths: &paths,
                include_defaults: row == 0,
            },
            &control(&cancel),
        )
        .unwrap();
        assert_eq!(
            normalize(
                json!({"skills":catalog.skills,"diagnostics":catalog.diagnostics}),
                dir.path()
            ),
            oracle["rows"][row]["output"],
            "standalone skills {row}"
        );
    }
    let mut templates = vec![];
    for (path, scope) in [
        ("agent/prompts", "user"),
        ("project/.ag1/prompts", "project"),
    ] {
        for input in direct(
            &resources,
            &dir.path().join(path),
            Discovery::DirectPrompts,
            scope,
            Kind::Prompt,
            &cancel,
        ) {
            templates.push(
                prompts::template(
                    &input.resource.path,
                    std::str::from_utf8(&input.bytes).unwrap(),
                    Some(input.resource.metadata.source_info(&input.resource.path)),
                )
                .unwrap(),
            );
        }
    }
    assert_eq!(
        normalize(json!(templates), dir.path()),
        oracle["rows"][2]["output"]
    );
    let auto = resources
        .scan(
            &dir.path().join("project/.ag1/prompts"),
            Discovery::Prompts,
            control(&cancel),
        )
        .unwrap();
    assert_eq!(
        auto.paths
            .iter()
            .map(|p| Path::new(p).file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        ["plain.md", "same.md"]
    );
    let agents = resources
        .scan(
            &dir.path().join("project/.agents/skills"),
            Discovery::Skills {
                root_markdown: false,
            },
            control(&cancel),
        )
        .unwrap();
    assert_eq!(agents.paths.len(), 1);
    assert!(agents.paths[0].ends_with("agent-only/SKILL.md"));
}
#[test]
fn discovery_aliases_remain_inside_registered_roots_and_cycles_cancel_and_limits_fail_closed() {
    let (dir, resources, _) = fixture();
    let cancel = AtomicBool::new(false);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "outside").unwrap();
    let root = dir.path().join("bounded");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("valid.md"), "---\ndescription: Valid.\n---\nBody").unwrap();
    symlink(outside.path().join("secret.md"), root.join("escape.md")).unwrap();
    symlink(&root, root.join("cycle")).unwrap();
    let scan = resources
        .scan(
            &root,
            Discovery::Skills {
                root_markdown: true,
            },
            control(&cancel),
        )
        .unwrap();
    assert_eq!(scan.paths, [root.join("valid.md").to_str().unwrap()]);
    assert_eq!(scan.diagnostics.len(), 2);
    assert!(matches!(
        resources.read(&root.join("escape.md"), 1024, &control(&cancel)),
        Err(DomainError::PermissionDenied)
    ));
    fs::write(root.join("large.md"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert!(matches!(
        resources.read(&root.join("large.md"), 1024 * 1024, &control(&cancel)),
        Err(DomainError::LimitExceeded(_))
    ));
    cancel.store(true, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        resources.scan(
            &dir.path().join("absent"),
            Discovery::Prompts,
            control(&cancel)
        ),
        Err(DomainError::Cancelled)
    ));
    cancel.store(false, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        resources.scan(
            &root,
            Discovery::Prompts,
            ScanControl {
                cancel: &cancel,
                deadline: Instant::now()
            }
        ),
        Err(DomainError::TimedOut)
    ));
    let deep = dir.path().join("deep");
    let mut leaf = deep.clone();
    for _ in 0..34 {
        leaf = leaf.join("a");
    }
    fs::create_dir_all(&leaf).unwrap();
    assert!(matches!(
        resources.scan(&deep, Discovery::Recursive(Kind::Prompt), control(&cancel)),
        Err(DomainError::LimitExceeded(_))
    ));
}
#[test]
fn extension_entries_are_data_and_smart_discovery_never_executes_them() {
    let (dir, resources, _) = fixture();
    let cancel = AtomicBool::new(false);
    let root = dir.path().join("extensions");
    for (file, body) in [
        ("loose.ts", "throw new Error('must not execute');"),
        ("nested/index.js", "throw new Error('must not execute');"),
        (
            "skipped/deep/entry.js",
            "throw new Error('must not execute');",
        ),
    ] {
        let file = root.join(file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, body).unwrap();
    }
    let scan = resources
        .scan(&root, Discovery::Extensions, control(&cancel))
        .unwrap();
    assert_eq!(scan.paths.len(), 2);
    let captured: Vec<_> = scan
        .paths
        .into_iter()
        .map(|path| {
            resources
                .capture(
                    ResolvedResource {
                        path,
                        enabled: true,
                        metadata: Metadata {
                            source: "auto".into(),
                            scope: "project".into(),
                            origin: "top-level".into(),
                            base_dir: None,
                        },
                    },
                    Kind::Extension,
                    Trust::UntrustedData,
                    &control(&cancel),
                )
                .unwrap()
        })
        .collect();
    let catalog = catalog::compile(&captured).unwrap();
    assert!(
        catalog
            .extensions
            .iter()
            .all(|e| e.trust == Trust::UntrustedData)
    );
    fs::write(
        root.join("package.json"),
        r#"{"ag1":{"extensions":["nested/index.js"]}}"#,
    )
    .unwrap();
    let scan = resources
        .scan(&root, Discovery::Extensions, control(&cancel))
        .unwrap();
    assert_eq!(scan.paths, [root.join("nested/index.js").to_str().unwrap()]);
}

#[test]
fn package_patterns_match_reference_includes_excludes_exact_overrides_braces_and_extglobs() {
    use ag1_platform::resource_patterns::Filters;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-patterns-oracle.json"
    ))
    .unwrap();
    let paths: Vec<String> = serde_json::from_value(oracle["paths"].clone()).unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let patterns: Vec<String> = serde_json::from_value(row["patterns"].clone()).unwrap();
        let selected = Filters::new(&patterns, false)
            .unwrap()
            .select(&paths, oracle["base"].as_str().unwrap())
            .unwrap();
        assert_eq!(json!(selected), row["enabled"], "pattern {i}: {patterns:?}");
        let selected = Filters::new(&patterns, true)
            .unwrap()
            .select(&paths, oracle["base"].as_str().unwrap())
            .unwrap();
        // Override selection itself retains source paths' order; force inclusion
        // ordering is only observable in applyPatterns' Set result above.
        let selected: Vec<_> = paths
            .iter()
            .filter(|p| selected.contains(p))
            .cloned()
            .collect();
        assert_eq!(json!(selected), row["overrides"], "overrides {i}");
    }
    assert!(matches!(
        Filters::new(&["{0..1000000}".into()], false),
        Err(DomainError::LimitExceeded(_))
    ));
    assert!(matches!(
        Filters::new(&vec!["*".into(); 257], false),
        Err(DomainError::LimitExceeded(_))
    ));
}

#[test]
fn full_package_loader_precedence_context_and_reload_match_reference_with_atomic_generation() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let (dir, resources, oracle) = fixture();
    let cancel = AtomicBool::new(false);
    let additional = Paths {
        prompts: vec![ResolvedResource {
            path: dir
                .path()
                .join("explicit/custom.md")
                .to_str()
                .unwrap()
                .into(),
            enabled: true,
            metadata: Metadata {
                source: "local".into(),
                scope: "temporary".into(),
                origin: "top-level".into(),
                base_dir: Some(dir.path().join("explicit").to_str().unwrap().into()),
            },
        }],
        ..Paths::default()
    };
    let mut loader = ResourceLoader::new(
        resources,
        Config {
            cwd: dir.path().join("project"),
            agent_dir: dir.path().join("agent"),
            home: dir.path().join("home"),
            additional,
            disabled: vec![Kind::Extension],
            installed: vec![],
            system: None,
            append_system: None,
        },
    )
    .unwrap();
    assert_eq!(loader.generation(), 0);
    assert!(loader.snapshot().is_none());
    for (generation, row) in [(1, 4), (2, 5)] {
        if generation == 2 {
            fs::write(
                dir.path().join("project/.ag1/skills/same/SKILL.md"),
                "---\nname: same\ndescription: Project copy reloaded.\n---\nChanged body.",
            )
            .unwrap();
        }
        assert_eq!(
            loader.reload(generation - 1, &control(&cancel)),
            Ok(generation)
        );
        let snapshot = loader.snapshot().unwrap();
        assert_eq!(
            normalize(json!(snapshot.paths), dir.path()),
            oracle["rows"][3]["output"]
        );
        let reference = &oracle["rows"][row]["output"];
        assert_eq!(
            normalize(json!(snapshot.catalog.skills), dir.path()),
            reference["skills"]["skills"]
        );
        assert_eq!(
            normalize(json!(snapshot.catalog.prompts), dir.path()),
            reference["prompts"]["prompts"]
        );
        let mut diagnostics = reference["skills"]["diagnostics"]
            .as_array()
            .unwrap()
            .clone();
        diagnostics.extend(
            reference["prompts"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        assert_eq!(
            normalize(json!(snapshot.catalog.diagnostics), dir.path()),
            json!(diagnostics)
        );
        assert_eq!(
            normalize(json!(snapshot.agents_files), dir.path()),
            reference["agents"]["agentsFiles"]
        );
        assert_eq!(json!(snapshot.system), reference["system"]);
        assert_eq!(json!(snapshot.append_system), reference["append"]);
        assert!(snapshot.diagnostics.is_empty());
    }
    let fingerprint = loader.snapshot().unwrap().fingerprint.clone();
    assert_eq!(
        loader.reload(1, &control(&cancel)),
        Err(DomainError::Conflict)
    );
    fs::write(
        dir.path().join("project/.ag1/settings.json"),
        "invalid JSON",
    )
    .unwrap();
    assert!(matches!(
        loader.reload(2, &control(&cancel)),
        Err(DomainError::InvalidInput(_))
    ));
    assert_eq!(loader.generation(), 2);
    assert_eq!(loader.snapshot().unwrap().fingerprint, fingerprint);
    cancel.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        loader.reload(2, &control(&cancel)),
        Err(DomainError::Cancelled)
    );
    assert_eq!(loader.snapshot().unwrap().fingerprint, fingerprint);
}

#[test]
fn skill_helper_is_approval_bound_atomic_and_matches_reference_with_quoted_yaml_correction() {
    use ag1_core::tools::{
        approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
        policy::Policy,
        registry::{Registry, Scope},
    };
    use ag1_platform::{file_tools::FileExecution, skill_creator::SkillCreator};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-discovery-oracle.json"
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let creator = SkillCreator::new(workspace.clone(), "project/.ag1/skills").unwrap();
    let mut registry = Registry::builtins().unwrap();
    SkillCreator::register(&mut registry).unwrap();
    let names = vec!["createSkill".to_string()];
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let scope = Scope {
        workspace: workspace.id().as_str().into(),
        session: "helper-fixture".into(),
        epoch: 1,
        revision: 1,
    };
    let cancel = AtomicBool::new(false);
    let policy: Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":dir.path().to_str().unwrap()}))
            .unwrap();
    let mut auth = Authorizer::new("helper-fixture").unwrap();
    for (index, row) in oracle["rows"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .skip(6)
    {
        let call = registry
            .prepare(
                &format!("helper-{index}"),
                &format!("helper-{index}"),
                "createSkill",
                &row["input"],
                true,
            )
            .unwrap();
        let prepared = creator.prepare(&call, &scope);
        if row["output"]["success"] == false {
            let error = match prepared {
                Ok(_) => panic!("invalid helper draft accepted"),
                Err(e) => e,
            };
            let DomainError::InvalidInput(error) = error else {
                panic!("unexpected helper failure")
            };
            assert_eq!(
                error.replace(dir.path().to_str().unwrap(), "/fixture"),
                row["output"]["error"]
            );
            continue;
        }
        let prepared = prepared.unwrap();
        let target = prepared.target_revision().to_string();
        let name = row["input"]["name"].as_str().unwrap();
        let destination = dir.path().join(format!("project/.ag1/skills/{name}"));
        assert!(!destination.exists());
        let request = match auth
            .preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 10,
                    ttl: 10000,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        {
            Preflight::Approval(r) => r,
            _ => panic!("helper requires explicit policy admission"),
        };
        assert!(!destination.exists());
        let ticket = auth
            .answer(
                &request.binding,
                true,
                &registry,
                ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 11,
                },
            )
            .unwrap()
            .unwrap();
        let created = creator
            .execute(
                prepared,
                ticket,
                FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(10),
                },
            )
            .unwrap();
        let bytes = fs::read_to_string(destination.join("SKILL.md")).unwrap();
        if name == "quoted-skill" {
            assert_ne!(bytes, row["bytes"]);
            let parsed = ag1_core::resources::frontmatter::parse(&bytes).unwrap();
            assert_eq!(parsed.frontmatter["name"], name);
            assert_eq!(
                parsed.frontmatter["description"],
                row["input"]["description"]
            );
            assert_eq!(parsed.frontmatter.as_object().unwrap().len(), 2);
        } else {
            assert_eq!(bytes, row["bytes"]);
        }
        assert_eq!(
            created.outcome.effect,
            ag1_core::tools::executor::Effect::Committed
        );
        assert_eq!(
            created.outcome.result["details"]["sourceSha256"],
            created.revision.sha256
        );
        assert!(
            auth.answer(
                &request.binding,
                true,
                &registry,
                ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 13
                }
            )
            .is_err()
        );
    }
}
#[test]
fn resource_directory_publication_rejects_competing_creators_and_cleans_interrupted_staging() {
    use std::sync::{Arc, Barrier, atomic::AtomicUsize};
    let dir = tempfile::tempdir().unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let mut threads = vec![];
    for body in [b"one".to_vec(), b"two".to_vec()] {
        let workspace = workspace.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            workspace.create_directory_document_checked("skills/same", "SKILL.md", &body, || Ok(()))
        }));
    }
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .any(|r| matches!(r, Err(DomainError::Conflict)))
    );
    let published = fs::read(dir.path().join("skills/same/SKILL.md")).unwrap();
    assert!(published == b"one" || published == b"two");
    for stop in 0..7 {
        let attempts = AtomicUsize::new(0);
        let name = format!("skills/interrupted-{stop}");
        let result =
            workspace.create_directory_document_checked(&name, "SKILL.md", b"complete", || {
                let n = attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if n == stop {
                    Err(DomainError::Cancelled)
                } else {
                    Ok(())
                }
            });
        if result.is_ok() {
            assert_eq!(
                fs::read(dir.path().join(&name).join("SKILL.md")).unwrap(),
                b"complete"
            );
        } else {
            assert_eq!(result, Err(DomainError::Cancelled));
            assert!(!dir.path().join(&name).exists());
        }
        assert!(fs::read_dir(dir.path().join("skills")).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".ag1-resource-")
        }));
    }
    fs::create_dir(dir.path().join("skills/existing-empty")).unwrap();
    assert_eq!(
        workspace.create_directory_document_checked(
            "skills/existing-empty",
            "SKILL.md",
            b"body",
            || Ok(())
        ),
        Err(DomainError::Conflict)
    );
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), dir.path().join("alias")).unwrap();
    assert_eq!(
        workspace.create_directory_document_checked("alias/skill", "SKILL.md", b"body", || Ok(())),
        Err(DomainError::PermissionDenied)
    );
    assert!(!outside.path().join("skill").exists());
}

#[test]
fn package_identities_use_native_source_parsing_and_settings_relative_paths() {
    use ag1_core::resources::packages;
    use ag1_platform::resources::resolve_path;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/package-source-oracle.json"
    ))
    .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        if row.get("error").is_some() {
            continue;
        }
        let source = packages::parse(row["source"].as_str().unwrap()).unwrap();
        let local = match &source {
            packages::Source::Local { path } => resolve_path(
                Path::new("/fixture/project/.ag1"),
                path,
                Path::new("/fixture/home"),
            )
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
            _ => String::new(),
        };
        assert_eq!(source.identity(&local), row["identity"]);
    }
    assert_eq!(
        resolve_path(
            Path::new("/fixture/project"),
            "  ~/skills ",
            Path::new("/fixture/home")
        )
        .unwrap(),
        Path::new("/fixture/home/skills")
    );
    assert_eq!(
        resolve_path(
            Path::new("/fixture/project"),
            "~other",
            Path::new("/fixture/home")
        )
        .unwrap(),
        Path::new("/fixture/home/other")
    );
}

#[test]
fn package_settings_edits_match_reference_identity_normalization_and_preserve_unknown_fields() {
    use ag1_platform::package_settings::{Change, edit_document};
    let trailing_slash = edit_document(
        &json!({"packages":["../project/remove-local-pkg"]}),
        Change::Remove,
        "/fixture/project/remove-local-pkg/",
        Path::new("/fixture/project"),
        Path::new("/fixture/agent"),
        Path::new("/fixture/home"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(trailing_slash["packages"], json!([]));
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/package-settings-oracle.json"
    ))
    .unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let project = row["input"]["local"] == true;
        let key = if project { "project" } else { "user" };
        let base = if project {
            "/fixture/project/.ag1"
        } else {
            "/fixture/agent"
        };
        let change = if row["input"]["action"] == "add" {
            Change::Add
        } else {
            Change::Remove
        };
        let document = edit_document(
            &row["before"][key],
            change,
            row["input"]["source"].as_str().unwrap(),
            Path::new("/fixture/project"),
            Path::new(base),
            Path::new("/fixture/home"),
        )
        .unwrap();
        assert_eq!(
            document.is_some(),
            row["output"]["changed"].as_bool().unwrap(),
            "settings case {i}"
        );
        assert_eq!(
            document.as_ref().unwrap_or(&row["before"][key]),
            &row["output"][key]
        );
    }
    let document = json!({"packages":[{"source":"npm:fixture@1","skills":["one"],"unknown":{"keep":true}},"npm:other"],"unknown":[1,2,3]});
    let result = edit_document(
        &document,
        Change::Remove,
        "npm:other",
        Path::new("/fixture"),
        Path::new("/fixture"),
        Path::new("/fixture"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(result["packages"][0], document["packages"][0]);
    assert_eq!(result["unknown"], document["unknown"]);
}

#[test]
fn package_settings_publication_conflicts_with_external_changes_while_approval_waits() {
    use ag1_core::tools::{
        approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
        policy::Policy,
        registry::{Registry, Scope},
    };
    use ag1_platform::{
        file_tools::FileExecution,
        package_settings::{Change, PackageSettings},
    };
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".ag1")).unwrap();
    let path = dir.path().join(".ag1/settings.json");
    let original = json!({"theme":"dark","unknown":{"keep":true},"packages":[]});
    fs::write(&path, original.to_string()).unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let settings = PackageSettings::new(
        workspace.clone(),
        ".ag1/settings.json",
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut registry = Registry::builtins().unwrap();
    let names = vec!["write".to_string()];
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let scope = Scope {
        workspace: workspace.id().as_str().into(),
        session: "settings-fixture".into(),
        epoch: 1,
        revision: 1,
    };
    let cancel = AtomicBool::new(false);
    let policy: Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":dir.path().to_str().unwrap()}))
            .unwrap();
    let mut authorizer = Authorizer::new("settings-fixture").unwrap();
    for (index, external) in [true, false].into_iter().enumerate() {
        let prepared = settings
            .prepare(
                &registry,
                &scope,
                &format!("settings-{index}"),
                Change::Add,
                "./package",
            )
            .unwrap()
            .unwrap();
        let target = prepared.target_revision().to_owned();
        let ticket = match authorizer
            .preflight(
                prepared.call().clone(),
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 10,
                    ttl: 10000,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        {
            Preflight::Ready(ticket) => ticket,
            Preflight::Approval(request) => authorizer
                .answer(
                    &request.binding,
                    true,
                    &registry,
                    ExecutionFence {
                        scope: &scope,
                        policy_revision: 1,
                        target_revision: &target,
                        now: 11,
                    },
                )
                .unwrap()
                .unwrap(),
            Preflight::Rejected(reason) => panic!("{reason}"),
        };
        if external {
            fs::write(
                &path,
                json!({"theme":"light","unknown":{"new":true},"packages":[]}).to_string(),
            )
            .unwrap();
        }
        let result = settings.execute(
            prepared,
            ticket,
            FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(10),
            },
        );
        if external {
            assert!(matches!(result, Err(DomainError::Conflict)));
        } else {
            assert!(result.unwrap().published.is_some());
        }
        let current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current["theme"], "light");
        assert_eq!(current["unknown"], json!({"new":true}));
        assert_eq!(
            current["packages"],
            if external {
                json!([])
            } else {
                json!(["../package"])
            }
        );
    }
    assert!(
        settings
            .prepare(
                &registry,
                &scope,
                "duplicate",
                Change::Add,
                &dir.path().join("package").to_string_lossy()
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn owned_package_directories_are_created_and_removed_without_following_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    workspace
        .create_directories_checked("owned/package/nested", || Ok(()))
        .unwrap();
    fs::write(dir.path().join("owned/package/nested/file"), "owned").unwrap();
    fs::write(outside.path().join("keep"), "outside").unwrap();
    symlink(outside.path(), dir.path().join("owned/package/alias")).unwrap();
    let pinned = workspace.pin_directory("owned/package").unwrap();
    workspace
        .remove_directory_tree_checked(pinned, || Ok(()))
        .unwrap();
    assert!(!dir.path().join("owned/package").exists());
    assert_eq!(
        fs::read_to_string(outside.path().join("keep")).unwrap(),
        "outside"
    );
    assert_eq!(fs::read_dir(dir.path().join("owned")).unwrap().count(), 0);
    symlink(outside.path(), dir.path().join("alias")).unwrap();
    assert_eq!(
        workspace.create_directories_checked("alias/blocked", || Ok(())),
        Err(DomainError::PermissionDenied)
    );
    assert!(!outside.path().join("blocked").exists());
    assert_eq!(
        workspace.remove_directory_tree_checked(workspace.pin_directory("").unwrap(), || Ok(())),
        Err(DomainError::PermissionDenied)
    );
    assert_eq!(
        workspace.create_directories_checked("cancelled", || Err(DomainError::Cancelled)),
        Err(DomainError::Cancelled)
    );
    assert!(!dir.path().join("cancelled").exists());
}
#[test]
fn package_removal_conflicts_on_replacement_and_distinguishes_cancel_before_and_after_detachment() {
    use std::sync::atomic::AtomicUsize;
    let dir = tempfile::tempdir().unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    workspace
        .create_directories_checked("owned/package", || Ok(()))
        .unwrap();
    fs::write(dir.path().join("owned/package/file"), "first").unwrap();
    let stale = workspace.pin_directory("owned/package").unwrap();
    fs::rename(
        dir.path().join("owned/package"),
        dir.path().join("owned/old"),
    )
    .unwrap();
    fs::create_dir(dir.path().join("owned/package")).unwrap();
    fs::write(dir.path().join("owned/package/file"), "replacement").unwrap();
    assert_eq!(
        workspace.remove_directory_tree_checked(stale, || Ok(())),
        Err(DomainError::Conflict)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("owned/package/file")).unwrap(),
        "replacement"
    );
    let pinned = workspace.pin_directory("owned/package").unwrap();
    assert_eq!(
        workspace.remove_directory_tree_checked(pinned, || Err(DomainError::Cancelled)),
        Err(DomainError::Cancelled)
    );
    assert!(dir.path().join("owned/package").exists());
    let pinned = workspace.pin_directory("owned/package").unwrap();
    let checks = AtomicUsize::new(0);
    let result = workspace.remove_directory_tree_checked(pinned, || {
        if checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2 {
            Err(DomainError::Cancelled)
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err(DomainError::DurabilityUnknown));
    assert!(!dir.path().join("owned/package").exists());
    let staged = fs::read_dir(dir.path().join("owned"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".ag1-package-remove-")
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(staged.join("file")).unwrap(),
        "replacement"
    );
}

#[test]
fn dynamic_resource_additions_match_reference_refresh_and_preserve_captured_provenance() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-extend-oracle.json"
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (path, bytes) in oracle["files"].as_object().unwrap() {
        fs::create_dir_all(dir.path().join(path).parent().unwrap()).unwrap();
        fs::write(dir.path().join(path), bytes.as_str().unwrap()).unwrap();
    }
    fs::create_dir_all(dir.path().join("project/.git")).unwrap();
    let resources = ResourceFs::new(vec![
        WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap(),
    ])
    .unwrap();
    let mut loader = ResourceLoader::new(
        resources,
        Config {
            cwd: dir.path().join("project"),
            agent_dir: dir.path().join("agent"),
            home: dir.path().join("home"),
            additional: Paths::default(),
            disabled: vec![Kind::Extension],
            installed: vec![],
            system: None,
            append_system: None,
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        if let Some(writes) = row["writes"].as_object() {
            for (path, bytes) in writes {
                fs::write(dir.path().join(path), bytes.as_str().unwrap()).unwrap();
            }
        }
        let generation = loader.generation();
        if i == 0 || row["reload"] == true {
            loader.reload(generation, &control(&cancel)).unwrap();
        } else {
            let mut paths = Paths::default();
            for (key, target) in [
                ("skillPaths", &mut paths.skills),
                ("promptPaths", &mut paths.prompts),
                ("themePaths", &mut paths.themes),
            ] {
                if let Some(entries) = row["paths"][key].as_array() {
                    for entry in entries {
                        let metadata: Metadata = serde_json::from_str(
                            &entry["metadata"]
                                .to_string()
                                .replace("/fixture", dir.path().to_str().unwrap()),
                        )
                        .unwrap();
                        target.push(ResolvedResource {
                            path: entry["path"].as_str().unwrap().into(),
                            enabled: true,
                            metadata,
                        });
                    }
                }
            }
            loader
                .extend_resources(generation, paths, &control(&cancel))
                .unwrap();
            if row["name"] == "empty" {
                assert_eq!(loader.generation(), generation);
            }
        }
        let snapshot = loader.snapshot().unwrap();
        let observed = normalize(
            json!({"skills":{"skills":snapshot.catalog.skills,"diagnostics":[]},"prompts":{"prompts":snapshot.catalog.prompts,"diagnostics":[]},"themes":snapshot.catalog.themes.iter().map(|t|json!({"name":t.name,"sourcePath":t.source_path,"sourceInfo":t.source_info})).collect::<Vec<_>>(),"themeDiagnostics":[],"agents":{"agentsFiles":snapshot.agents_files},"system":snapshot.system,"append":snapshot.append_system}),
            dir.path(),
        );
        let mut expected = row["output"].clone();
        // Explicit provenance correction: the SDK drops prior auto/package
        // metadata when extending a kind; native retained capture keeps it.
        for kind in ["skills", "prompts"] {
            for resource in expected[kind][kind].as_array_mut().unwrap() {
                if resource["name"] == "base" && resource["sourceInfo"]["source"] == "local" {
                    resource["sourceInfo"]["source"] = json!("auto");
                    resource["sourceInfo"]["baseDir"] = json!("/fixture/project/.ag1");
                }
            }
        }
        assert_eq!(observed, expected, "{}", row["name"]);
        assert!(snapshot.catalog.diagnostics.is_empty());
    }
    let generation = loader.generation();
    let fingerprint = loader.snapshot().unwrap().fingerprint.clone();
    assert!(matches!(
        loader.extend_resources(generation - 1, Paths::default(), &control(&cancel)),
        Err(DomainError::Conflict)
    ));
    let contribution = |path: &str| Paths {
        prompts: vec![ResolvedResource {
            path: path.into(),
            enabled: true,
            metadata: Metadata {
                source: "extension:fixture".into(),
                scope: "temporary".into(),
                origin: "package".into(),
                base_dir: None,
            },
        }],
        ..Paths::default()
    };
    cancel.store(true, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        loader.extend_resources(
            generation,
            contribution("../contrib/prompts"),
            &control(&cancel)
        ),
        Err(DomainError::Cancelled)
    ));
    cancel.store(false, std::sync::atomic::Ordering::Release);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("outside.md"), "unadmitted").unwrap();
    assert!(matches!(
        loader.extend_resources(
            generation,
            contribution(outside.path().join("outside.md").to_str().unwrap()),
            &control(&cancel)
        ),
        Err(DomainError::PermissionDenied)
    ));
    let mut extension = contribution("../contrib/prompts");
    extension.extensions = std::mem::take(&mut extension.prompts);
    assert!(matches!(
        loader.extend_resources(generation, extension, &control(&cancel)),
        Err(DomainError::PermissionDenied)
    ));
    assert_eq!(loader.generation(), generation);
    assert_eq!(loader.snapshot().unwrap().fingerprint, fingerprint);
    // A later update of another kind must retain a previous missing-skill note.
    let mut missing = contribution("../absent.md");
    missing.skills = std::mem::take(&mut missing.prompts);
    loader
        .extend_resources(generation, missing, &control(&cancel))
        .unwrap();
    let notes = serde_json::to_value(&loader.snapshot().unwrap().catalog.diagnostics).unwrap();
    assert_eq!(notes.as_array().unwrap().len(), 1);
    loader
        .extend_resources(
            loader.generation(),
            contribution("../contrib/prompts"),
            &control(&cancel),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&loader.snapshot().unwrap().catalog.diagnostics).unwrap(),
        notes
    );
}

use ag1_platform::resource_overrides::{Overrides, RegisteredOverride, View};
use std::sync::{Arc, Mutex};
struct FixtureOverrides {
    variant: String,
    skill: ag1_core::resources::skills::Skill,
    calls: Arc<Mutex<Vec<String>>>,
}
impl Overrides for FixtureOverrides {
    fn apply(
        &self,
        view: &mut View,
        changed: &[Kind],
        full: bool,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        control.check()?;
        let mut calls = self.calls.lock().unwrap();
        if self.variant != "system" && changed.contains(&Kind::Skill) {
            calls.push("skills".into());
            view.catalog.skills = vec![self.skill.clone()];
        }
        if self.variant == "combined" && changed.contains(&Kind::Prompt) {
            calls.push("prompts".into());
            for p in &mut view.catalog.prompts {
                p.content.push_str(" overridden");
            }
        }
        if full && self.variant == "combined" {
            calls.push("agents".into());
            for f in &mut view.agents_files {
                f.content.push_str(" overridden");
            }
        }
        if full && self.variant != "skills" {
            calls.push("system".into());
            view.system = Some(if self.variant == "system" {
                "Custom system prompt".into()
            } else {
                format!("{} overridden", view.system.as_deref().unwrap_or(""))
            });
        }
        if full && self.variant == "combined" {
            calls.push("append".into());
            view.append_system.push("Injected appendix.".into());
        }
        Ok(())
    }
}
#[test]
fn native_resource_overrides_match_sdk_and_keep_original_inputs_across_partial_refresh() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-overrides-oracle.json"
    ))
    .unwrap();
    for variant in ["skills", "system", "combined"] {
        let dir = tempfile::tempdir().unwrap();
        for (p, b) in oracle["files"].as_object().unwrap() {
            let p = dir.path().join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b.as_str().unwrap()).unwrap();
        }
        fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        let ws = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        let resources = ResourceFs::new(vec![ws]).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let registration = RegisteredOverride::new(
            "owned-fixture",
            Arc::new(FixtureOverrides {
                variant: variant.into(),
                skill: serde_json::from_value(oracle["skill"].clone()).unwrap(),
                calls: calls.clone(),
            }),
        )
        .unwrap();
        let mut loader = ResourceLoader::new(
            resources,
            Config {
                cwd: dir.path().join("project"),
                agent_dir: dir.path().join("agent"),
                home: dir.path().into(),
                additional: Paths::default(),
                disabled: vec![Kind::Extension],
                installed: vec![],
                system: None,
                append_system: None,
            },
        )
        .unwrap()
        .with_overrides(registration)
        .unwrap();
        let cancel = AtomicBool::new(false);
        loader.reload(0, &control(&cancel)).unwrap();
        for stage in ["reload", "extend-prompts"] {
            if stage == "extend-prompts" {
                calls.lock().unwrap().clear();
                let mut paths = Paths::default();
                paths.prompts.push(ResolvedResource {
                    path: "../contrib/extra.md".into(),
                    enabled: true,
                    metadata: Metadata {
                        source: "extension:fixture".into(),
                        scope: "temporary".into(),
                        origin: "package".into(),
                        base_dir: Some(dir.path().join("contrib").to_str().unwrap().into()),
                    },
                });
                loader
                    .extend_resources(1, paths, &control(&cancel))
                    .unwrap();
            }
            let snapshot = loader.snapshot().unwrap();
            let actual = normalize(
                json!({"skills":snapshot.catalog.skills,"prompts":snapshot.catalog.prompts,"agents":snapshot.agents_files,"system":snapshot.system,"append":snapshot.append_system}),
                dir.path(),
            );
            let row = oracle["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["variant"] == variant && r["stage"] == stage)
                .unwrap();
            let mut expected = row["output"].clone();
            // Existing P09 provenance correction: refresh retains captured project metadata
            // where the SDK drops auto metadata while reloading an affected prompt group.
            if stage == "extend-prompts" {
                for p in expected["prompts"].as_array_mut().unwrap() {
                    if p["name"] == "base" {
                        p["sourceInfo"]["source"] = json!("auto");
                        p["sourceInfo"]["baseDir"] = json!("/fixture/project/.ag1");
                    }
                }
            }
            assert_eq!(actual, expected, "{variant}/{stage}");
            assert_eq!(json!(*calls.lock().unwrap()), row["calls"]);
            let evidence = snapshot.override_evidence.as_ref().unwrap();
            assert_eq!(evidence.original.system.as_deref(), Some("Base system."));
            assert_eq!(evidence.original.agents_files[0].content, "Base context.");
            assert_eq!(evidence.original.catalog.skills[0].name, "base");
            if stage == "reload" || variant == "combined" {
                assert_ne!(evidence.before_sha256, evidence.after_sha256);
            } else {
                assert_eq!(evidence.before_sha256, evidence.after_sha256);
            }
        }
    }
}

struct GuardOverrides {
    mode: Arc<std::sync::atomic::AtomicUsize>,
    source: std::path::PathBuf,
}
impl Overrides for GuardOverrides {
    fn apply(
        &self,
        view: &mut View,
        _: &[Kind],
        _: bool,
        _: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        match self.mode.load(std::sync::atomic::Ordering::SeqCst) {
            1 => {
                fs::write(&self.source, "Changed during override").unwrap();
            }
            2 => {
                view.system = Some("x".repeat(32 * 1024 * 1024 + 1));
            }
            3 => return Err(DomainError::InvalidInput("fixture callback failed".into())),
            4 => {
                view.system = Some("forbidden partial context change".into());
            }
            _ => {}
        }
        Ok(())
    }
}
#[test]
fn resource_override_errors_limits_stale_inputs_and_partial_context_mutation_never_publish() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("project/.ag1/prompts")).unwrap();
    fs::create_dir(dir.path().join("agent")).unwrap();
    let source = dir.path().join("project/.ag1/prompts/base.md");
    fs::write(&source, "Original").unwrap();
    let ws = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let resources = ResourceFs::new(vec![ws]).unwrap();
    let mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut loader = ResourceLoader::new(
        resources,
        Config {
            cwd: dir.path().join("project"),
            agent_dir: dir.path().join("agent"),
            home: dir.path().into(),
            additional: Paths::default(),
            disabled: vec![Kind::Extension],
            installed: vec![],
            system: None,
            append_system: None,
        },
    )
    .unwrap()
    .with_overrides(
        RegisteredOverride::new(
            "guard-fixture",
            Arc::new(GuardOverrides {
                mode: mode.clone(),
                source: source.clone(),
            }),
        )
        .unwrap(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    loader.reload(0, &control(&cancel)).unwrap();
    let initial = loader.snapshot().unwrap().fingerprint.clone();
    for value in [1, 2, 3] {
        mode.store(value, std::sync::atomic::Ordering::SeqCst);
        assert!(loader.reload(1, &control(&cancel)).is_err());
        assert_eq!(loader.generation(), 1);
        assert_eq!(loader.snapshot().unwrap().fingerprint, initial);
        fs::write(&source, "Original").unwrap();
    }
    mode.store(4, std::sync::atomic::Ordering::SeqCst);
    let mut paths = Paths::default();
    paths.prompts.push(ResolvedResource {
        path: ".ag1/prompts/base.md".into(),
        enabled: true,
        metadata: Metadata {
            source: "local".into(),
            scope: "project".into(),
            origin: "top-level".into(),
            base_dir: None,
        },
    });
    assert!(matches!(
        loader.extend_resources(1, paths, &control(&cancel)),
        Err(DomainError::PermissionDenied)
    ));
    assert_eq!(loader.generation(), 1);
    assert_eq!(loader.snapshot().unwrap().fingerprint, initial);
}

#[test]
fn native_extension_discovery_matches_sixteen_sdk_selection_cases() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/extension-discovery-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 16);
    let cancel = AtomicBool::new(false);
    for row in oracle["rows"].as_array().unwrap() {
        let input = &row["input"];
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("extensions")).unwrap();
        for (name, body) in input["files"].as_object().unwrap() {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body.as_str().unwrap()).unwrap();
        }
        let root = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        let resources = ResourceFs::new(vec![root]).unwrap();
        let mut roots = vec![];
        if input["mode"] != "explicit" {
            roots.push(dir.path().join("extensions"));
        }
        if let Some(explicit) = input["explicit"].as_array() {
            roots.extend(
                explicit
                    .iter()
                    .map(|p| dir.path().join(p.as_str().unwrap())),
            );
        }
        let mut paths = std::collections::BTreeSet::new();
        for root in roots {
            let scan = resources
                .scan(&root, Discovery::Extensions, control(&cancel))
                .unwrap();
            assert!(scan.diagnostics.is_empty(), "{}", input["id"]);
            paths.extend(scan.paths.into_iter().map(|p| {
                Path::new(&p)
                    .strip_prefix(dir.path())
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned()
            }));
        }
        assert_eq!(json!(paths), row["output"]["paths"], "{}", input["id"]);
    }
}

#[test]
fn selected_skill_fixtures_discovery_diagnostics_home_paths_and_collisions_match_sdk() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-skills-oracle.json"
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in oracle["files"].as_object().unwrap() {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content.as_str().unwrap()).unwrap();
    }
    for path in ["cwd", "agent"] {
        fs::create_dir(dir.path().join(path)).unwrap();
    }
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let resources = ResourceFs::new(vec![workspace]).unwrap();
    let cancel = AtomicBool::new(false);
    let mut checked = 0;
    for row in oracle["rows"].as_array().unwrap() {
        let mut catalog = match row["kind"].as_str().unwrap() {
            "directory" => {
                let mut captures = direct(
                    &resources,
                    &dir.path().join(row["path"].as_str().unwrap()),
                    Discovery::Skills {
                        root_markdown: true,
                    },
                    "temporary",
                    Kind::Skill,
                    &cancel,
                );
                for capture in &mut captures {
                    capture.resource.metadata.source = "test".into();
                }
                catalog::compile(&captures).unwrap()
            }
            "explicit" => {
                let paths = row["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let value = p.as_str().unwrap();
                        if value.starts_with('~') {
                            value.into()
                        } else {
                            dir.path().join(value).to_str().unwrap().into()
                        }
                    })
                    .collect::<Vec<String>>();
                ag1_platform::resources::load_skills(
                    &resources,
                    ag1_platform::resources::StandaloneSkills {
                        cwd: &dir.path().join("cwd"),
                        agent_dir: &dir.path().join("agent"),
                        home: &dir.path().join("home"),
                        paths: &paths,
                        include_defaults: false,
                    },
                    &control(&cancel),
                )
                .unwrap()
            }
            _ => continue,
        };
        checked += 1;
        for diagnostic in &mut catalog.diagnostics {
            if diagnostic.path.ends_with("/invalid-yaml/SKILL.md") {
                assert!(
                    diagnostic.message.contains("at line"),
                    "{}",
                    diagnostic.message
                );
                diagnostic.message = "<invalid YAML with location>".into();
            }
        }
        assert_eq!(
            normalize(
                json!({"skills":catalog.skills,"diagnostics":catalog.diagnostics}),
                dir.path()
            ),
            row["output"],
            "{row}"
        );
    }
    assert_eq!(checked, 20);
}

#[test]
fn selected_loader_disabling_overrides_project_precedence_and_explicit_extension_order_match_sdk() {
    use ag1_platform::resources::{Config, Paths, ResourceLoader};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/loader-selection-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 7);
    for row in oracle["rows"].as_array().unwrap() {
        let input = &row["input"];
        let dir = tempfile::tempdir().unwrap();
        for (p, body) in input["files"].as_object().unwrap() {
            let path = dir.path().join(p);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body.as_str().unwrap()).unwrap();
        }
        for p in ["project/.git", "agent", "home"] {
            fs::create_dir_all(dir.path().join(p)).unwrap();
        }
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        let resources = ResourceFs::new(vec![workspace]).unwrap();
        let mut additional = Paths::default();
        for (key, list) in [
            ("additionalSkills", &mut additional.skills),
            ("additionalExtensions", &mut additional.extensions),
        ] {
            if let Some(paths) = input[key].as_array() {
                for path in paths {
                    let path = dir.path().join(path.as_str().unwrap());
                    let base = if path.is_dir() {
                        path.clone()
                    } else {
                        path.parent().unwrap().to_owned()
                    };
                    list.push(ResolvedResource {
                        path: path.to_str().unwrap().into(),
                        enabled: true,
                        metadata: Metadata {
                            source: "local".into(),
                            scope: "temporary".into(),
                            origin: "top-level".into(),
                            base_dir: Some(base.to_str().unwrap().into()),
                        },
                    });
                }
            }
        }
        let mut loader = ResourceLoader::new(
            resources,
            Config {
                cwd: dir.path().join("project"),
                agent_dir: dir.path().join("agent"),
                home: dir.path().join("home"),
                additional,
                disabled: if input["disabledSkills"] == true {
                    vec![Kind::Skill]
                } else {
                    vec![]
                },
                installed: vec![],
                system: None,
                append_system: None,
            },
        )
        .unwrap();
        assert!(loader.snapshot().is_none());
        loader.reload(0, &control(&AtomicBool::new(false))).unwrap();
        let snapshot = loader.snapshot().unwrap();
        assert!(
            snapshot.diagnostics.is_empty(),
            "{:?}",
            snapshot.diagnostics
        );
        let expected = &row["output"];
        assert_eq!(
            normalize(json!(snapshot.catalog.skills), dir.path()),
            expected["skills"]["skills"],
            "{}",
            input["name"]
        );
        assert_eq!(
            normalize(json!(snapshot.catalog.prompts), dir.path()),
            expected["prompts"]["prompts"],
            "{}",
            input["name"]
        );
        assert_eq!(
            normalize(
                json!(
                    snapshot
                        .catalog
                        .themes
                        .iter()
                        .map(|t| json!({"name":t.name,"sourcePath":t.source_path}))
                        .collect::<Vec<_>>()
                ),
                dir.path()
            ),
            expected["themes"],
            "{}",
            input["name"]
        );
        assert_eq!(
            normalize(
                json!(
                    snapshot
                        .catalog
                        .extensions
                        .iter()
                        .map(|e| e.path.clone())
                        .collect::<Vec<_>>()
                ),
                dir.path()
            ),
            json!(
                expected["extensions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["path"].clone())
                    .collect::<Vec<_>>()
            ),
            "{}",
            input["name"]
        );
        let mut diagnostics = expected["skills"]["diagnostics"]
            .as_array()
            .unwrap()
            .clone();
        diagnostics.extend(
            expected["prompts"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        // Native theme duplicate diagnostics accompany the same project winner.
        let actual: Vec<_> = snapshot
            .catalog
            .diagnostics
            .iter()
            .filter(|d| {
                d.collision
                    .as_ref()
                    .is_none_or(|c| c["resourceType"] != "theme")
            })
            .collect();
        assert_eq!(
            normalize(json!(actual), dir.path()),
            json!(diagnostics),
            "{}",
            input["name"]
        );
    }
}
