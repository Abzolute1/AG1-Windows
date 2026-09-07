use super::*;
use ag1_core::{
    resources::{
        catalog::{ExtensionResource, Trust},
        prompts::SourceInfo,
        trust,
    },
    tools::registry::Scope,
};
use ag1_platform::{
    extension_host::{Config, ExtensionService},
    process::ProcessSupervisor,
    resource_fs::{ResourceFs, ScanControl},
    session::extensions::NativeExtensions,
};
use std::{collections::BTreeMap, path::Path};
mod compaction;
mod context;
mod input;
mod payload;
mod start;
mod tools;

struct LifecycleServices {
    workspace: Arc<ag1_platform::workspace::Workspace>,
    configuration: Configuration,
    files: Arc<FileRunner>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_client_binding_transfers_large_unicode_history_to_the_real_extension_host() {
    use ag1_platform::session::{
        commands::RunSettings,
        registry::{PreparedRuntime, RuntimeRegistry, StoredRuntimeFactory},
        runtime_commands::RuntimeCommands,
    };
    let content = "hé🔍".repeat(180_000);
    let mut f = fixture_seeded(
        "http://127.0.0.1:1",
        "disabled",
        vec![json!({"role":"user","content":content,"timestamp":0})],
    );
    let records = [
        json!({"type":"label","id":"label","parentId":"seed-0","timestamp":NOW,"targetId":"seed-0","label":"Bookmark"}),
        json!({"type":"session_info","id":"title","parentId":"label","timestamp":NOW,"name":"  Retained title  "}),
    ].iter().map(|r|serde_json::to_vec(r).unwrap()).collect::<Vec<_>>();
    f.store
        .append_batch("session", "inspection-off-path", 1, &records)
        .unwrap();
    f.store
        .select_leaf("session", "inspection-leaf", 3, Some("seed-0"))
        .unwrap();
    let state = SessionRuntime::resume(
        SessionId::new("session").unwrap(),
        1,
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
        f.host.configuration().unwrap(),
    )
    .unwrap();
    f.host = Arc::new(
        NativeSession::new(
            state,
            f.store.clone(),
            "session".into(),
            f.workspace.clone(),
            Arc::new(BlockingPool::new(1, 16).unwrap()),
            f.scheduler.clone(),
            ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap(),
        )
        .unwrap(),
    );
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';import {createHash} from 'node:crypto';export default api=>{
        api.on('session_start',(e,ctx)=>{const branch=ctx.sessionManager.getBranch(); const entries=ctx.sessionManager.getEntries();
        fs.appendFileSync(ctx.cwd+'/large-start',JSON.stringify({reason:e.reason,count:entries.length,branch:branch.length,
        label:ctx.sessionManager.getLabel('seed-0'),title:ctx.sessionManager.getSessionName(),
        alternate:ctx.sessionManager.getBranch('title').map(e=>e.id),treeLabel:ctx.sessionManager.getTree()[0].label,
        cwd:ctx.sessionManager.getCwd(),hash:createHash('sha256').update(branch[0].message.content).digest('hex')})+'\n');});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let registry = Arc::new(
        RuntimeRegistry::durable(
            PreparedRuntime::from_shared(
                f.host.clone(),
                Arc::new(Resolver::new(false)),
                f.files.clone(),
            ),
            None,
            "select-initial".into(),
        )
        .unwrap(),
    );
    registry.reconcile_selection().await.unwrap();
    let factory = StoredRuntimeFactory {
        store: f.store.clone(),
        pool: Arc::new(BlockingPool::new(1, 16).unwrap()),
        scheduler: f.scheduler.clone(),
        http: ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap(),
        services: Arc::new(LifecycleServices {
            workspace: f.workspace.clone(),
            configuration: f.host.configuration().unwrap(),
            files: f.files.clone(),
        }),
    };
    let host = RuntimeCommands::new(
        registry,
        Arc::new(factory),
        "client".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    host.initialize().await.unwrap();
    host.initialize().await.unwrap();
    let text = std::fs::read_to_string(f.root.join("large-start")).unwrap();
    assert_eq!(text.lines().count(), 1);
    let event: Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(
        event,
        json!({"reason":"startup","count":3,"branch":1,"label":"Bookmark","title":"Retained title",
        "alternate":["seed-0","label","title"],"treeLabel":"Bookmark","cwd":f.root,"hash":ag1_storage::digest(content.as_bytes())})
    );
    assert!(f.host.accepts_commands().unwrap());
    assert_eq!(f.host.journal_revision().unwrap(), 4);
    extension.stop().unwrap();
}
impl ag1_platform::session::registry::ServiceFactory for LifecycleServices {
    fn prepare<'a>(
        &'a self,
        input: ag1_platform::session::registry::ServiceInput<'a>,
    ) -> PortFuture<'a, ag1_platform::session::registry::ScopedServices> {
        Box::pin(async move {
            let mut files = (*self.files).clone();
            files.origin = input.target.session.as_str().into();
            files.adapters.extensions = None;
            Ok(ag1_platform::session::registry::ScopedServices {
                workspace: self.workspace.clone(),
                origin: input.target.session.as_str().into(),
                configuration: self.configuration.clone(),
                provider: Arc::new(Resolver::new(false)),
                tools: Arc::new(files),
            })
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_lifecycle_callbacks_veto_before_creation_and_failed_shutdown_keeps_previous_selection()
 {
    use ag1_platform::session::registry::{
        PreparedRuntime, RuntimeRegistry, SessionAction, SessionRequest, StoredRuntimeFactory,
    };
    for scenario in ["cancel-new", "cancel-fork", "shutdown-exit"] {
        let mut f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1);
        let code = if scenario == "shutdown-exit" {
            r#"export default api=>{api.on('session_shutdown',()=>{process.exit(23)});};"#
        } else {
            r#"import fs from 'node:fs';export default api=>{
              for(const event of ['session_before_switch','session_before_fork'])api.on(event,(e,ctx)=>{
                fs.appendFileSync(ctx.cwd+'/lifecycle-log',JSON.stringify({type:e.type,session:ctx.sessionManager.getSessionId(),model:ctx.model.id,idle:ctx.isIdle(),branch:ctx.sessionManager.getBranch().length})+'\n');return {cancel:true};});
            };"#
        };
        let extension = extensions(&f, code);
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let registry = RuntimeRegistry::durable(
            PreparedRuntime::from_shared(
                f.host.clone(),
                Arc::new(Resolver::new(false)),
                f.files.clone(),
            ),
            None,
            "select-initial".into(),
        )
        .unwrap();
        registry.reconcile_selection().await.unwrap();
        let factory = StoredRuntimeFactory {
            store: f.store.clone(),
            pool: Arc::new(BlockingPool::new(1, 16).unwrap()),
            scheduler: f.scheduler.clone(),
            http: ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1))
                .unwrap(),
            services: Arc::new(LifecycleServices {
                workspace: f.workspace.clone(),
                configuration: f.host.configuration().unwrap(),
                files: f.files.clone(),
            }),
        };
        let action = if scenario == "cancel-fork" {
            SessionAction::Fork {
                entry: "invalid-user-entry".into(),
            }
        } else {
            SessionAction::New
        };
        let result = registry
            .change_session(
                1,
                SessionRequest {
                    command: "bootstrap-next".into(),
                    target: SessionId::new("next").unwrap(),
                    expected_revision: 0,
                    timestamp: NOW.into(),
                    action,
                },
                &factory,
            )
            .await;
        if scenario.starts_with("cancel") {
            assert!(matches!(result, Err(DomainError::Cancelled)));
            assert!(f.store.snapshot("next").is_err());
            let event: Value = serde_json::from_str(
                std::fs::read_to_string(f.root.join("lifecycle-log"))
                    .unwrap()
                    .trim(),
            )
            .unwrap();
            assert_eq!(event["session"], "session");
            assert_eq!(event["model"], "initial");
            assert_eq!(event["idle"], true);
            assert_eq!(event["branch"], 0);
            assert_eq!(
                event["type"],
                if scenario == "cancel-fork" {
                    "session_before_fork"
                } else {
                    "session_before_switch"
                }
            );
        } else {
            assert!(result.is_err());
            assert!(f.store.snapshot("next").is_ok());
        }
        assert_eq!(registry.selected().unwrap().epoch, 1);
        assert!(f.host.accepts_commands().unwrap());
        assert_eq!(
            f.store
                .runtime_selection(f.workspace.id().as_str())
                .unwrap()
                .unwrap()
                .selection
                .session,
            "session"
        );
        extension.stop().unwrap();
    }
}

pub(super) fn extensions(f: &Fixture, code: &str) -> Arc<NativeExtensions> {
    extensions_configured(f, code, None)
}
fn extensions_configured(
    f: &Fixture,
    code: &str,
    resources: Option<ag1_platform::resources::ResourceLoader>,
) -> Arc<NativeExtensions> {
    let config = f.root.join(".ag1/extensions");
    std::fs::create_dir_all(&config).unwrap();
    let source = config.join("fixture.ts");
    std::fs::write(&source, code).unwrap();
    let spools = f._directory.path().join("extension-spools");
    std::fs::create_dir(&spools).unwrap();
    let workspaces = WorkspaceService::new().unwrap();
    let root = workspaces.register(f._directory.path()).unwrap();
    let workspace = workspaces.register(&f.root).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let service = ExtensionService::new(
        workspace.clone(),
        ResourceFs::new(vec![root]).unwrap(),
        Arc::new(
            ProcessSupervisor::new(Path::new(env!("CARGO_BIN_EXE_ag1-process-host"))).unwrap(),
        ),
        Config {
            node: project.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
            host_script: project.join("product/compat/extensions/host.mjs"),
            spools,
            env: BTreeMap::new(),
        },
    )
    .unwrap();
    let source = source.to_str().unwrap().to_owned();
    let entry = ExtensionResource {
        path: source.clone(),
        sha256: ag1_storage::digest(code.as_bytes()),
        source_info: SourceInfo {
            path: source,
            source: "auto".into(),
            scope: "project".into(),
            origin: "top-level".into(),
            base_dir: Some(config.parent().unwrap().to_str().unwrap().into()),
        },
        trust: Trust::UntrustedData,
    };
    let scope = Scope {
        workspace: workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: f.host.journal_revision().unwrap(),
    };
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let prepared = service
        .prepare(
            &scope,
            &[entry],
            trust::Policy {
                project_config_dir: config.parent().unwrap().to_str().unwrap(),
                canonical_project_dir: config.parent().unwrap().to_str().unwrap(),
                explicit: Some(true),
                environment: None,
                user_trusted_projects: &[],
            },
            &control,
        )
        .unwrap();
    let extension =
        NativeExtensions::bind(service.start(prepared, &scope, &control).unwrap(), &control)
            .unwrap();
    Arc::new(match resources {
        Some(resources) => extension.with_resources(resources).unwrap(),
        None => extension,
    })
}

struct ResourceFault(ag1_storage::Boundary, AtomicBool);
impl ag1_storage::FaultInjector for ResourceFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        boundary: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command.starts_with("resources-")
            && boundary == self.0
            && self.1.swap(false, Ordering::AcqRel)
        {
            return Err(ag1_storage::StorageError::Injected(boundary));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovered_resources_acknowledge_prompt_before_client_input_and_do_not_replay_on_failed_custody()
 {
    use ag1_platform::{
        resources::{Config as ResourceConfig, Paths, ResourceLoader},
        session::{
            commands::RunSettings,
            registry::{PreparedRuntime, RuntimeRegistry, StoredRuntimeFactory},
            runtime_commands::RuntimeCommands,
        },
    };
    for fault in [
        None,
        Some(ag1_storage::Boundary::BeforeCommit),
        Some(ag1_storage::Boundary::BeforeAck),
    ] {
        let injector = fault.map(|b| {
            Arc::new(ResourceFault(b, AtomicBool::new(true))) as Arc<dyn ag1_storage::FaultInjector>
        });
        let mut f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], injector, None, 1);
        let root = f._directory.path();
        std::fs::create_dir(root.join("agent")).unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        std::fs::create_dir_all(f.root.join("extras/checks")).unwrap();
        std::fs::write(f.root.join("extras/checks/SKILL.md"), "---\nname: discovered-skill\ndescription: Use the extension-discovered checks.\n---\nOwned content.").unwrap();
        let resource_root = WorkspaceService::new().unwrap().register(root).unwrap();
        let mut loader = ResourceLoader::new(
            ResourceFs::new(vec![resource_root]).unwrap(),
            ResourceConfig {
                cwd: f.root.clone(),
                agent_dir: root.join("agent"),
                home: root.join("home"),
                additional: Paths::default(),
                disabled: vec![],
                installed: vec![],
                system: None,
                append_system: None,
            },
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        loader
            .reload(
                0,
                &ScanControl {
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(5),
                },
            )
            .unwrap();
        assert!(loader.snapshot().unwrap().catalog.skills.is_empty());
        let extension = extensions_configured(
            &f,
            r#"import fs from 'node:fs'; export default api=>{
            api.on('session_start',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/resource-events',e.type+'\n');});
            api.on('resources_discover',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/resource-events',e.type+':'+e.reason+'\n');return {skillPaths:[ctx.cwd+'/extras/checks']};});};"#,
            Some(loader),
        );
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let registry = Arc::new(
            RuntimeRegistry::durable(
                PreparedRuntime::from_shared(
                    f.host.clone(),
                    Arc::new(Resolver::new(false)),
                    f.files.clone(),
                ),
                None,
                "select-initial".into(),
            )
            .unwrap(),
        );
        registry.reconcile_selection().await.unwrap();
        let factory = StoredRuntimeFactory {
            store: f.store.clone(),
            pool: Arc::new(BlockingPool::new(1, 16).unwrap()),
            scheduler: f.scheduler.clone(),
            http: ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1))
                .unwrap(),
            services: Arc::new(LifecycleServices {
                workspace: f.workspace.clone(),
                configuration: f.host.configuration().unwrap(),
                files: f.files.clone(),
            }),
        };
        let host = RuntimeCommands::new(
            registry,
            Arc::new(factory),
            "client".to_owned().try_into().unwrap(),
            Arc::new(FixedClock),
            RunSettings::default(),
            tokio::runtime::Handle::current(),
        )
        .unwrap();
        let result = host.initialize().await;
        assert_eq!(result.is_ok(), fault.is_none(), "{fault:?}: {result:?}");
        assert_eq!(host.initialize().await.is_ok(), fault.is_none());
        assert_eq!(
            std::fs::read_to_string(f.root.join("resource-events")).unwrap(),
            "session_start\nresources_discover:startup\n"
        );
        let journal = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        assert_eq!(
            journal.entries.len(),
            usize::from(fault != Some(ag1_storage::Boundary::BeforeCommit))
        );
        if fault.is_none() {
            assert!(
                f.host
                    .configuration()
                    .unwrap()
                    .system_prompt
                    .contains("discovered-skill")
            );
            assert_eq!(host.snapshot().unwrap().revision.get(), 1);
        } else {
            assert!(!f.host.persistence_healthy().unwrap());
            assert!(host.run_next(&ignore).await.is_err());
        }
        extension.stop().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_extension_hooks_block_before_write_and_preserve_native_output_under_projection() {
    let (endpoint, server) = server(vec![
        calls(vec![
            (
                "blocked",
                "write",
                json!({"path":"blocked.txt","content":"forbidden"}),
            ),
            (
                "allowed",
                "write",
                json!({"path":"output.txt","content":"actual native bytes"}),
            ),
            ("read", "read", json!({"path":"output.txt"})),
        ]),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs'; export default api=>{
      api.on('tool_call',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/hook-log',JSON.stringify({type:e.type,id:e.toolCallId,branch:ctx.sessionManager.getBranch().map(e=>e.message?.toolCallId).filter(Boolean),model:ctx.model.id})+'\n');if(e.input.path==='blocked.txt')return {block:true,reason:'Fixture policy'};});
      api.on('tool_result',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/hook-log',JSON.stringify({type:e.type,id:e.toolCallId})+'\n');return {content:[{type:'text',text:'hook display'}],details:{pretendNative:true}};});
    };"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert!(!f.root.join("blocked.txt").exists());
    assert_eq!(
        std::fs::read_to_string(f.root.join("output.txt")).unwrap(),
        "actual native bytes"
    );
    let events: Vec<Value> = std::fs::read_to_string(f.root.join("hook-log"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|e| (e["type"].as_str().unwrap(), e["id"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        vec![
            ("tool_call", "blocked"),
            ("tool_call", "allowed"),
            ("tool_result", "allowed"),
            ("tool_call", "read"),
            ("tool_result", "read")
        ]
    );
    assert_eq!(events[1]["branch"], json!(["blocked"]));
    assert_eq!(events[3]["branch"], json!(["blocked", "allowed"]));
    let journal = f.store.snapshot("session").unwrap();
    let entries: Vec<Value> = journal
        .entries
        .iter()
        .map(|entry| serde_json::from_slice(&entry.normalized).unwrap())
        .collect();
    let result = entries
        .iter()
        .find(|entry| entry["message"]["toolCallId"] == "read")
        .unwrap();
    assert_eq!(result["message"]["content"][0]["text"], "hook display");
    assert_eq!(result["message"]["details"]["postHookOutput"], true);
    let artifact: ag1_core::context::ContextArtifact =
        serde_json::from_value(result["message"]["details"]["nativeResultArtifact"].clone())
            .unwrap();
    let artifacts = ag1_storage::context::ContextStore::new(
        &f.store,
        SessionId::new("session").unwrap(),
        "session",
        "inspect-hook",
        NOW,
        Default::default(),
    )
    .unwrap();
    use ag1_core::tools::executor::ExecutionArtifacts;
    let native = artifacts.read_json(&artifact).unwrap();
    assert!(
        native["content"]
            .to_string()
            .contains("actual native bytes")
    );
    assert_ne!(native["details"]["pretendNative"], true);
    extension.stop().unwrap();
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_crash_after_native_write_retains_its_receipt_and_blocks_later_hooks() {
    let (endpoint, server) = server(vec![
        calls(vec![
            (
                "write",
                "write",
                json!({"path":"output.txt","content":"retained"}),
            ),
            (
                "later",
                "write",
                json!({"path":"later.txt","content":"forbidden"}),
            ),
        ]),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        "export default api=>{api.on('tool_call',()=>{});api.on('tool_result',()=>process.exit(17));};",
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("output.txt")).unwrap(),
        "retained"
    );
    assert!(!f.root.join("later.txt").exists());
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_hooks_refresh_after_compaction_and_an_acknowledged_task_mutation() {
    let (endpoint, server) = server(vec![
        stop(),
        stop(),
        calls(vec![
            (
                "state",
                "taskState",
                json!({"action":"set","currentPhase":"extension-current-phase"}),
            ),
            (
                "write",
                "write",
                json!({"path":"output.txt","content":"after task update"}),
            ),
        ]),
        stop(),
    ]);
    let f = fixture_seeded(&endpoint, "disabled", summary_history());
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('tool_call',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/scopes',JSON.stringify({type:e.type,id:e.toolCallId,entries:ctx.sessionManager.getBranch()})+'\n');});api.on('tool_result',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/scopes',JSON.stringify({type:e.type,id:e.toolCallId,entries:ctx.sessionManager.getBranch()})+'\n');});};"#,
    );
    f.host
        .compact(compact_input(), &Resolver::new(false), &ignore)
        .await
        .unwrap()
        .unwrap();
    let mut runner = native_task_runner(&f).await;
    let native = Arc::get_mut(&mut runner).unwrap();
    native.adapters.extensions = Some(extension.clone());
    let registry = Arc::get_mut(&mut native.registry).unwrap();
    registry
        .activate(
            &["taskState".into(), "write".into()],
            &native.adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| matches!(tool["name"].as_str(), Some("taskState" | "write")))
        .collect();
    f.host
        .select_tools(
            "extension-task-tools",
            "extension-task-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    f.host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let events: Vec<Value> = std::fs::read_to_string(f.root.join("scopes"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 4);
    assert_eq!(events[1]["type"], "tool_result");
    assert_eq!(events[1]["id"], "state");
    assert!(
        events[1]["entries"]
            .to_string()
            .contains("extension-current-phase")
    );
    assert!(
        events[0]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["type"] == "compaction" && entry["contextEpoch"] == 2)
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("output.txt")).unwrap(),
        "after task update"
    );
    extension.stop().unwrap();
    assert_eq!(server.join().unwrap().len(), 4);
}
