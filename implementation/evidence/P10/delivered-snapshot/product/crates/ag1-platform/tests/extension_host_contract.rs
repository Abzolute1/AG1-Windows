#![cfg(target_os = "linux")]
use ag1_core::{
    DomainError,
    resources::{
        catalog::{ExtensionResource, Trust},
        prompts::SourceInfo,
        trust,
    },
    tools::registry::Scope,
};
use ag1_platform::{
    extension_host::{self, Bridge, DenyBridge, ExtensionService, NativeRequest},
    process::ProcessSupervisor,
    resource_fs::{ResourceFs, ScanControl},
    workspace::WorkspaceService,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
struct Fixture {
    dir: tempfile::TempDir,
    service: ExtensionService,
    scope: Scope,
    entry: ExtensionResource,
    code: String,
}
impl Fixture {
    fn new(code: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("project/.ag1/extensions")).unwrap();
        fs::create_dir(dir.path().join("spools")).unwrap();
        let workspaces = WorkspaceService::new().unwrap();
        let root = workspaces.register(dir.path()).unwrap();
        let ws = workspaces.register(&dir.path().join("project")).unwrap();
        let source = dir.path().join("project/.ag1/extensions/fixture.ts");
        fs::write(&source, code).unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        let service = ExtensionService::new(
            ws.clone(),
            ResourceFs::new(vec![root]).unwrap(),
            Arc::new(
                ProcessSupervisor::new(Path::new(env!("CARGO_BIN_EXE_ag1-process-host"))).unwrap(),
            ),
            extension_host::Config {
                node: project.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
                host_script: project.join("product/compat/extensions/host.mjs"),
                spools: dir.path().join("spools"),
                env: BTreeMap::new(),
            },
        )
        .unwrap();
        let source = source.to_str().unwrap().to_owned();
        let entry = ExtensionResource {
            path: source.clone(),
            sha256: format!("{:x}", Sha256::digest(code)),
            source_info: SourceInfo {
                path: source,
                source: "auto".into(),
                scope: "project".into(),
                origin: "top-level".into(),
                base_dir: Some(dir.path().join("project/.ag1").to_str().unwrap().into()),
            },
            trust: Trust::UntrustedData,
        };
        let scope = Scope {
            workspace: ws.id().as_str().into(),
            session: "session".into(),
            epoch: 1,
            revision: 0,
        };
        Self {
            dir,
            service,
            scope,
            entry,
            code: code.into(),
        }
    }
    fn prepare(&self, trusted: bool, cancel: &AtomicBool) -> extension_host::PreparedExtensions {
        let config = self.dir.path().join("project/.ag1");
        self.service
            .prepare(
                &self.scope,
                std::slice::from_ref(&self.entry),
                trust::Policy {
                    project_config_dir: config.to_str().unwrap(),
                    canonical_project_dir: config.to_str().unwrap(),
                    explicit: Some(trusted),
                    environment: None,
                    user_trusted_projects: &[],
                },
                &control(cancel),
            )
            .unwrap()
    }
}
fn control(cancel: &AtomicBool) -> ScanControl<'_> {
    ScanControl {
        cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
#[derive(Default)]
struct FixtureBridge {
    requests: Vec<Value>,
    events: Vec<Value>,
}
impl Bridge for FixtureBridge {
    fn request(
        &mut self,
        r: NativeRequest<'_>,
        _: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        self.requests.push(json!({"scope":{"workspace":r.scope.workspace,"session":r.scope.session,"epoch":r.scope.epoch,"revision":r.scope.revision},"operation":r.operation,"phase":r.phase,"extension":r.extension,"method":r.method,"args":r.arguments}));
        match r.method {
            "ui.select" => Ok(Some(json!("two"))),
            "session.appendEntry" => Ok(None),
            _ => Err(DomainError::PermissionDenied),
        }
    }
    fn event(&mut self, _: &Scope, _: &str, event: &Value) -> Result<(), DomainError> {
        self.events.push(event.clone());
        Ok(())
    }
}
#[test]
fn native_reentrant_provider_hooks_preserve_helper_state_and_defer_other_parent_callbacks() {
    use extension_host::{NativeStream, ProviderInvocation};
    #[derive(Default)]
    struct NestedBridge {
        registration: String,
        results: Vec<Value>,
        phases: Vec<(String, String)>,
    }
    impl Bridge for NestedBridge {
        fn request(
            &mut self,
            _: NativeRequest<'_>,
            _: &ScanControl<'_>,
        ) -> Result<Option<Value>, DomainError> {
            Err(DomainError::PermissionDenied)
        }
        fn request_stream(
            &mut self,
            r: NativeRequest<'_>,
            _: &ScanControl<'_>,
            stream: &mut dyn NativeStream,
        ) -> Result<Option<Value>, DomainError> {
            self.phases.push((r.method.into(), r.phase.into()));
            match r.method {
                "provider.register" | "session.setThinkingLevel" => {
                    if r.method == "provider.register" {
                        assert_eq!(r.phase, "bind");
                        self.registration = r.arguments["registration"].as_str().unwrap().into();
                    } else {
                        assert_eq!(r.phase, "command");
                    }
                    let registration = self.registration.clone();
                    let value = stream
                        .invoke_provider(
                            ProviderInvocation {
                                deadline: None,
                                name: "fixture",
                                registration: &registration,
                                hook: "oauth.modifyModels",
                                arguments: &json!({"models":[{"id":"one"}],"credentials":{}}),
                            },
                            self,
                        )?
                        .unwrap();
                    self.results.push(value);
                    Ok(Some(json!({"retainRegistrations":[registration]})))
                }
                "model.set" => {
                    assert_eq!(r.phase, "provider");
                    let registration = self.registration.clone();
                    assert_eq!(
                        stream.invoke_provider(
                            ProviderInvocation {
                                deadline: None,
                                name: "fixture",
                                registration: &registration,
                                hook: "oauth.getApiKey",
                                arguments: &json!({"credentials":{"access":"fixture-access"}})
                            },
                            self
                        )?,
                        Some(json!("fixture-access"))
                    );
                    Ok(Some(json!(true)))
                }
                "session.appendEntry" => {
                    assert_eq!(r.phase, "command");
                    Ok(None)
                }
                _ => Err(DomainError::PermissionDenied),
            }
        }
    }
    let f = Fixture::new(
        "export default api=>{let calls=0;api.registerProvider('fixture',{api:'custom',oauth:{name:'Fixture',getApiKey:c=>c.access,modifyModels:async models=>{calls++;await api.setModel({provider:'fixture',id:'one'});return models.map(m=>({...m,calls}));}}});api.registerCommand('again',{handler:()=>{api.setThinkingLevel('high');api.appendEntry('outside',{});}});};",
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut bridge = NestedBridge::default();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    host.call(
        &f.scope,
        "command",
        &json!({"name":"again","arguments":""}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    assert_eq!(
        bridge.results,
        json!([[{"id":"one","calls":1}],[{"id":"one","calls":2}]])
            .as_array()
            .unwrap()
            .clone()
    );
    assert_eq!(
        bridge.phases.last().unwrap(),
        &("session.appendEntry".into(), "command".into())
    );
    host.stop().unwrap();
}

#[test]
fn nested_provider_failure_is_isolated_and_uncooperative_hooks_obey_native_deadlines() {
    use extension_host::{NativeStream, ProviderInvocation};
    struct InvokeBridge;
    impl Bridge for InvokeBridge {
        fn request(
            &mut self,
            _: NativeRequest<'_>,
            _: &ScanControl<'_>,
        ) -> Result<Option<Value>, DomainError> {
            Err(DomainError::PermissionDenied)
        }
        fn request_stream(
            &mut self,
            request: NativeRequest<'_>,
            _: &ScanControl<'_>,
            stream: &mut dyn NativeStream,
        ) -> Result<Option<Value>, DomainError> {
            if request.method != "provider.register" {
                return Err(DomainError::PermissionDenied);
            }
            stream.invoke_provider(
                ProviderInvocation {
                    deadline: None,
                    name: "fixture",
                    registration: request.arguments["registration"].as_str().unwrap(),
                    hook: "oauth.modifyModels",
                    arguments: &json!({"models":[],"credentials":{}}),
                },
                &mut DenyBridge,
            )
        }
    }
    for fail in [true, false] {
        let body = if fail {
            "throw new Error('fixture hook failure');"
        } else {
            "fs.writeFileSync('./provider-started',String(process.pid));while(true){}"
        };
        let f = Fixture::new(&format!(
            "import fs from 'node:fs';export default api=>api.registerProvider('fixture',{{api:'custom',oauth:{{name:'Fixture',modifyModels:()=>{{{body}}}}}}});"
        ));
        let cancel = AtomicBool::new(false);
        let mut host = f
            .service
            .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
            .unwrap();
        let start = Instant::now();
        let result = host.call(
            &f.scope,
            "bind",
            &json!({"snapshot":{}}),
            &ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_millis(200),
            },
            &mut InvokeBridge,
        );
        if fail {
            assert_eq!(
                result.unwrap().unwrap()["errors"].as_array().unwrap().len(),
                1
            );
            assert!(
                host.call(
                    &f.scope,
                    "catalog",
                    &json!({}),
                    &control(&cancel),
                    &mut DenyBridge
                )
                .is_ok()
            );
        } else {
            assert!(matches!(result, Err(DomainError::TimedOut)));
            assert!(start.elapsed() < Duration::from_secs(2));
            let pid = fs::read_to_string(f.dir.path().join("project/provider-started")).unwrap();
            assert!(!Path::new(&format!("/proc/{pid}")).exists());
            assert!(
                host.call(
                    &f.scope,
                    "catalog",
                    &json!({}),
                    &control(&cancel),
                    &mut DenyBridge
                )
                .is_err()
            );
        }
        host.stop().unwrap();
    }
}

#[test]
fn native_provider_registry_admits_names_invokes_staged_oauth_and_restores_baseline() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::extension_providers::{Grant, OAuth, RegistryBridge};
    struct FixtureOAuth;
    impl OAuth for FixtureOAuth {
        fn credentials(
            &mut self,
            provider: &str,
            extension: &str,
            control: &ScanControl<'_>,
        ) -> Result<Option<Value>, DomainError> {
            control.check()?;
            assert_eq!(provider, "fixture");
            assert!(extension.ends_with("fixture.ts"));
            Ok(Some(json!({"type":"oauth","access":"fixture-token"})))
        }
    }
    let f = Fixture::new(
        "import {getModels} from '@ag1/ai';export default api=>{let calls=0;const model={id:'model',name:'Model',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:8192,maxTokens:1024};api.registerProvider('fixture',{api:'fixture-api',baseUrl:'https://fixture.invalid',models:[model],oauth:{name:'Fixture',modifyModels:(models,c)=>{if(c.access!=='fixture-token')throw new Error('wrong credentials');calls++;return models.map(m=>m.provider==='fixture'?{...m,name:m.name+' via '+calls}:m);}}});api.registerCommand('models',{handler:()=>getModels('fixture')});api.registerCommand('settings',{handler:()=>api.registerProvider('fixture',{baseUrl:'https://override.invalid'})});api.registerCommand('deny',{handler:()=>api.registerProvider('local',{baseUrl:'https://ungranted.invalid'})});api.registerCommand('reject',{handler:()=>api.registerProvider('fixture',{api:'fixture-api',baseUrl:'https://fixture.invalid',models:[model],oauth:{name:'Fixture',modifyModels:()=>{throw new Error('fixture rejected')}}})});api.registerCommand('remove',{handler:()=>api.unregisterProvider('fixture')});};",
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let preserved = json!({"provider":"local","id":"preserved","name":"Preserved native model","api":"openai-completions","baseUrl":"http://127.0.0.1:8080"});
    let baseline = Snapshot {
        models: vec![preserved.clone()],
        ..Snapshot::default()
    };
    let mut bridge = RegistryBridge::new(
        &host,
        baseline,
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: std::collections::BTreeSet::from(["fixture".into()]),
        }],
        FixtureOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = bridge.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    assert_eq!(catalog.generation().unwrap(), 1);
    let models = host
        .call(
            &f.scope,
            "command",
            &json!({"name":"models","arguments":""}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    assert_eq!(models[0]["name"], "Model via 1");
    assert_eq!(catalog.models().unwrap()[0], preserved);
    host.call(
        &f.scope,
        "command",
        &json!({"name":"settings","arguments":""}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    let before = catalog.models().unwrap();
    assert_eq!(before[1]["baseUrl"], "https://override.invalid");
    for command in ["deny", "reject"] {
        assert!(
            host.call(
                &f.scope,
                "command",
                &json!({"name":command,"arguments":""}),
                &control(&cancel),
                &mut bridge
            )
            .is_err()
        );
        assert_eq!(catalog.models().unwrap(), before);
        assert_eq!(catalog.generation().unwrap(), 2);
    }
    host.call(
        &f.scope,
        "command",
        &json!({"name":"remove","arguments":""}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    assert_eq!(catalog.models().unwrap(), vec![preserved]);
    assert_eq!(
        host.call(
            &f.scope,
            "command",
            &json!({"name":"models","arguments":""}),
            &control(&cancel),
            &mut bridge
        )
        .unwrap(),
        Some(json!([]))
    );
    host.stop().unwrap();
}

#[test]
fn custom_extension_streams_share_native_admission_and_forward_live_events_without_stock_fallback()
{
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        extension_custom_models::{self, CustomModels},
        extension_providers::{Grant, NoOAuth, RegistryBridge},
    };
    struct NoNative;
    impl ModelResolver for NoNative {
        fn resolve(
            &mut self,
            _: InferenceInput<'_>,
            _: &ScanControl<'_>,
        ) -> Result<ag1_platform::extension_inference::ResolvedGeneration, DomainError> {
            panic!("custom provider used stock fallback")
        }
    }
    struct CustomCredentials;
    impl extension_custom_models::Credentials for CustomCredentials {
        fn options(
            &mut self,
            owner: &str,
            _: &Value,
            mut options: Value,
            control: &ScanControl<'_>,
        ) -> Result<Value, DomainError> {
            control.check()?;
            assert!(owner.ends_with("fixture.ts"));
            options["apiKey"] = json!("native-custom-fixture-key");
            Ok(options)
        }
    }
    let f = Fixture::new(
        r#"import fs from 'node:fs';import {stream,getModel} from '@ag1/ai';export default api=>{
        let calls=0;api.registerProvider('fixture',{api:'fixture-custom-api',baseUrl:'https://fixture.invalid',apiKey:'fixture-registration-key',models:[{id:'model',name:'Model',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:8192,maxTokens:1024}],streamSimple:(model,context,options)=>{
            if(options.apiKey!=='native-custom-fixture-key')throw new Error('native credential missing');
            fs.writeFileSync('./custom-started','yes');calls++;
            const message={role:'assistant',provider:model.provider,model:model.id,api:model.api,content:[],usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:12};
            return {async *[Symbol.asyncIterator](){yield {type:'start',partial:structuredClone(message)};message.content=[{type:'text',text:'custom '+calls}];yield {type:'text_delta',contentIndex:0,delta:message.content[0].text,partial:structuredClone(message)};
                for(let i=0;!fs.existsSync('./observed-stream')&&i<100;i++)await new Promise(r=>setTimeout(r,5));
                if(!fs.existsSync('./observed-stream'))throw new Error('stream was buffered until completion');
                yield {type:'done',reason:'stop',message};},async result(){return message;}};
        }});
        api.registerCommand('infer',{handler:async()=>{const events=stream({...getModel('fixture','model'),baseUrl:'https://forged.invalid'},{messages:[{role:'user',content:'hello',timestamp:1}]},{temperature:0.2});let text='';for await(const event of events){if(event.type==='text_delta'){text+=event.delta;fs.writeFileSync('./observed-stream','yes');}}return {text,result:await events.result()};}});
    };"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut registration = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: std::collections::BTreeSet::from(["fixture".into()]),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registration.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registration,
    )
    .unwrap();
    let endpoint = "owned-local-endpoint";
    let config = extension_custom_models::Config {
        provider: "fixture".into(),
        model: "model".into(),
        endpoint: endpoint.into(),
        callers: std::collections::BTreeSet::from([f.entry.path.clone()]),
        hook_owners: std::collections::BTreeSet::from([f.entry.path.clone()]),
        origins: std::collections::BTreeSet::from(["https://fixture.invalid".into()]),
        options: json!({}),
        allowed_options: std::collections::BTreeSet::from(["temperature".into()]),
        max_output_bytes: 65536,
        overall_ms: 2000,
    };
    let resolver = CustomModels::new(
        catalog,
        vec![config],
        NoNative,
        CustomCredentials,
        Arc::new(TestClock),
    )
    .unwrap();
    let scheduler = Scheduler::default();
    scheduler.register(endpoint, EndpointKind::Local).unwrap();
    let root = scheduler
        .submit(endpoint, OperationId::new("custom-root-fixture").unwrap())
        .unwrap();
    let permit = scheduler.start(&root).unwrap().unwrap();
    let marker = f.dir.path().join("project/custom-started");
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        assert!(!marker.exists(), "custom hook bypassed shared admission");
        permit.terminal().unwrap();
    });
    let mut bridge = InferenceBridge::new(
        scheduler.clone(),
        ProviderHttp::new(&["https://fixture.invalid".into()], Duration::from_secs(1)).unwrap(),
        resolver,
        registration,
    )
    .unwrap();
    let output = host
        .call(
            &f.scope,
            "command",
            &json!({"name":"infer","arguments":""}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    release.join().unwrap();
    assert_eq!(output["text"], "custom 1");
    assert_eq!(output["result"]["model"], "model");
    assert_eq!(scheduler.status(endpoint).unwrap().active, 0);
    assert_eq!(scheduler.status(endpoint).unwrap().quarantined, 0);
    host.stop().unwrap();
}

#[test]
fn custom_provider_registration_cannot_grant_inference_or_credential_access() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        extension_custom_models::{self, CustomModels},
        extension_providers::{Grant, NoOAuth, RegistryBridge},
    };
    struct NoFallback;
    impl ModelResolver for NoFallback {
        fn resolve(
            &mut self,
            _: InferenceInput<'_>,
            _: &ScanControl<'_>,
        ) -> Result<ag1_platform::extension_inference::ResolvedGeneration, DomainError> {
            panic!("registered custom model must not use a stock fallback")
        }
    }
    struct NoCredentials;
    impl extension_custom_models::Credentials for NoCredentials {
        fn options(
            &mut self,
            _: &str,
            _: &Value,
            _: Value,
            _: &ScanControl<'_>,
        ) -> Result<Value, DomainError> {
            panic!("denied grant must precede credential lookup")
        }
    }
    let f = Fixture::new(
        r#"import fs from 'node:fs';import {complete} from '@ag1/ai';export default api=>{
        api.registerProvider('fixture',{api:'custom-api',baseUrl:'https://fixture.invalid',apiKey:'fixture',models:[{id:'model',name:'Fixture',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:4096,maxTokens:256}],streamSimple:()=>{fs.writeFileSync('./forbidden-hook','yes');throw new Error('hook must not run');}});
        api.registerCommand('infer',{handler:async()=>complete({provider:'fixture',id:'model',baseUrl:'https://forged.invalid'},{messages:[{role:'user',content:'fixture',timestamp:0}]},{temperature:0})});
    };"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut registry = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registry.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registry,
    )
    .unwrap();
    for variant in 0..5 {
        let configs = if variant == 4 {
            vec![]
        } else {
            vec![extension_custom_models::Config {
                provider: "fixture".into(),
                model: "model".into(),
                endpoint: "custom-endpoint".into(),
                callers: [if variant == 0 {
                    "/foreign/caller.ts".into()
                } else {
                    f.entry.path.clone()
                }]
                .into(),
                hook_owners: [if variant == 1 {
                    "/foreign/hook.ts".into()
                } else {
                    f.entry.path.clone()
                }]
                .into(),
                origins: [if variant == 2 {
                    "https://foreign.invalid".into()
                } else {
                    "https://fixture.invalid".into()
                }]
                .into(),
                options: json!({}),
                allowed_options: if variant == 3 {
                    Default::default()
                } else {
                    ["temperature".into()].into()
                },
                max_output_bytes: 65536,
                overall_ms: 1000,
            }]
        };
        let models = CustomModels::new(
            catalog.clone(),
            configs,
            NoFallback,
            NoCredentials,
            Arc::new(TestClock),
        )
        .unwrap();
        let scheduler = Scheduler::default();
        scheduler
            .register("custom-endpoint", EndpointKind::Local)
            .unwrap();
        let mut bridge = InferenceBridge::new(
            scheduler.clone(),
            ProviderHttp::new(&["https://fixture.invalid".into()], Duration::from_secs(1)).unwrap(),
            models,
            DenyBridge,
        )
        .unwrap();
        let result = host.call(
            &f.scope,
            "command",
            &json!({"name":"infer","arguments":""}),
            &control(&cancel),
            &mut bridge,
        );
        assert!(
            result.is_err(),
            "variant {variant} must require a separate native inference grant"
        );
        assert!(
            host.is_alive(),
            "ordinary denied requests must leave the helper available"
        );
        assert!(!f.dir.path().join("project/forbidden-hook").exists());
        let status = scheduler.status("custom-endpoint").unwrap();
        assert_eq!(
            status.active + status.waiting,
            0,
            "denial must precede endpoint admission"
        );
    }
    host.stop().unwrap();
}

#[test]
fn nested_custom_provider_transport_inherits_real_permit_and_latches_interrupted_children() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        extension_custom_models::{self, CustomModels},
        extension_providers::{Grant, NoOAuth, RegistryBridge},
    };
    struct NoCustomCredentials;
    impl extension_custom_models::Credentials for NoCustomCredentials {
        fn options(
            &mut self,
            _: &str,
            _: &Value,
            options: Value,
            _: &ScanControl<'_>,
        ) -> Result<Value, DomainError> {
            Ok(options)
        }
    }
    for interrupted in [false, true] {
        let (origin, hits, server) = provider_fixture(interrupted);
        let f = Fixture::new(
            r#"import fs from 'node:fs';import {complete} from '@ag1/ai';export default api=>{
            api.registerProvider('wrapper',{api:'wrapper-api',baseUrl:'https://wrapper.invalid',apiKey:'fixture',models:[{id:'wrapped',name:'Wrapped',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:4096,maxTokens:256}],streamSimple:(model,context)=>{
                let message; return {async *[Symbol.asyncIterator](){
                    try {message=await complete({provider:'fixture',id:'fixture-model'},context);}
                    catch(error){fs.writeFileSync('./caught-child','yes');message={role:'assistant',content:[{type:'text',text:'caught'}],usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:1};}
                    message={...message,provider:model.provider,model:model.id,api:model.api};
                    fs.writeFileSync('./child-returned',message.content[0].text);
                    for(let i=0;!fs.existsSync('./release-custom')&&i<200;i++)await new Promise(r=>setTimeout(r,5));
                    if(!fs.existsSync('./release-custom'))throw new Error('native sibling check did not finish');
                    yield {type:'done',reason:message.stopReason,message};
                },async result(){return message;}};
            }});
            api.registerCommand('infer',{handler:async()=>complete({provider:'wrapper',id:'wrapped'},{messages:[{role:'user',content:'nested fixture request',timestamp:0}]})});
        };"#,
        );
        let cancel = AtomicBool::new(false);
        let mut host = f
            .service
            .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
            .unwrap();
        let mut registry = RegistryBridge::new(
            &host,
            Snapshot::default(),
            vec![Grant {
                extension: f.entry.path.clone(),
                providers: ["wrapper".into()].into(),
            }],
            NoOAuth,
            DenyBridge,
        )
        .unwrap();
        let catalog = registry.catalog();
        host.call(
            &f.scope,
            "bind",
            &json!({"snapshot":{}}),
            &control(&cancel),
            &mut registry,
        )
        .unwrap();
        let models = CustomModels::new(
            catalog,
            vec![extension_custom_models::Config {
                provider: "wrapper".into(),
                model: "wrapped".into(),
                endpoint: origin.clone(),
                callers: [f.entry.path.clone()].into(),
                hook_owners: [f.entry.path.clone()].into(),
                origins: ["https://wrapper.invalid".into()].into(),
                options: json!({}),
                allowed_options: Default::default(),
                max_output_bytes: 65536,
                overall_ms: 2000,
            }],
            fixture_models(&f, &origin),
            NoCustomCredentials,
            Arc::new(TestClock),
        )
        .unwrap();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let mut bridge = InferenceBridge::new(
            scheduler.clone(),
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap(),
            models,
            registry,
        )
        .unwrap();
        let observed_scheduler = scheduler.clone();
        let observed_origin = origin.clone();
        let project = f.dir.path().join("project");
        let sibling = thread::spawn(move || {
            let started = Instant::now();
            while !project.join("child-returned").exists() {
                assert!(
                    started.elapsed() < Duration::from_secs(3),
                    "custom provider never resumed"
                );
                thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                observed_scheduler.status(&observed_origin).unwrap().active,
                1,
                "nested transport must not release the custom root's permit"
            );
            let ticket = observed_scheduler
                .submit(
                    &observed_origin,
                    OperationId::new("custom-sibling").unwrap(),
                )
                .unwrap();
            assert!(observed_scheduler.start(&ticket).unwrap().is_none());
            fs::write(project.join("release-custom"), "yes").unwrap();
            ticket
        });
        let output = host.call(
            &f.scope,
            "command",
            &json!({"name":"infer","arguments":""}),
            &control(&cancel),
            &mut bridge,
        );
        let request = server.join().unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "nested transport must use the real endpoint slot"
        );
        assert!(request.contains("nested fixture request"));
        assert!(request.contains("authorization: Bearer owned-fixture-token"));
        let sibling = sibling.join().unwrap();
        let status = scheduler.status(&origin).unwrap();
        assert_eq!(
            status.waiting + status.active,
            if interrupted { 2 } else { 1 }
        );
        if interrupted {
            assert!(
                output.is_err(),
                "caught interrupted child must not settle its parent"
            );
            assert!(
                f.dir.path().join("project/caught-child").exists(),
                "provider must actually catch the transport error"
            );
            assert_eq!(
                fs::read_to_string(f.dir.path().join("project/child-returned")).unwrap(),
                "caught"
            );
            assert_eq!(status.quarantined, 1);
            assert!(scheduler.start(&sibling).unwrap().is_none());
            scheduler.cancel(&sibling).unwrap();
        } else {
            let output = output.unwrap().unwrap();
            assert_eq!(output["content"][0]["text"], "hello world");
            assert_eq!(output["provider"], "wrapper");
            scheduler
                .start(&sibling)
                .unwrap()
                .unwrap()
                .terminal()
                .unwrap();
            assert_eq!(status.quarantined, 0);
        }
        host.stop().unwrap();
    }
}

#[test]
fn native_trust_admission_and_epoch_fences_precede_any_extension_process() {
    let f = Fixture::new("export default ()=>{};");
    let cancel = AtomicBool::new(false);
    let prepared = f.prepare(false, &cancel);
    assert!(prepared.selection().allowed.is_empty());
    assert!(prepared.selection().pending.is_some());
    assert!(matches!(
        f.service.start(prepared, &f.scope, &control(&cancel)),
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(
        fs::read_dir(f.dir.path().join("spools")).unwrap().count(),
        0
    );
    let prepared = f.prepare(true, &cancel);
    let mut stale = f.scope.clone();
    stale.epoch += 1;
    assert!(matches!(
        f.service.start(prepared, &stale, &control(&cancel)),
        Err(DomainError::Conflict)
    ));
    let prepared = f.prepare(true, &cancel);
    fs::write(
        &f.entry.path,
        "export default ()=>{throw new Error('changed')};",
    )
    .unwrap();
    assert!(matches!(
        f.service.start(prepared, &f.scope, &control(&cancel)),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        fs::read_dir(f.dir.path().join("spools")).unwrap().count(),
        0
    );
    fs::write(&f.entry.path, &f.code).unwrap();
}
#[test]
fn native_owned_host_routes_callbacks_and_blocks_preparation_side_effects() {
    let f = Fixture::new(
        "import {Type} from '@sinclair/typebox';export default api=>{console.log('owned fixture log');api.registerTool({name:'test',parameters:Type.Object({n:Type.Number()}),description:'fixture',prepareArguments:a=>{if(a.write)api.appendEntry('cannot-before-policy',{});return {n:Number(a.n)};},execute:async(id,args,signal,update)=>{update({content:[{type:'text',text:'progress'}]});return {content:[{type:'text',text:String(args.n)}],details:{signal:!!signal}};}});api.registerCommand('ask',{handler:async(args,ctx)=>{api.appendEntry('custom.record',{unknown:[1,2]});return await ctx.ui.select('Choose',['one','two']);}});api.registerCommand('fail',{handler:async()=>{throw new Error('isolated command error')}});};",
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut bridge = FixtureBridge::default();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{"hasUI":true}}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    let prepared = host
        .call(
            &f.scope,
            "tool.prepare",
            &json!({"name":"test","arguments":{"n":"7"}}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    assert_eq!(prepared, json!({"n":7}));
    let result = host
        .call(
            &f.scope,
            "tool.execute",
            &json!({"name":"test","callId":"call","arguments":prepared}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    assert_eq!(result["content"][0]["text"], "7");
    assert_eq!(bridge.events.last().unwrap()["type"], "tool_update");
    assert_eq!(
        host.call(
            &f.scope,
            "command",
            &json!({"name":"ask","arguments":""}),
            &control(&cancel),
            &mut bridge
        )
        .unwrap(),
        Some(json!("two"))
    );
    assert_eq!(bridge.requests.len(), 2);
    assert_eq!(bridge.requests[0]["method"], "session.appendEntry");
    assert_eq!(bridge.requests[0]["scope"]["epoch"], 1);
    let count = bridge.requests.len();
    assert!(
        host.call(
            &f.scope,
            "tool.prepare",
            &json!({"name":"test","arguments":{"n":2,"write":true}}),
            &control(&cancel),
            &mut bridge
        )
        .is_err()
    );
    assert_eq!(bridge.requests.len(), count);
    assert!(host.is_alive());
    assert!(
        host.call(
            &f.scope,
            "command",
            &json!({"name":"fail","arguments":""}),
            &control(&cancel),
            &mut bridge
        )
        .is_err()
    );
    assert!(host.is_alive());
    assert!(
        host.call(
            &f.scope,
            "catalog",
            &json!({}),
            &control(&cancel),
            &mut bridge
        )
        .is_ok()
    );
    assert!(
        String::from_utf8(host.diagnostics().unwrap())
            .unwrap()
            .contains("owned fixture log")
    );
    let pid = host.process_id();
    host.stop().unwrap();
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}
#[test]
fn deadline_host_death_and_cancellation_close_only_owned_hosts_and_reload_changes_scope() {
    let f = Fixture::new(
        "export default api=>{api.registerCommand('hang',{handler:async()=>{while(true){}}});api.registerCommand('die',{handler:async()=>process.exit(19)});api.registerTool({name:'wait',description:'fixture',parameters:{type:'object'},execute:async(id,args,signal,update)=>{update({content:[{type:'text',text:'ready'}]});return await new Promise(()=>{});}});};",
    );
    let cancel = AtomicBool::new(false);
    let start = || {
        let mut h = f
            .service
            .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
            .unwrap();
        h.call(
            &f.scope,
            "bind",
            &json!({"snapshot":{}}),
            &control(&cancel),
            &mut DenyBridge,
        )
        .unwrap();
        h
    };
    let mut h = start();
    let pid = h.process_id();
    let short = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_millis(150),
    };
    assert!(matches!(
        h.call(
            &f.scope,
            "command",
            &json!({"name":"hang","arguments":""}),
            &short,
            &mut DenyBridge
        ),
        Err(DomainError::TimedOut)
    ));
    assert!(!h.is_alive());
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let mut h = start();
    assert!(matches!(
        h.call(
            &f.scope,
            "command",
            &json!({"name":"die","arguments":""}),
            &control(&cancel),
            &mut DenyBridge
        ),
        Err(DomainError::Unavailable(_))
    ));
    assert!(!h.is_alive());
    struct CancelBridge<'a>(&'a AtomicBool);
    impl Bridge for CancelBridge<'_> {
        fn request(
            &mut self,
            _: NativeRequest<'_>,
            _: &ScanControl<'_>,
        ) -> Result<Option<Value>, DomainError> {
            Err(DomainError::PermissionDenied)
        }
        fn event(&mut self, _: &Scope, _: &str, _: &Value) -> Result<(), DomainError> {
            self.0.store(true, Ordering::Release);
            Ok(())
        }
    }
    let mut h = start();
    assert!(matches!(
        h.call(
            &f.scope,
            "tool.execute",
            &json!({"name":"wait","callId":"wait","arguments":{}}),
            &control(&cancel),
            &mut CancelBridge(&cancel)
        ),
        Err(DomainError::Cancelled)
    ));
    assert!(!h.is_alive());
    cancel.store(false, Ordering::Release);
    let mut fresh = f.scope.clone();
    fresh.epoch += 1;
    let config = f.dir.path().join("project/.ag1");
    let grant = f
        .service
        .prepare(
            &fresh,
            std::slice::from_ref(&f.entry),
            trust::Policy {
                project_config_dir: config.to_str().unwrap(),
                canonical_project_dir: config.to_str().unwrap(),
                explicit: Some(true),
                environment: None,
                user_trusted_projects: &[],
            },
            &control(&cancel),
        )
        .unwrap();
    let mut h = f.service.start(grant, &fresh, &control(&cancel)).unwrap();
    assert!(matches!(
        h.call(
            &f.scope,
            "catalog",
            &json!({}),
            &control(&cancel),
            &mut DenyBridge
        ),
        Err(DomainError::Conflict)
    ));
    assert!(h.is_alive());
}

use ag1_core::{
    Clock, SessionId,
    context::features::Features,
    tools::{
        approval::{AuthorizedCall, Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{self, Effect, Status},
        policy::Policy,
        registry::Registry,
        result::ResultBudget,
    },
};
use ag1_platform::extension_tools::{ExtensionTools, PreparedExtensionTool};
use ag1_storage::{Store, context::ContextStore};
const NOW: &str = "2026-09-05T00:00:00.000Z";
struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        12
    }
}
struct DurableFixture {
    f: Fixture,
    tools: ExtensionTools,
    registry: Registry,
    store: Store,
}
impl DurableFixture {
    fn new(code: &str, cancel: &AtomicBool) -> Self {
        Self::from_fixture(Fixture::new(code), cancel)
    }
    fn from_fixture(f: Fixture, cancel: &AtomicBool) -> Self {
        let host = f
            .service
            .start(f.prepare(true, cancel), &f.scope, &control(cancel))
            .unwrap();
        let tools = ExtensionTools::bind(
            host,
            &f.scope,
            &json!({}),
            &control(cancel),
            &mut DenyBridge,
        )
        .unwrap();
        let mut registry = Registry::builtins().unwrap();
        tools.register(&mut registry).unwrap();
        registry
            .activate(&["fixture".into()], &["fixture".into()].into())
            .unwrap();
        let store = Store::open(&f.dir.path().join("store")).unwrap();
        store.create_session("session",json!({"type":"session","version":3,"id":"session","cwd":f.dir.path().join("project"),"timestamp":NOW}).to_string().as_bytes()).unwrap();
        store
            .register_origin("origin", "session", &[4; 32])
            .unwrap();
        Self {
            f,
            tools,
            registry,
            store,
        }
    }
    fn prepare(
        &self,
        operation: &str,
        args: &Value,
        cancel: &AtomicBool,
    ) -> Result<PreparedExtensionTool, DomainError> {
        self.tools.prepare(
            &self.registry,
            &self.f.scope,
            executor::Invocation {
                operation,
                call_id: operation,
                name: "fixture",
                arguments: args,
                complete: true,
            },
            &json!({}),
            &control(cancel),
        )
    }
    fn ticket(&self, p: &PreparedExtensionTool) -> AuthorizedCall {
        let mut a = Authorizer::new("extension-fixture").unwrap();
        let policy:Policy=serde_json::from_value(json!({"mode":"interactive","cwd":self.f.dir.path().join("project"),"extensionTools":["fixture"]})).unwrap();
        let Preflight::Approval(request) = a
            .preflight(
                p.call().clone(),
                PreflightContext {
                    scope: &self.f.scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: p.target_revision(),
                    now: 10,
                    ttl: 1000,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        else {
            panic!("extension tool must request approval")
        };
        a.answer(
            &request.binding,
            true,
            &self.registry,
            ExecutionFence {
                scope: &self.f.scope,
                policy_revision: 1,
                target_revision: p.target_revision(),
                now: 11,
            },
        )
        .unwrap()
        .unwrap()
    }
    fn execute(
        &self,
        p: PreparedExtensionTool,
        ticket: AuthorizedCall,
        cancel: &AtomicBool,
        events: &mut Vec<executor::ToolEvent>,
    ) -> Result<executor::Outcome, DomainError> {
        let target = p.target_revision().to_owned();
        let operation = p.call().operation().to_owned();
        let mut artifacts = ContextStore::new(
            &self.store,
            SessionId::new("session").unwrap(),
            "origin",
            &operation,
            NOW,
            Features::default(),
        )
        .unwrap();
        self.tools.execute(
            p,
            ticket,
            executor::Execution {
                registry: &self.registry,
                fence: ExecutionFence {
                    scope: &self.f.scope,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 12,
                },
                budget: ResultBudget::default(),
                clock: &TestClock,
            },
            &self.store,
            &mut artifacts,
            &json!({}),
            &control(cancel),
            &mut DenyBridge,
            |e| events.push(e),
        )
    }
}
#[test]
fn extension_tools_require_native_validation_approval_and_durable_intent_then_replay_without_host()
{
    let cancel = AtomicBool::new(false);
    let f = DurableFixture::new(
        "import fs from 'node:fs';export default api=>{api.registerTool({name:'fixture',description:'fixture',parameters:{type:'object',properties:{n:{type:'number'}},required:['n'],additionalProperties:false},prepareArguments:a=>({n:Number(a.n)}),execute:async(id,args,signal,update,ctx)=>{fs.appendFileSync(ctx.cwd+'/executed','x');return {content:[{type:'text',text:'native '+args.n}],details:{native:7}};}});api.on('tool_call',e=>e.input.n===13?{block:true,reason:'blocked'}:undefined);api.on('tool_result',()=>({content:[{type:'text',text:'projected'}],details:{pretendNative:true}}));};",
        &cancel,
    );
    let marker = f.f.dir.path().join("project/executed");
    assert!(f.prepare("invalid", &json!({"n":"bad"}), &cancel).is_err());
    assert!(matches!(
        f.prepare("blocked", &json!({"n":13}), &cancel),
        Err(DomainError::PermissionDenied)
    ));
    let p = f.prepare("run", &json!({"n":"4"}), &cancel).unwrap();
    assert_eq!(p.call().args(), &json!({"n":4}));
    let replay = f.prepare("run", &json!({"n":"4"}), &cancel).unwrap();
    let ticket = f.ticket(&p);
    let replay_ticket = f.ticket(&replay);
    assert!(!marker.exists());
    assert!(
        executor::recover(&f.store, &f.f.scope.workspace, "session", "run")
            .unwrap()
            .is_none()
    );
    let mut events = vec![];
    let out = f.execute(p, ticket, &cancel, &mut events).unwrap();
    assert!(!out.replayed);
    assert_eq!(out.record.status, Status::Completed);
    assert_eq!(out.record.effect, Effect::Unknown);
    assert_eq!(out.result["content"][0]["text"], "projected");
    assert_eq!(
        out.result["details"]["extensionDetails"]["pretendNative"],
        true
    );
    assert_eq!(out.result["details"]["postHookOutput"], true);
    assert!(out.record.native_result.is_some());
    assert_eq!(
        events.iter().map(|e| e.stage.as_str()).collect::<Vec<_>>(),
        vec!["start", "end"]
    );
    assert_eq!(fs::read_to_string(&marker).unwrap(), "x");
    f.tools.stop().unwrap();
    let out = f
        .execute(replay, replay_ticket, &cancel, &mut events)
        .unwrap();
    assert!(out.replayed);
    assert_eq!(events.len(), 2);
    assert_eq!(fs::read_to_string(&marker).unwrap(), "x");
}
#[test]
fn extension_host_crash_after_effect_leaves_uncertain_intent_and_never_reexecutes() {
    let cancel = AtomicBool::new(false);
    let f = DurableFixture::new(
        "import fs from 'node:fs';export default api=>api.registerTool({name:'fixture',description:'fixture',parameters:{type:'object'},execute:async(id,args,signal,update,ctx)=>{fs.appendFileSync(ctx.cwd+'/executed','x');process.exit(17);}});",
        &cancel,
    );
    let p = f.prepare("crash", &json!({}), &cancel).unwrap();
    let replay = f.prepare("crash", &json!({}), &cancel).unwrap();
    let ticket = f.ticket(&p);
    let replay_ticket = f.ticket(&replay);
    let mut events = vec![];
    assert!(f.execute(p, ticket, &cancel, &mut events).is_err());
    let record = executor::recover(&f.store, &f.f.scope.workspace, "session", "crash")
        .unwrap()
        .unwrap();
    assert_eq!(record.status, Status::Intent);
    assert_eq!(record.effect, Effect::Unknown);
    assert!(matches!(
        f.execute(replay, replay_ticket, &cancel, &mut events),
        Err(DomainError::DurabilityUnknown)
    ));
    assert_eq!(events.len(), 1);
    assert_eq!(
        fs::read_to_string(f.f.dir.path().join("project/executed")).unwrap(),
        "x"
    );
}
#[test]
fn extension_registration_is_atomic_and_preparation_cannot_smuggle_parent_callbacks() {
    let mut registry = Registry::builtins().unwrap();
    let revision = registry.revision();
    assert!(
        registry
            .register_extensions(&[
                json!({"name":"newtool","description":"new","parameters":{"type":"object"}}),
                json!({"name":"invalid","description":"bad","parameters":{"type":"not-a-type"}})
            ])
            .is_err()
    );
    assert_eq!(registry.revision(), revision);
    assert!(
        !registry
            .definitions()
            .any(|d| d.metadata["name"] == "newtool")
    );
    let cancel = AtomicBool::new(false);
    let f = DurableFixture::new(
        "export default api=>api.registerTool({name:'fixture',description:'fixture',parameters:{type:'object'},prepareArguments:a=>{api.appendEntry('not-authorized',{});return a},execute:async()=>({content:[]})});",
        &cancel,
    );
    assert!(f.prepare("denied", &json!({}), &cancel).is_err());
    assert!(
        executor::recover(&f.store, &f.f.scope.workspace, "session", "denied")
            .unwrap()
            .is_none()
    );
}

use ag1_core::{
    OperationId, SecretBytes,
    inference::{EndpointKind, Scheduler},
};
use ag1_platform::extension_models::{ConfiguredModels, Credential, Credentials, ModelConfig};
use ag1_platform::{
    extension_inference::{InferenceBridge, InferenceInput, ModelResolver},
    provider::{generation::GenerationLimits, http::ProviderHttp},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::AtomicUsize,
    thread,
};
struct FixtureCredentials;
impl Credentials for FixtureCredentials {
    fn resolve(&mut self, _: &Value, control: &ScanControl<'_>) -> Result<Credential, DomainError> {
        control.check()?;
        Ok(Credential::ApiKey(SecretBytes::new(
            b"owned-fixture-token".to_vec(),
        )))
    }
}
fn fixture_models(f: &Fixture, origin: &str) -> ConfiguredModels<FixtureCredentials> {
    ConfiguredModels::new(f.scope.clone(),[f.entry.path.clone()].into(),vec![ModelConfig{
  model:json!({"id":"fixture-model","provider":"fixture","api":"openai-completions","name":"Owned fixture","baseUrl":format!("{origin}/v1"),"reasoning":false,"input":["text"],"contextWindow":4096,"maxTokens":256,"cost":{"input":1,"output":2,"cacheRead":0,"cacheWrite":0}}),
  endpoint:origin.into(),options:json!({}),allowed_options:["temperature".into(),"maxTokens".into()].into(),headers:None,
  limits:GenerationLimits{max_output_bytes:512*1024,prefill_ms:0,idle_ms:0,overall_ms:5000},max_response_bytes:1024*1024,
 }],FixtureCredentials,Arc::new(TestClock)).unwrap()
}

fn provider_fixture(stall: bool) -> (String, Arc<AtomicUsize>, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut socket: TcpStream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(4),
                        "fixture never contacted"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = vec![];
        let mut chunk = [0; 4096];
        loop {
            let n = socket.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            assert!(bytes.len() < 65536);
            if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..index]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= index + 4 + length {
                    break;
                }
            }
        }
        count.fetch_add(1, Ordering::SeqCst);
        if stall {
            thread::sleep(Duration::from_millis(250));
            return String::from_utf8(bytes).unwrap();
        }
        let body = "data: {\"id\":\"owned-response\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"owned-response\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\ndata: [DONE]\n\n";
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (origin, hits, worker)
}
#[test]
fn extension_stream_uses_shared_native_endpoint_admission_credentials_and_real_http() {
    let cancel = AtomicBool::new(false);
    let f = Fixture::new(
        "import {streamSimple} from '@ag1/ai';export default api=>api.registerCommand('infer',{handler:async()=>{const stream=streamSimple({provider:'fixture',id:'fixture-model',baseUrl:'https://forged.invalid',apiKey:'forged'},{messages:[{role:'user',content:'fixture request',timestamp:0}]},{temperature:0});const events=[];for await(const event of stream)events.push(event.type);return {events,message:await stream.result()};}});",
    );
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut DenyBridge,
    )
    .unwrap();
    let (origin, hits, server) = provider_fixture(false);
    let scheduler = Scheduler::default();
    scheduler.register(&origin, EndpointKind::Local).unwrap();
    let root = scheduler
        .submit(&origin, OperationId::new("root-generation").unwrap())
        .unwrap();
    let permit = scheduler.start(&root).unwrap().unwrap();
    let observed = hits.clone();
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        assert_eq!(
            observed.load(Ordering::SeqCst),
            0,
            "extension bypassed shared local queue"
        );
        permit.terminal().unwrap();
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    let mut bridge = InferenceBridge::new(
        scheduler.clone(),
        http,
        fixture_models(&f, &origin),
        DenyBridge,
    )
    .unwrap();
    let output = host
        .call(
            &f.scope,
            "command",
            &json!({"name":"infer","arguments":""}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    release.join().unwrap();
    let request = server.join().unwrap();
    assert!(request.contains("authorization: Bearer owned-fixture-token"));
    assert!(!request.contains("forged"));
    assert!(request.contains("fixture request"));
    assert_eq!(output["message"]["content"][0]["text"], "hello world");
    assert_eq!(output["message"]["usage"]["totalTokens"], 5);
    assert_eq!(output["message"]["usage"]["cost"]["total"], 0.000007);
    let events = output["events"].as_array().unwrap();
    assert_eq!(events.first().unwrap(), "start");
    assert_eq!(events.last().unwrap(), "done");
    assert!(events.contains(&json!("text_delta")));
    assert_eq!(scheduler.status(&origin).unwrap().active, 0);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}
#[test]
fn extension_signal_and_helper_death_cancel_stalled_native_generation_and_quarantine() {
    for death in [false, true] {
        let cancel = AtomicBool::new(false);
        let action = if death {
            "setTimeout(()=>process.exit(17),100);"
        } else {
            "setTimeout(()=>controller.abort(),100);"
        };
        let f = Fixture::new(&format!(
            "import {{streamSimple}} from '@ag1/ai';export default api=>api.registerCommand('infer',{{handler:async()=>{{const controller=new AbortController();{action}const stream=streamSimple({{provider:'fixture',id:'fixture-model'}},{{messages:[{{role:'user',content:'fixture',timestamp:0}}]}},{{signal:controller.signal}});for await(const event of stream){{}}return await stream.result();}}}});"
        ));
        let mut host = f
            .service
            .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
            .unwrap();
        host.call(
            &f.scope,
            "bind",
            &json!({"snapshot":{}}),
            &control(&cancel),
            &mut DenyBridge,
        )
        .unwrap();
        let (origin, hits, server) = provider_fixture(true);
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let http =
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
        let mut bridge = InferenceBridge::new(
            scheduler.clone(),
            http,
            fixture_models(&f, &origin),
            DenyBridge,
        )
        .unwrap();
        let started = Instant::now();
        assert!(
            host.call(
                &f.scope,
                "command",
                &json!({"name":"infer","arguments":""}),
                &control(&cancel),
                &mut bridge
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(scheduler.status(&origin).unwrap().quarantined, 1);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        server.join().unwrap();
    }
}

#[test]
fn configured_extension_model_grants_reject_foreign_scope_owner_model_and_options() {
    let f = Fixture::new("export default ()=>{};");
    let mut models = fixture_models(&f, "http://127.0.0.1:1");
    let cancel = AtomicBool::new(false);
    let context = json!({"messages":[{"role":"user","content":"fixture","timestamp":0}]});
    let options = json!({});
    for variant in 0..5 {
        let mut scope = f.scope.clone();
        if variant == 0 {
            scope.epoch += 1;
        }
        let forbidden = json!({"apiKey":"cannot-override"});
        let input = InferenceInput {
            scope: &scope,
            extension: if variant == 1 {
                "/foreign/extension.ts"
            } else {
                &f.entry.path
            },
            operation: "fixture-request",
            host_operation: "fixture-host-operation",
            provider: if variant == 2 { "foreign" } else { "fixture" },
            model: if variant == 3 {
                "unknown"
            } else {
                "fixture-model"
            },
            context: &context,
            options: if variant == 4 { &forbidden } else { &options },
        };
        assert!(matches!(
            models.resolve(input, &control(&cancel)),
            Err(DomainError::PermissionDenied)
        ));
    }
    let resolved = models
        .resolve(
            InferenceInput {
                scope: &f.scope,
                extension: &f.entry.path,
                operation: "fixture-request",
                host_operation: "fixture-host-operation",
                provider: "fixture",
                model: "fixture-model",
                context: &context,
                options: &json!({"temperature":0}),
            },
            &control(&cancel),
        )
        .unwrap();
    assert_eq!(
        resolved.generation.request.url,
        "http://127.0.0.1:1/v1/chat/completions"
    );
    assert_eq!(resolved.generation.limits.prefill_ms, 0);
    assert_eq!(resolved.generation.limits.idle_ms, 0);
    let body: Value = serde_json::from_slice(&resolved.generation.request.body).unwrap();
    assert_eq!(body["temperature"], 0);
    assert_eq!(body["model"], "fixture-model");
}

#[test]
fn dynamic_extension_refresh_updates_native_tools_and_invalidates_same_schema_replacements() {
    let cancel = AtomicBool::new(false);
    let mut fixture = Fixture::new(
        "export default api=>{const definition={name:'fixture',description:'fixture',parameters:{type:'object'},execute:async()=>({content:[{type:'text',text:'first'}]})};api.registerTool(definition);api.registerCommand('replace',{handler:async()=>api.registerTool({...definition,execute:async()=>({content:[{type:'text',text:'replacement'}]})})});api.on('session_start',()=>{api.registerTool({name:'dynamic',description:'Dynamic description',promptSnippet:'Dynamic snippet',promptGuidelines:['Dynamic guidance'],parameters:{type:'object'},execute:async()=>({content:[{type:'text',text:'dynamic'}]})});api.registerTool({name:'hidden_tool',description:'Description should appear in available tools',parameters:{type:'object'},execute:async()=>({content:[{type:'text',text:'hidden'}]})});});};",
    );
    // The host materializes an inline factory into an owned, hash-checked entry;
    // display provenance remains distinct from the actual execution identity.
    fixture.entry.source_info = SourceInfo {
        path: "<inline:1>".into(),
        source: "inline".into(),
        scope: "temporary".into(),
        origin: "top-level".into(),
        base_dir: None,
    };
    let mut f = DurableFixture::from_fixture(fixture, &cancel);
    assert!(
        !f.registry
            .definitions()
            .any(|d| d.metadata["name"] == "dynamic")
    );
    assert!(!f.registry.active_names().contains(&"hidden_tool".into()));
    let p = f.prepare("stale", &json!({}), &cancel).unwrap();
    let ticket = f.ticket(&p);
    let old = f.registry.revision();
    let allowed: std::collections::BTreeSet<String> =
        ["fixture".into(), "dynamic".into(), "hidden_tool".into()].into();
    f.tools
        .dispatch(
            &mut f.registry,
            "command",
            &json!({"name":"replace","arguments":""}),
            &control(&cancel),
            &mut DenyBridge,
            &mut |_| Ok(allowed.clone()),
            true,
        )
        .unwrap();
    assert!(f.registry.revision() > old);
    assert!(matches!(
        f.execute(p, ticket, &cancel, &mut vec![]),
        Err(DomainError::Conflict)
    ));
    let p = f.prepare("replacement", &json!({}), &cancel).unwrap();
    let ticket = f.ticket(&p);
    let out = f.execute(p, ticket, &cancel, &mut vec![]).unwrap();
    assert_eq!(out.result["content"][0]["text"], "replacement");
    f.tools
        .dispatch(
            &mut f.registry,
            "event",
            &json!({"event":{"type":"session_start"}}),
            &control(&cancel),
            &mut DenyBridge,
            &mut |_| Ok(allowed.clone()),
            true,
        )
        .unwrap();
    assert!(f.registry.active_names().contains(&"dynamic".into()));
    let definition = f
        .registry
        .definitions()
        .find(|d| d.metadata["name"] == "dynamic")
        .unwrap();
    assert_eq!(definition.metadata["promptSnippet"], "Dynamic snippet");
    assert_eq!(
        definition.metadata["promptGuidelines"],
        json!(["Dynamic guidance"])
    );
    assert_eq!(
        definition.metadata["sourceInfo"],
        json!({"path":"<inline:1>","source":"inline","scope":"temporary","origin":"top-level"})
    );
    assert_ne!(f.f.entry.path, "<inline:1>");
    let public = f.registry.public_tools();
    assert_eq!(
        public.iter().find(|d| d["name"] == "dynamic").unwrap()["sourceInfo"],
        definition.metadata["sourceInfo"]
    );
    let prompt = f
        .registry
        .build_prompt(&ag1_core::context::prompt::PromptInput {
            date: "2026-09-05".into(),
            cwd: f.f.dir.path().join("project").to_str().unwrap().into(),
            ..Default::default()
        })
        .unwrap();
    assert!(prompt.contains("- dynamic: Dynamic snippet"));
    assert!(prompt.contains("- Dynamic guidance"));
    assert!(prompt.contains("- hidden_tool: Description should appear in available tools"));
    assert!(f.registry.active_names().contains(&"hidden_tool".into()));
}

#[test]
fn extension_resource_hooks_publish_native_data_with_owned_provenance_and_atomic_failures() {
    use ag1_core::resources::catalog::Kind;
    use ag1_platform::resources::{Config, DiscoveryReason, Paths, ResourceLoader};
    let f = Fixture::new(
        r#"export default api=>{
        let mode='valid', events=[];
        api.registerCommand('mode',{handler:arg=>{mode=arg;return events}});
        api.on('resources_discover',()=>{throw new Error('isolated resource handler')});
        api.on('resources_discover',event=>{
            events.push({cwd:event.cwd,reason:event.reason});
            if(mode==='empty')return {};
            if(mode==='malformed')return {promptPaths:[{path:'../contrib/prompts',extensionPath:'/forged.ts'}]};
            if(mode==='many')return {promptPaths:Array.from({length:257},(_,i)=>'../contrib/prompts/'+i)};
            if(mode.startsWith('outside:'))return {promptPaths:[mode.slice(8)]};
            if(mode==='mutate')return {extensionPaths:['../contrib/execute.ts']};
            return {skillPaths:['../contrib/skill'],promptPaths:['../contrib/prompts'],themePaths:['../contrib/theme.json']};
        });
    }"#,
    );
    let base = f.dir.path();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join("ungranted.md"),
        "Outside the admitted resource roots.",
    )
    .unwrap();
    let outside_mode = format!("outside:{}", outside.path().join("ungranted.md").display());
    fs::create_dir_all(base.join("contrib/skill")).unwrap();
    fs::create_dir_all(base.join("contrib/prompts")).unwrap();
    fs::create_dir_all(base.join("agent")).unwrap();
    fs::create_dir_all(base.join("home")).unwrap();
    fs::write(
        base.join("contrib/skill/SKILL.md"),
        "---\nname: contributed\ndescription: Contributed skill.\n---\nSkill instructions.",
    )
    .unwrap();
    fs::write(base.join("contrib/prompts/extra.md"), "Contributed prompt.").unwrap();
    fs::write(
        base.join("contrib/execute.ts"),
        "throw new Error('must never execute')",
    )
    .unwrap();
    let theme: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/theme-oracle.json")).unwrap();
    fs::write(
        base.join("contrib/theme.json"),
        theme["rows"][0]["input"].to_string(),
    )
    .unwrap();
    let make_loader = |cwd| {
        ResourceLoader::new(
            ResourceFs::new(vec![
                WorkspaceService::new().unwrap().register(base).unwrap(),
            ])
            .unwrap(),
            Config {
                cwd,
                agent_dir: base.join("agent"),
                home: base.join("home"),
                additional: Paths::default(),
                disabled: vec![Kind::Extension],
                installed: vec![],
                system: None,
                append_system: None,
            },
        )
        .unwrap()
    };
    let mut loader = make_loader(base.join("project"));
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut DenyBridge,
    )
    .unwrap();
    let discover = |loader: &mut ResourceLoader,
                    host: &mut extension_host::ExtensionHost,
                    revision,
                    scope: &Scope,
                    reason| {
        loader.discover_extensions(
            revision,
            host,
            scope,
            reason,
            &control(&cancel),
            &mut DenyBridge,
        )
    };
    assert!(matches!(
        discover(
            &mut loader,
            &mut host,
            0,
            &f.scope,
            DiscoveryReason::Startup
        ),
        Err(DomainError::Unavailable(_))
    ));
    loader.reload(0, &control(&cancel)).unwrap();
    let mut stale = f.scope.clone();
    stale.epoch += 1;
    assert!(matches!(
        discover(&mut loader, &mut host, 1, &stale, DiscoveryReason::Startup),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        discover(
            &mut loader,
            &mut host,
            1,
            &f.scope,
            DiscoveryReason::Startup
        )
        .unwrap(),
        2
    );
    let snapshot = loader.snapshot().unwrap();
    assert_eq!(snapshot.catalog.skills.len(), 1);
    assert_eq!(snapshot.catalog.prompts.len(), 1);
    assert_eq!(snapshot.catalog.themes.len(), 1);
    assert!(snapshot.catalog.extensions.is_empty());
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/extension-resource-paths-oracle.json"
    ))
    .unwrap();
    for (key, source) in [
        (
            "skillPaths",
            serde_json::to_value(&snapshot.catalog.skills[0].source_info).unwrap(),
        ),
        (
            "promptPaths",
            serde_json::to_value(&snapshot.catalog.prompts[0].source_info).unwrap(),
        ),
        (
            "themePaths",
            serde_json::to_value(&snapshot.catalog.themes[0].source_info).unwrap(),
        ),
    ] {
        let source: Value = serde_json::from_str(
            &source
                .to_string()
                .replace(base.to_str().unwrap(), "/fixture"),
        )
        .unwrap();
        for (field, value) in oracle["output"][key][0]["metadata"].as_object().unwrap() {
            assert_eq!(&source[field], value, "{key} {field}");
        }
    }
    let fingerprint = snapshot.fingerprint.clone();
    let set_mode = |host: &mut extension_host::ExtensionHost, mode| {
        host.call(
            &f.scope,
            "command",
            &json!({"name":"mode","arguments":mode}),
            &control(&cancel),
            &mut DenyBridge,
        )
        .unwrap()
        .unwrap()
    };
    let events = set_mode(&mut host, "malformed");
    assert_eq!(
        events,
        json!([{"cwd":base.join("project"),"reason":"startup"}])
    );
    for (mode, expected) in [
        ("malformed", "invalid"),
        (outside_mode.as_str(), "permission"),
    ] {
        set_mode(&mut host, mode);
        let error =
            discover(&mut loader, &mut host, 2, &f.scope, DiscoveryReason::Reload).unwrap_err();
        assert!(
            matches!(
                (&error, expected),
                (DomainError::InvalidInput(_), "invalid")
                    | (DomainError::LimitExceeded(_), "limit")
                    | (DomainError::PermissionDenied, "permission")
            ),
            "{mode}: {error:?}"
        );
        assert_eq!(loader.generation(), 2);
        assert_eq!(loader.snapshot().unwrap().fingerprint, fingerprint);
    }
    for mode in ["empty", "mutate", "many"] {
        set_mode(&mut host, mode);
        assert_eq!(
            discover(&mut loader, &mut host, 2, &f.scope, DiscoveryReason::Reload).unwrap(),
            2
        );
        assert_eq!(loader.snapshot().unwrap().fingerprint, fingerprint);
    }
    let mut foreign = make_loader(base.join("contrib"));
    foreign.reload(0, &control(&cancel)).unwrap();
    assert!(matches!(
        discover(
            &mut foreign,
            &mut host,
            1,
            &f.scope,
            DiscoveryReason::Reload
        ),
        Err(DomainError::PermissionDenied)
    ));
    set_mode(&mut host, "valid");
    fs::write(base.join("contrib/prompts/extra.md"), "Updated prompt.").unwrap();
    assert!(matches!(
        discover(&mut loader, &mut host, 1, &f.scope, DiscoveryReason::Reload),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        discover(&mut loader, &mut host, 2, &f.scope, DiscoveryReason::Reload).unwrap(),
        3
    );
    assert_eq!(
        loader.snapshot().unwrap().catalog.prompts[0].content,
        "Updated prompt."
    );
    assert!(host.is_alive());
    host.stop().unwrap();
}

#[test]
fn prebind_provider_removal_clears_all_queued_versions_and_invalid_registration_is_isolated() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::extension_providers::{Grant, NoOAuth, RegistryBridge};
    let f = Fixture::new(
        r#"export default api=>{
        api.registerProvider('queued-provider',{api:'openai-completions',baseUrl:'https://provider.test/v1',apiKey:'fixture',models:[{id:'one'}]});
        api.registerProvider('queued-provider',{api:'openai-completions',baseUrl:'https://provider.test/v1',apiKey:'fixture',models:[{id:'two'}]});
        api.unregisterProvider('queued-provider');
        api.registerProvider('broken-provider',{streamSimple:()=>{throw new Error('must not run')}});
        api.registerCommand('alive',{handler:()=> 'alive'});
    }"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut bridge = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["queued-provider".into(), "broken-provider".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = bridge.catalog();
    let result = host
        .call(
            &f.scope,
            "bind",
            &json!({"snapshot":{}}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    assert_eq!(catalog.generation().unwrap(), 0);
    assert!(catalog.models().unwrap().is_empty());
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
    assert_eq!(result["errors"][0]["extensionPath"], f.entry.path);
    let error = result["errors"][0]["error"].as_str().unwrap();
    assert!(
        error.contains("broken-provider")
            && error.contains("required when registering streamSimple"),
        "{error}"
    );
    assert!(!error.contains("must not run"));
    assert_eq!(
        host.call(
            &f.scope,
            "command",
            &json!({"name":"alive","arguments":""}),
            &control(&cancel),
            &mut bridge
        )
        .unwrap(),
        Some(json!("alive"))
    );
    host.stop().unwrap();
}

#[test]
fn postbind_provider_catalog_is_synchronous_and_native_rejections_preserve_prior_models() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::extension_providers::{Grant, OAuth, RegistryBridge};
    struct FixtureOAuth;
    impl OAuth for FixtureOAuth {
        fn credentials(
            &mut self,
            provider: &str,
            _: &str,
            control: &ScanControl<'_>,
        ) -> Result<Option<Value>, DomainError> {
            control.check()?;
            assert_eq!(provider, "instant-provider");
            Ok(Some(json!({"type":"oauth","access":"owned-fixture-token"})))
        }
    }
    let f = Fixture::new(
        r#"
        import {getModels} from '@ag1/ai';
        export default api => {
            const config = {api:'instant-api',apiKey:'owned-fixture-key',baseUrl:'https://fixture.invalid',models:[{
                id:'instant-model',name:'Instant Model',reasoning:false,input:['text'],
                cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:128000,maxTokens:4096
            }]};
            let calls=0;
            api.registerCommand('immediate',{handler:(_,ctx)=>{
                const absent = ctx.modelRegistry.find('instant-provider','instant-model') === undefined;
                api.registerProvider('instant-provider',config);
                const found = ctx.modelRegistry.find('instant-provider','instant-model');
                const all = getModels('instant-provider');
                api.unregisterProvider('instant-provider');
                return {absent,found,all,removed:ctx.modelRegistry.find('instant-provider','instant-model')===undefined};
            }});
            api.registerCommand('oauth',{handler:(_,ctx)=>{
                api.registerProvider('instant-provider',{...config,oauth:{name:'Fixture',modifyModels:(models,credentials)=>{
                    if(credentials.access!=='owned-fixture-token')throw new Error('wrong fixture credential');
                    return models.map(m=>({...m,name:m.name+' '+(++calls)}));
                }}});
                const before = ctx.modelRegistry.find('instant-provider','instant-model');
                let denied=false, rejected=false;
                try { api.registerProvider('ungranted',config); } catch { denied=true; }
                try { api.registerProvider('instant-provider',{...config,oauth:{name:'Rejected',modifyModels:()=>{throw new Error('fixture rejected')}}}); }
                catch { rejected=true; }
                return {before,after:ctx.modelRegistry.find('instant-provider','instant-model'),denied,rejected};
            }});
        };
    "#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut bridge = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: std::collections::BTreeSet::from(["instant-provider".into()]),
        }],
        FixtureOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = bridge.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    let result = host
        .call(
            &f.scope,
            "command",
            &json!({"name":"immediate","arguments":""}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    assert_eq!(result["absent"], true);
    assert_eq!(result["found"]["id"], "instant-model");
    assert_eq!(result["found"]["provider"], "instant-provider");
    assert_eq!(result["all"], json!([result["found"].clone()]));
    assert_eq!(result["removed"], true);
    assert!(catalog.models().unwrap().is_empty());
    assert_eq!(catalog.generation().unwrap(), 2);
    let result = host
        .call(
            &f.scope,
            "command",
            &json!({"name":"oauth","arguments":""}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap()
        .unwrap();
    assert_eq!(result["before"]["name"], "Instant Model 1");
    assert_eq!(result["before"], result["after"]);
    assert_eq!(result["denied"], true);
    assert_eq!(result["rejected"], true);
    assert_eq!(catalog.models().unwrap(), vec![result["before"].clone()]);
    assert_eq!(catalog.generation().unwrap(), 3);
    host.stop().unwrap();
}

#[test]
fn extension_oauth_login_refresh_and_key_hooks_use_the_native_vault_and_current_registration() {
    use ag1_core::{Clock, SecretBytes, resources::extension_providers::Snapshot};
    use ag1_platform::{
        executor::BlockingPool,
        extension_auth,
        extension_host::NativeStream,
        extension_providers::{Catalog, Grant, NoOAuth, OAuthTarget, RegistryBridge},
        provider::{auth::CredentialBackend, credential_storage::StorageCredentialBackend},
    };
    use std::sync::atomic::AtomicU64;
    struct TestClock(AtomicU64);
    impl Clock for TestClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    type Backend = StorageCredentialBackend<ag1_storage::Store>;
    struct AuthBridge {
        registry: RegistryBridge<NoOAuth, DenyBridge>,
        catalog: Catalog,
        auth: extension_auth::Service<Backend>,
        old: Option<OAuthTarget>,
    }
    impl Bridge for AuthBridge {
        fn request(
            &mut self,
            _: NativeRequest<'_>,
            _: &ScanControl<'_>,
        ) -> Result<Option<Value>, DomainError> {
            Err(DomainError::PermissionDenied)
        }
        fn request_stream(
            &mut self,
            r: NativeRequest<'_>,
            control: &ScanControl<'_>,
            stream: &mut dyn NativeStream,
        ) -> Result<Option<Value>, DomainError> {
            if matches!(r.method, "provider.register" | "provider.unregister") {
                return self.registry.request_stream(r, control, stream);
            }
            if r.phase != "command" {
                return Err(DomainError::PermissionDenied);
            }
            let target = self
                .catalog
                .oauth_target(r.scope, r.operation, "fixture")?
                .ok_or(DomainError::PermissionDenied)?;
            if r.extension != target.binding().extension {
                return Err(DomainError::PermissionDenied);
            }
            if r.method == "host.request" && r.arguments["method"] == "fixture.login" {
                self.auth.login(&target, control, stream, &mut DenyBridge)?;
                self.old = Some(target);
                return Ok(Some(json!(true)));
            }
            if r.method == "host.request" && r.arguments["method"] == "fixture.old" {
                return self
                    .auth
                    .key(
                        self.old.as_ref().unwrap(),
                        false,
                        control,
                        stream,
                        &mut DenyBridge,
                    )
                    .map(|_| Some(json!(true)));
            }
            if r.method != "auth.getApiKeyForProvider" || r.arguments["provider"] != "fixture" {
                return Err(DomainError::PermissionDenied);
            }
            let key = self
                .auth
                .key(&target, false, control, stream, &mut DenyBridge)?;
            Ok(key.map(|key| json!(std::str::from_utf8(key.expose()).unwrap())))
        }
    }
    let f = Fixture::new(
        r#"export default api=>{
        let login=0,refresh=0,keys=0,mode="ok";
        const oauth={name:'Fixture',login:async()=>{login++;if(mode==='bad-login')return {expires:'bad'};return {access:'fixture-initial',refresh:'fixture-refresh',expires:100,extra:{keep:true}}},
            refreshToken:async old=>{refresh++;if(mode==='throw-refresh')throw new Error('fixture-sensitive-secret');if(mode==='bad-refresh')return {expires:'bad'};if(old.refresh!=='fixture-refresh'||!old.extra.keep)throw new Error('bad fixture state');return {...old,access:'fixture-refreshed',expires:1000}},
            getApiKey:credentials=>{keys++;return credentials.access+'-key'}};
        api.registerProvider('fixture',{api:'fixture-api',oauth});
        api.registerCommand('login',{handler:(_,ctx)=>ctx.host.request('fixture.login',{})});
        api.registerCommand('key',{handler:(_,ctx)=>ctx.modelRegistry.getApiKeyForProvider('fixture')});
        api.registerCommand('old',{handler:(_,ctx)=>ctx.host.request('fixture.old',{})});
        api.registerCommand('replace',{handler:()=>api.registerProvider('fixture',{api:'fixture-api',oauth})});
        api.registerCommand('counts',{handler:()=>({login,refresh,keys})});api.registerCommand('mode',{handler:args=>{mode=args}});
    };"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let registry = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registry.catalog();
    let vault = tempfile::tempdir().unwrap();
    let backend = StorageCredentialBackend::new(
        ag1_storage::Store::open(vault.path()).unwrap(),
        Arc::new(BlockingPool::new(2, 16).unwrap()),
    );
    let clock = Arc::new(TestClock(AtomicU64::new(50)));
    let auth = extension_auth::Service::new(
        backend.clone(),
        vec![extension_auth::Grant {
            provider: "fixture".into(),
            hook_owners: [f.entry.path.clone()].into(),
        }],
        clock.clone(),
    )
    .unwrap();
    let mut bridge = AuthBridge {
        registry,
        catalog,
        auth,
        old: None,
    };
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{"hasHost":true}}),
        &control(&cancel),
        &mut bridge,
    )
    .unwrap();
    let invoke = |host: &mut extension_host::ExtensionHost, bridge: &mut AuthBridge, name: &str| {
        host.call(
            &f.scope,
            "command",
            &json!({"name":name,"arguments":""}),
            &control(&cancel),
            bridge,
        )
    };
    assert_eq!(invoke(&mut host, &mut bridge, "key").unwrap(), None);
    assert_eq!(
        invoke(&mut host, &mut bridge, "login").unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        invoke(&mut host, &mut bridge, "key").unwrap(),
        Some(json!("fixture-initial-key"))
    );
    clock.0.store(200, Ordering::SeqCst);
    assert_eq!(
        invoke(&mut host, &mut bridge, "key").unwrap(),
        Some(json!("fixture-refreshed-key"))
    );
    assert_eq!(
        invoke(&mut host, &mut bridge, "key").unwrap(),
        Some(json!("fixture-refreshed-key"))
    );
    assert_eq!(
        invoke(&mut host, &mut bridge, "counts").unwrap(),
        Some(json!({"login":1,"refresh":1,"keys":3}))
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let record = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
    assert_eq!(record.revision, 1);
    let credential: Value = serde_json::from_slice(record.raw.expose()).unwrap();
    assert_eq!(credential["access"], "fixture-refreshed");
    assert_eq!(credential["extra"], json!({"keep":true}));
    assert!(
        runtime
            .block_on(backend.create("fixture", SecretBytes::new(b"{}".to_vec())))
            .is_err()
    );
    invoke(&mut host, &mut bridge, "replace").unwrap();
    assert!(invoke(&mut host, &mut bridge, "old").is_err());
    assert_eq!(
        invoke(&mut host, &mut bridge, "counts").unwrap(),
        Some(json!({"login":1,"refresh":1,"keys":3}))
    );
    assert_eq!(
        runtime
            .block_on(backend.load("fixture"))
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    let mut model_credentials = extension_auth::Service::new(
        backend.clone(),
        vec![extension_auth::Grant {
            provider: "fixture".into(),
            hook_owners: [f.entry.path.clone()].into(),
        }],
        clock.clone(),
    )
    .unwrap();
    assert_eq!(
        ag1_platform::extension_providers::OAuth::credentials(
            &mut model_credentials,
            "fixture",
            &f.entry.path,
            &control(&cancel)
        )
        .unwrap(),
        Some(credential.clone())
    );
    assert!(
        ag1_platform::extension_providers::OAuth::credentials(
            &mut model_credentials,
            "fixture",
            "/ungranted/extension.ts",
            &control(&cancel)
        )
        .is_err()
    );
    clock.0.store(2000, Ordering::SeqCst);
    for mode in ["ok", "bad-refresh", "throw-refresh", "bad-login"] {
        host.call(
            &f.scope,
            "command",
            &json!({"name":"mode","arguments":mode}),
            &control(&cancel),
            &mut bridge,
        )
        .unwrap();
        let error = invoke(
            &mut host,
            &mut bridge,
            if mode == "bad-login" { "login" } else { "key" },
        )
        .unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-sensitive-secret"));
        let retained = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
        assert_eq!(retained.revision, 1);
        assert_eq!(
            serde_json::from_slice::<Value>(retained.raw.expose()).unwrap(),
            credential
        );
    }
    host.stop().unwrap();
}

#[test]
fn oauth_inference_credentials_and_extension_key_disclosure_have_independent_native_grants() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        executor::BlockingPool,
        extension_auth,
        extension_auth_bridge::{KeyBridge, KeyGrant, ModelCredentials, RecipientGrant},
        extension_custom_models::{self, CustomModels},
        extension_providers::{Grant, NoOAuth, RegistryBridge},
        provider::credential_storage::StorageCredentialBackend,
    };
    struct NoFallback;
    impl ModelResolver for NoFallback {
        fn resolve(
            &mut self,
            _: InferenceInput<'_>,
            _: &ScanControl<'_>,
        ) -> Result<ag1_platform::extension_inference::ResolvedGeneration, DomainError> {
            panic!("OAuth custom inference used native stock fallback")
        }
    }
    impl extension_custom_models::Credentials for NoFallback {
        fn options(
            &mut self,
            _: &str,
            _: &Value,
            _: Value,
            _: &ScanControl<'_>,
        ) -> Result<Value, DomainError> {
            panic!("OAuth key was replaced by a fallback credential")
        }
    }
    let f = Fixture::new(
        r#"import {complete} from '@ag1/ai';export default api=>{
        let refresh=0,streams=0;
        api.registerProvider('fixture',{api:'custom',baseUrl:'https://fixture.invalid',models:[{id:'model',name:'Model',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:4096,maxTokens:256}],
            oauth:{name:'Fixture',refreshToken:async old=>{refresh++;if(old.refresh!=='fixture-refresh')throw new Error('bad private refresh');return {...old,access:'fixture-fresh',expires:1000000}},getApiKey:cred=>cred.access+'-key'},
            streamSimple:(model,context,options)=>{
                if(options.apiKey!=='fixture-fresh-key')throw new Error('private native key missing');streams++;
                const message={role:'assistant',provider:model.provider,model:model.id,api:model.api,content:[{type:'text',text:'authenticated'}],usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:12};
                return {async *[Symbol.asyncIterator](){yield {type:'start',partial:structuredClone(message)};yield {type:'done',reason:'stop',message}},async result(){return message}};
            }});
        api.registerCommand('infer',{handler:()=>complete({provider:'fixture',id:'model',baseUrl:'https://forged.invalid'},{messages:[{role:'user',content:'hello',timestamp:1}]},{temperature:0})});
        api.registerCommand('provider-key',{handler:(_,ctx)=>ctx.modelRegistry.getApiKeyForProvider('fixture')});
        api.registerCommand('model-key',{handler:(id,ctx)=>ctx.modelRegistry.getApiKey({provider:'fixture',id,baseUrl:'https://forged.invalid'})});
        api.registerCommand('counts',{handler:()=>({refresh,streams})});
    }"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut registration = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registration.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registration,
    )
    .unwrap();
    let vault = tempfile::tempdir().unwrap();
    let backend = StorageCredentialBackend::new(
        ag1_storage::Store::open(vault.path()).unwrap(),
        Arc::new(BlockingPool::new(2, 16).unwrap()),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(backend.import("fixture",SecretBytes::new(json!({"type":"oauth","access":"fixture-old","refresh":"fixture-refresh","expires":0}).to_string().into_bytes()))).unwrap();
    let auth = Arc::new(
        extension_auth::Service::new(
            backend,
            vec![extension_auth::Grant {
                provider: "fixture".into(),
                hook_owners: [f.entry.path.clone()].into(),
            }],
            Arc::new(TestClock),
        )
        .unwrap(),
    );
    for variant in 0..4 {
        let credential_grants = if variant == 0 {
            vec![]
        } else {
            vec![RecipientGrant {
                provider: "fixture".into(),
                hook_owners: [f.entry.path.clone()].into(),
            }]
        };
        let credentials =
            ModelCredentials::new(catalog.clone(), auth.clone(), credential_grants, NoFallback)
                .unwrap();
        let resolver = CustomModels::new(
            catalog.clone(),
            vec![extension_custom_models::Config {
                provider: "fixture".into(),
                model: "model".into(),
                endpoint: "fixture-endpoint".into(),
                callers: [f.entry.path.clone()].into(),
                hook_owners: [f.entry.path.clone()].into(),
                origins: ["https://fixture.invalid".into()].into(),
                options: json!({}),
                allowed_options: ["temperature".into()].into(),
                max_output_bytes: 65536,
                overall_ms: 2000,
            }],
            NoFallback,
            credentials,
            Arc::new(TestClock),
        )
        .unwrap();
        let scheduler = Scheduler::default();
        scheduler
            .register("fixture-endpoint", EndpointKind::Local)
            .unwrap();
        let inference = InferenceBridge::new(
            scheduler.clone(),
            ProviderHttp::new(&["https://fixture.invalid".into()], Duration::from_secs(1)).unwrap(),
            resolver,
            DenyBridge,
        )
        .unwrap();
        let grants = if variant < 2 {
            vec![]
        } else {
            vec![KeyGrant {
                provider: "fixture".into(),
                readers: [f.entry.path.clone()].into(),
                model_ids: ["model".into()].into(),
                provider_lookup: variant == 3,
            }]
        };
        let mut bridge =
            KeyBridge::new(&host, catalog.clone(), auth.clone(), grants, inference).unwrap();
        let output = host.call(
            &f.scope,
            "command",
            &json!({"name":"infer","arguments":""}),
            &control(&cancel),
            &mut bridge,
        );
        if variant == 0 {
            assert!(output.is_err())
        } else {
            let output = output.unwrap().unwrap();
            assert_eq!(output["content"][0]["text"], "authenticated");
            assert!(!output.to_string().contains("fixture-fresh-key"));
        }
        let provider_key = host.call(
            &f.scope,
            "command",
            &json!({"name":"provider-key","arguments":""}),
            &control(&cancel),
            &mut bridge,
        );
        if variant == 3 {
            assert_eq!(provider_key.unwrap(), Some(json!("fixture-fresh-key")))
        } else {
            assert!(provider_key.is_err())
        }
        let model_key = host.call(
            &f.scope,
            "command",
            &json!({"name":"model-key","arguments":"model"}),
            &control(&cancel),
            &mut bridge,
        );
        if variant >= 2 {
            assert_eq!(model_key.unwrap(), Some(json!("fixture-fresh-key")))
        } else {
            assert!(model_key.is_err())
        }
        assert!(
            host.call(
                &f.scope,
                "command",
                &json!({"name":"model-key","arguments":"not-granted"}),
                &control(&cancel),
                &mut bridge
            )
            .is_err()
        );
        let counts = host
            .call(
                &f.scope,
                "command",
                &json!({"name":"counts","arguments":""}),
                &control(&cancel),
                &mut bridge,
            )
            .unwrap()
            .unwrap();
        assert_eq!(counts["refresh"], if variant == 0 { 0 } else { 1 });
        assert_eq!(counts["streams"], variant);
        assert_eq!(scheduler.status("fixture-endpoint").unwrap().active, 0);
        assert_eq!(scheduler.status("fixture-endpoint").unwrap().quarantined, 0);
    }
    host.stop().unwrap();
}

#[test]
fn native_login_routes_only_granted_oauth_ui_and_cancellation_preserves_the_vault() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        executor::BlockingPool,
        extension_auth,
        extension_login::{self, LoginUi},
        extension_providers::{Grant, NoOAuth, RegistryBridge},
        provider::{auth::CredentialBackend, credential_storage::StorageCredentialBackend},
    };
    struct Ui<'a> {
        manual: bool,
        cancel: Option<&'a AtomicBool>,
        events: Vec<String>,
    }
    impl LoginUi for Ui<'_> {
        fn auth(&mut self, info: &Value, control: &ScanControl<'_>) -> Result<(), DomainError> {
            control.check()?;
            assert_eq!(info["url"], "https://fixture.invalid/login");
            self.events.push("auth".into());
            Ok(())
        }
        fn prompt(
            &mut self,
            prompt: &Value,
            control: &ScanControl<'_>,
        ) -> Result<SecretBytes, DomainError> {
            control.check()?;
            assert_eq!(prompt["message"], "Fixture code");
            self.events.push("prompt".into());
            if let Some(cancel) = self.cancel {
                cancel.store(true, Ordering::SeqCst)
            }
            Ok(SecretBytes::new(b"fixture-code".to_vec()))
        }
        fn progress(
            &mut self,
            message: &str,
            control: &ScanControl<'_>,
        ) -> Result<(), DomainError> {
            control.check()?;
            assert_eq!(message, "verified");
            self.events.push("progress".into());
            Ok(())
        }
        fn supports_manual_code(&self) -> bool {
            self.manual
        }
        fn manual_code(&mut self, control: &ScanControl<'_>) -> Result<SecretBytes, DomainError> {
            control.check()?;
            self.events.push("manual".into());
            Ok(SecretBytes::new(b"fixture-manual".to_vec()))
        }
    }
    let f = Fixture::new(
        r#"export default api=>{
        api.registerProvider('fixture',{api:'custom',oauth:{name:'Fixture',login:async callbacks=>{
            callbacks.onAuth({url:'https://fixture.invalid/login',instructions:'Fixture only'});
            const code=await callbacks.onPrompt({message:'Fixture code',placeholder:'code',allowEmpty:false});
            if(code!=='fixture-code')throw new Error('bad fixture prompt');callbacks.onProgress('verified');
            const manual=callbacks.onManualCodeInput?await callbacks.onManualCodeInput():undefined;
            let denied=0;try{await api.setModel({provider:'fixture',id:'forbidden'})}catch{denied++}
            try{api.registerProvider('fixture',{api:'forbidden'})}catch{denied++}
            if(denied!==2)throw new Error('login acquired unrelated parent authority');
            return {access:manual??code,refresh:'fixture-refresh',expires:1000000,manualAvailable:!!callbacks.onManualCodeInput};
        },getApiKey:cred=>cred.access}});
    }"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut registry = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registry.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registry,
    )
    .unwrap();
    let vault = tempfile::tempdir().unwrap();
    let backend = StorageCredentialBackend::new(
        ag1_storage::Store::open(vault.path()).unwrap(),
        Arc::new(BlockingPool::new(2, 16).unwrap()),
    );
    let service = extension_auth::Service::new(
        backend.clone(),
        vec![extension_auth::Grant {
            provider: "fixture".into(),
            hook_owners: [f.entry.path.clone()].into(),
        }],
        Arc::new(TestClock),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for manual in [false, true] {
        let mut ui = Ui {
            manual,
            cancel: None,
            events: vec![],
        };
        extension_login::login(
            &service,
            &catalog,
            &mut host,
            "fixture",
            &mut ui,
            &control(&cancel),
        )
        .unwrap();
        assert_eq!(
            ui.events,
            if manual {
                vec!["auth", "prompt", "progress", "manual"]
            } else {
                vec!["auth", "prompt", "progress"]
            }
        );
        assert_eq!(catalog.generation().unwrap(), 1);
        let record = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
        assert_eq!(record.revision, u64::from(manual));
        let value: Value = serde_json::from_slice(record.raw.expose()).unwrap();
        assert_eq!(value["manualAvailable"], manual);
        assert_eq!(
            value["access"],
            if manual {
                "fixture-manual"
            } else {
                "fixture-code"
            }
        );
    }
    let before = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
    let mut ui = Ui {
        manual: false,
        cancel: Some(&cancel),
        events: vec![],
    };
    assert!(matches!(
        extension_login::login(
            &service,
            &catalog,
            &mut host,
            "fixture",
            &mut ui,
            &control(&cancel)
        ),
        Err(DomainError::Cancelled)
    ));
    assert!(!host.is_alive());
    let after = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.raw.expose(), before.raw.expose());
}

#[test]
fn registered_oauth_native_http_uses_private_refreshed_keys_only_for_current_granted_models() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        executor::BlockingPool,
        extension_auth,
        extension_auth_bridge::{HttpCredentials, HttpGrant, HttpKeyKind, KeyBridge},
        extension_custom_models::{self, CustomModels},
        extension_providers::{Grant, NoOAuth, RegistryBridge},
        provider::credential_storage::StorageCredentialBackend,
    };
    struct NoFallback;
    impl Credentials for NoFallback {
        fn resolve(&mut self, _: &Value, _: &ScanControl<'_>) -> Result<Credential, DomainError> {
            panic!("native OAuth credential fallback was used")
        }
    }
    impl extension_custom_models::Credentials for NoFallback {
        fn options(
            &mut self,
            _: &str,
            _: &Value,
            _: Value,
            _: &ScanControl<'_>,
        ) -> Result<Value, DomainError> {
            panic!("native HTTP model used custom stream credentials")
        }
    }
    let (origin, hits, server) = provider_fixture(false);
    let f=Fixture::new(&r#"import {complete} from '@ag1/ai';export default api=>{
        let refresh=0,keys=0;
        const config={api:'openai-completions',baseUrl:'FIXTURE_ORIGIN/v1',models:[{id:'model',name:'Owned HTTP model',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:4096,maxTokens:256}],
            oauth:{name:'Fixture',refreshToken:async old=>{refresh++;return {...old,access:'fixture-http-fresh',expires:1000000}},getApiKey:cred=>{keys++;return cred.access+'-key'}}};
        api.registerProvider('fixture',config);
        api.registerCommand('infer',{handler:()=>complete({provider:'fixture',id:'model',baseUrl:'https://forged.invalid',apiKey:'forged-key'},{messages:[{role:'user',content:'hello',timestamp:1}]},{temperature:0})});
        api.registerCommand('key',{handler:(_,ctx)=>ctx.modelRegistry.getApiKeyForProvider('fixture')});
        api.registerCommand('counts',{handler:()=>({refresh,keys})});
        api.registerCommand('replace',{handler:()=>api.registerProvider('fixture',{...config,baseUrl:'https://changed.invalid/v1'})});
    }"#.replace("FIXTURE_ORIGIN",&origin));
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut registration = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registration.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registration,
    )
    .unwrap();
    let model = catalog
        .models()
        .unwrap()
        .into_iter()
        .find(|m| m["provider"] == "fixture" && m["id"] == "model")
        .unwrap();
    let vault = tempfile::tempdir().unwrap();
    let backend = StorageCredentialBackend::new(
        ag1_storage::Store::open(vault.path()).unwrap(),
        Arc::new(BlockingPool::new(2, 16).unwrap()),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(backend.import("fixture",SecretBytes::new(json!({"type":"oauth","access":"fixture-http-old","refresh":"fixture-refresh","expires":0}).to_string().into_bytes()))).unwrap();
    let auth = Arc::new(
        extension_auth::Service::new(
            backend,
            vec![extension_auth::Grant {
                provider: "fixture".into(),
                hook_owners: [f.entry.path.clone()].into(),
            }],
            Arc::new(TestClock),
        )
        .unwrap(),
    );
    for variant in 0..5 {
        if variant == 4 {
            host.call(
                &f.scope,
                "command",
                &json!({"name":"replace","arguments":""}),
                &control(&cancel),
                &mut registration,
            )
            .unwrap();
        }
        let credentials = HttpCredentials::new(
            catalog.clone(),
            auth.clone(),
            vec![HttpGrant {
                provider: "fixture".into(),
                model_ids: if variant == 1 {
                    Default::default()
                } else {
                    ["model".into()].into()
                },
                origins: if variant == 0 {
                    Default::default()
                } else {
                    [origin.clone()].into()
                },
                kind: HttpKeyKind::ApiKey,
            }],
            NoFallback,
        )
        .unwrap();
        let mut configured = model.clone();
        if variant == 2 {
            configured["name"] = json!("stale host configuration");
        }
        let configured = ConfiguredModels::new(
            f.scope.clone(),
            [f.entry.path.clone()].into(),
            vec![ModelConfig {
                model: configured,
                endpoint: origin.clone(),
                options: json!({}),
                allowed_options: ["temperature".into()].into(),
                headers: None,
                limits: GenerationLimits {
                    max_output_bytes: 65536,
                    prefill_ms: 0,
                    idle_ms: 0,
                    overall_ms: 2000,
                },
                max_response_bytes: 1024 * 1024,
            }],
            credentials,
            Arc::new(TestClock),
        )
        .unwrap();
        // Exercise forwarding through the production custom/native resolver chain.
        let resolver = CustomModels::new(
            catalog.clone(),
            vec![],
            configured,
            NoFallback,
            Arc::new(TestClock),
        )
        .unwrap();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let inference = InferenceBridge::new(
            scheduler.clone(),
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap(),
            resolver,
            DenyBridge,
        )
        .unwrap();
        let mut bridge =
            KeyBridge::new(&host, catalog.clone(), auth.clone(), vec![], inference).unwrap();
        let result = host.call(
            &f.scope,
            "command",
            &json!({"name":"infer","arguments":""}),
            &control(&cancel),
            &mut bridge,
        );
        if variant == 3 {
            let result = result.unwrap().unwrap();
            assert_eq!(result["content"][0]["text"], "hello world");
            assert!(!result.to_string().contains("fixture-http-fresh"));
        } else {
            assert!(result.is_err(), "variant {variant}");
        }
        assert!(
            host.call(
                &f.scope,
                "command",
                &json!({"name":"key","arguments":""}),
                &control(&cancel),
                &mut bridge
            )
            .is_err()
        );
        let counts = host
            .call(
                &f.scope,
                "command",
                &json!({"name":"counts","arguments":""}),
                &control(&cancel),
                &mut bridge,
            )
            .unwrap()
            .unwrap();
        assert_eq!(counts["refresh"], if variant < 3 { 0 } else { 1 });
        assert_eq!(counts["keys"], if variant < 3 { 0 } else { 1 });
        assert_eq!(hits.load(Ordering::SeqCst), usize::from(variant >= 3));
        assert_eq!(scheduler.status(&origin).unwrap().active, 0);
        assert_eq!(scheduler.status(&origin).unwrap().quarantined, 0);
    }
    let request = server.join().unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-http-fresh-key")
    );
    assert!(!request.contains("forged-key"));
    assert!(request.starts_with("POST /v1/chat/completions "));
    host.stop().unwrap();
}

#[test]
fn native_logout_fences_inflight_login_and_retains_no_current_secret() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        executor::BlockingPool,
        extension_auth,
        extension_host::NativeStream,
        extension_login::{self, LoginUi},
        extension_providers::{Grant, NoOAuth, OAuthTarget, RegistryBridge},
        provider::{
            auth::{CredentialBackend, REVOKED_CREDENTIAL_TYPE},
            credential_storage::StorageCredentialBackend,
        },
    };
    type Auth = extension_auth::Service<StorageCredentialBackend<ag1_storage::Store>>;
    struct Ui<'a> {
        auth: &'a Auth,
        target: &'a OAuthTarget,
        revoke: bool,
    }
    impl LoginUi for Ui<'_> {
        fn auth(&mut self, _: &Value, _: &ScanControl<'_>) -> Result<(), DomainError> {
            Ok(())
        }
        fn progress(&mut self, _: &str, _: &ScanControl<'_>) -> Result<(), DomainError> {
            Ok(())
        }
        fn prompt(
            &mut self,
            _: &Value,
            control: &ScanControl<'_>,
        ) -> Result<SecretBytes, DomainError> {
            if self.revoke {
                self.auth.logout(self.target, control)?;
            }
            Ok(SecretBytes::new(b"fixture-login-secret".to_vec()))
        }
    }
    struct NoHooks;
    impl NativeStream for NoHooks {
        fn emit(&mut self, _: &Value) -> Result<(), DomainError> {
            panic!("revoked credential emitted provider output")
        }
        fn cancelled(&mut self) -> Result<bool, DomainError> {
            panic!("revoked credential invoked a provider hook")
        }
    }
    let f = Fixture::new(
        r#"export default api=>api.registerProvider('fixture',{api:'custom',oauth:{name:'Fixture',
        login:async callbacks=>({access:await callbacks.onPrompt({message:'Fixture login'}),refresh:'fixture-refresh-secret',expires:1000000}),
        refreshToken:()=>{throw new Error('must not refresh revoked credential')},getApiKey:cred=>cred.access}});"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let mut registry = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        NoOAuth,
        DenyBridge,
    )
    .unwrap();
    let catalog = registry.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registry,
    )
    .unwrap();
    let target = catalog
        .oauth_target_for_host(&host, "fixture")
        .unwrap()
        .unwrap();
    let vault = tempfile::tempdir().unwrap();
    let backend = StorageCredentialBackend::new(
        ag1_storage::Store::open(vault.path()).unwrap(),
        Arc::new(BlockingPool::new(2, 16).unwrap()),
    );
    let service = Auth::new(
        backend.clone(),
        vec![extension_auth::Grant {
            provider: "fixture".into(),
            hook_owners: [f.entry.path.clone()].into(),
        }],
        Arc::new(TestClock),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(matches!(
        extension_login::logout(
            &service,
            &catalog,
            &host,
            "fixture",
            &control(&AtomicBool::new(true))
        ),
        Err(DomainError::Cancelled)
    ));
    assert!(runtime.block_on(backend.load("fixture")).unwrap().is_none());
    let mut ui = Ui {
        auth: &service,
        target: &target,
        revoke: true,
    };
    assert!(matches!(
        extension_login::login(
            &service,
            &catalog,
            &mut host,
            "fixture",
            &mut ui,
            &control(&cancel)
        ),
        Err(DomainError::Conflict)
    ));
    assert!(host.is_alive());
    let revoked = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
    assert_eq!(revoked.revision, 0);
    assert_eq!(
        serde_json::from_slice::<Value>(revoked.raw.expose()).unwrap(),
        json!({"type":REVOKED_CREDENTIAL_TYPE})
    );
    assert!(
        service
            .key(
                &target,
                false,
                &control(&cancel),
                &mut NoHooks,
                &mut DenyBridge
            )
            .unwrap()
            .is_none()
    );
    // Revocation is idempotent, and a new explicit login can replace its tombstone.
    extension_login::logout(&service, &catalog, &host, "fixture", &control(&cancel)).unwrap();
    assert_eq!(
        runtime
            .block_on(backend.load("fixture"))
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    ui.revoke = false;
    extension_login::login(
        &service,
        &catalog,
        &mut host,
        "fixture",
        &mut ui,
        &control(&cancel),
    )
    .unwrap();
    let logged_in = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
    assert_eq!(logged_in.revision, 1);
    assert!(String::from_utf8_lossy(logged_in.raw.expose()).contains("fixture-login-secret"));
    // An older interactive replacement cannot restore credentials after logout either.
    ui.revoke = true;
    assert!(matches!(
        extension_login::login(
            &service,
            &catalog,
            &mut host,
            "fixture",
            &mut ui,
            &control(&cancel)
        ),
        Err(DomainError::Conflict)
    ));
    let revoked = runtime.block_on(backend.load("fixture")).unwrap().unwrap();
    assert_eq!(revoked.revision, 2);
    assert_eq!(
        serde_json::from_slice::<Value>(revoked.raw.expose()).unwrap(),
        json!({"type":REVOKED_CREDENTIAL_TYPE})
    );
    assert!(
        service
            .key(
                &target,
                false,
                &control(&cancel),
                &mut NoHooks,
                &mut DenyBridge
            )
            .unwrap()
            .is_none()
    );
    host.stop().unwrap();
}

#[test]
fn credential_changes_refresh_native_and_helper_model_views_atomically() {
    use ag1_core::resources::extension_providers::Snapshot;
    use ag1_platform::{
        executor::BlockingPool,
        extension_auth,
        extension_login::{self, LoginUi},
        extension_providers::{Grant, RegistryBridge},
        provider::credential_storage::StorageCredentialBackend,
    };
    struct Ui;
    impl LoginUi for Ui {
        fn auth(&mut self, _: &Value, _: &ScanControl<'_>) -> Result<(), DomainError> {
            Ok(())
        }
        fn progress(&mut self, _: &str, _: &ScanControl<'_>) -> Result<(), DomainError> {
            Ok(())
        }
        fn prompt(&mut self, _: &Value, _: &ScanControl<'_>) -> Result<SecretBytes, DomainError> {
            Err(DomainError::PermissionDenied)
        }
    }
    let f = Fixture::new(
        r#"export default api=>{
        let mode='ok',calls=0;
        api.registerProvider('fixture',{api:'openai-completions',baseUrl:'https://fixture.invalid/v1',
        models:[{id:'model',name:'Model',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:8192,maxTokens:1024}],
        oauth:{name:'Fixture',login:()=>({access:'fixture-private-access',refresh:'fixture-private-refresh',expires:1000000,projectId:'fixture-account'}),
        refreshToken:c=>c,getApiKey:c=>c.access,
        modifyModels:(models,c)=>{calls++;if(c.access!=='fixture-private-access')throw new Error('wrong credential');
            if(mode==='bad')return {invalid:'not an array'};
            return models.map(m=>m.provider==='fixture'?{...m,name:m.name+' for '+c.projectId}:m);}}});
        api.registerCommand('view',{handler:(_,ctx)=>({models:ctx.modelRegistry.getAll(),calls})});
        api.registerCommand('mode',{handler:args=>{mode=args}});
    };"#,
    );
    let cancel = AtomicBool::new(false);
    let mut host = f
        .service
        .start(f.prepare(true, &cancel), &f.scope, &control(&cancel))
        .unwrap();
    let vault = tempfile::tempdir().unwrap();
    let service = Arc::new(
        extension_auth::Service::new(
            StorageCredentialBackend::new(
                ag1_storage::Store::open(vault.path()).unwrap(),
                Arc::new(BlockingPool::new(2, 16).unwrap()),
            ),
            vec![extension_auth::Grant {
                provider: "fixture".into(),
                hook_owners: [f.entry.path.clone()].into(),
            }],
            Arc::new(TestClock),
        )
        .unwrap(),
    );
    let mut registry = RegistryBridge::new(
        &host,
        Snapshot::default(),
        vec![Grant {
            extension: f.entry.path.clone(),
            providers: ["fixture".into()].into(),
        }],
        service.clone(),
        DenyBridge,
    )
    .unwrap();
    let catalog = registry.catalog();
    host.call(
        &f.scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut registry,
    )
    .unwrap();
    let view = |host: &mut extension_host::ExtensionHost| {
        host.call(
            &f.scope,
            "command",
            &json!({"name":"view","arguments":""}),
            &control(&cancel),
            &mut DenyBridge,
        )
        .unwrap()
        .unwrap()
    };
    assert_eq!(view(&mut host)["calls"], 0);
    assert_eq!(catalog.models().unwrap()[0]["name"], "Model");
    let before = catalog
        .oauth_target_for_host(&host, "fixture")
        .unwrap()
        .unwrap();
    extension_login::login(
        &service,
        &catalog,
        &mut host,
        "fixture",
        &mut Ui,
        &control(&cancel),
    )
    .unwrap();
    registry.refresh(&mut host, &control(&cancel)).unwrap();
    assert!(matches!(before.validate(), Err(DomainError::Conflict)));
    let after = view(&mut host);
    assert_eq!(after["models"], json!(catalog.models().unwrap()));
    assert_eq!(after["models"][0]["name"], "Model for fixture-account");
    assert_eq!(after["calls"], 1);
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/provider-refresh-oracle.json"
    ))
    .unwrap();
    assert_eq!(
        after,
        json!({"models":oracle["rows"][1]["models"],"calls":oracle["rows"][1]["calls"]})
    );
    assert!(!after.to_string().contains("fixture-private"));
    // Rebuilding always starts from baseline, so repeated refresh cannot stack modifications.
    registry.refresh(&mut host, &control(&cancel)).unwrap();
    assert_eq!(view(&mut host)["models"], after["models"]);
    let generation = catalog.generation().unwrap();
    host.call(
        &f.scope,
        "command",
        &json!({"name":"mode","arguments":"bad"}),
        &control(&cancel),
        &mut DenyBridge,
    )
    .unwrap();
    assert!(registry.refresh(&mut host, &control(&cancel)).is_err());
    assert_eq!(catalog.generation().unwrap(), generation);
    assert_eq!(view(&mut host)["models"], after["models"]);
    assert!(host.is_alive());
    assert!(matches!(
        registry.refresh(&mut host, &control(&AtomicBool::new(true))),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(catalog.generation().unwrap(), generation);
    // Logout removes the stored OAuth input, so even the failing hook is no longer called.
    extension_login::logout(&service, &catalog, &host, "fixture", &control(&cancel)).unwrap();
    registry.refresh(&mut host, &control(&cancel)).unwrap();
    let logged_out = view(&mut host);
    assert_eq!(logged_out["models"], json!(catalog.models().unwrap()));
    assert_eq!(logged_out["models"][0]["name"], "Model");
    assert_eq!(logged_out["calls"], 3);
    host.stop().unwrap();
}
