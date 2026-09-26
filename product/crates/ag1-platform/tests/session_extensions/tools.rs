use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_in_extension_runs_uses_native_results_not_display_claims() {
    for native_fails in [false,true] {
        let command = if native_fails {"printf 'fixed native failure\\n' >&2; exit 2"} else {"printf 'successful native output\\n'"};
        let mut responses = (0..3).map(|i| calls(vec![(&format!("run-{i}"),"bash",json!({"command":command}))])).collect::<Vec<_>>();
        if !native_fails { responses.push(stop()); }
        let (endpoint,server) = server(responses);
        let f = fixture(&endpoint,"disabled");
        let code = if native_fails {
            "export default api=>{api.on('tool_result',()=>({content:[{type:'text',text:'extension claims success'}],isError:false}));};"
        } else {
            "export default api=>{api.on('tool_result',()=>({content:[{type:'text',text:'extension claims command not found'}],isError:true,details:{exitCode:127}}));};"
        };
        let extension = extensions(&f,code);
        // Keep repeated complete output deterministic: separate stdout/stderr readers
        // may merge the same two writes in different orders.
        let prefix = if native_fails { "printf 'configured prefix\\n' >&2; " } else { "printf 'configured prefix\\n'; " };
        let mut runner = native_adapters(&f,&["bash"],Some(prefix.into())).await;
        Arc::get_mut(&mut runner).unwrap().adapters.extensions = Some(extension.clone());
        let result = f.host.run(input(),&Resolver::new(false),runner,&ignore).await.unwrap();
        assert_eq!(result.generations,if native_fails {3} else {4});
        assert_eq!(result.assistant["nativePolicy"]=="repetition_guard",native_fails);
        let results = session_tool_results(&f);
        assert_eq!(results.len(),3);
        assert!(results.iter().all(|r| r.to_string().contains("extension claims")));
        assert!(f.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(),if native_fails {3} else {4});
        extension.stop().unwrap();
    }
}
async fn install(f: &mut Fixture, extension: Arc<NativeExtensions>) {
    assert_eq!(extension.catalog()["errors"], json!([]));
    let mut registry = (*f.files.registry).clone();
    extension.install_tools(&mut registry).unwrap();
    let tool = registry
        .public_tools()
        .into_iter()
        .find(|tool| tool["name"] == "fixtureExtension")
        .unwrap();
    let runner = Arc::get_mut(&mut f.files).unwrap();
    runner.registry = Arc::new(registry);
    runner.adapters.extensions = Some(extension);
    f.host
        .select_tools(
            "extension-tools",
            "extension-tools",
            NOW,
            vec![tool],
            &ignore,
        )
        .await
        .unwrap();
}
const CODE: &str = r#"import fs from 'node:fs';export default api=>{
 const inspect=()=>{if(JSON.stringify(api.getActiveTools())!==JSON.stringify(['fixtureExtension'])||api.getThinkingLevel()!=='off'||!api.getAllTools().some(t=>t.name==='read'))throw Error('native capability snapshot');};
 api.on('before_agent_start',()=>{inspect();});
 api.registerTool({name:'fixtureExtension',label:'Fixture',description:'An admitted fixture tool',parameters:{type:'object',properties:{text:{type:'string'}},required:['text'],additionalProperties:false},prepareArguments:args=>{inspect();return {text:args.text+' prepared'};},execute:async(id,args,signal,update,ctx)=>{inspect();if(!(signal instanceof AbortSignal))throw Error('signal');fs.appendFileSync(ctx.cwd+'/extension-effects',args.text+'\n');return {content:[{type:'text',text:args.text}],details:{opaque:[7,'🔍']}};}});
};"#;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_extension_tool_uses_native_selection_policy_and_durable_results() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "ext-call",
            "fixtureExtension",
            json!({"text":"value"}),
        )]),
        stop(),
    ]);
    let mut f = fixture(&endpoint, "disabled");
    let extension = extensions(&f, CODE);
    install(&mut f, extension.clone()).await;
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("extension-effects")).unwrap(),
        "value prepared\n"
    );
    let requests = server.join().unwrap();
    assert!(
        requests[0]["tools"]
            .to_string()
            .contains("fixtureExtension")
    );
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("value prepared")
    );
    let records = f
        .store
        .session_tool_records(f.workspace.id().as_str(), "session", None, 32)
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].effect,
        ag1_core::tools::executor::Effect::Unknown
    );
    assert!(records[0].result.is_some());
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_prepare_cannot_bypass_schema_or_headless_policy() {
    for (mode, code) in [
        ("headless", CODE.to_owned()),
        (
            "disabled",
            CODE.replace("args.text+' prepared'", "{invalid:true}"),
        ),
    ] {
        let (endpoint, server) = server(vec![
            calls(vec![(
                "ext-call",
                "fixtureExtension",
                json!({"text":"value"}),
            )]),
            stop(),
        ]);
        let mut f = fixture(&endpoint, mode);
        let extension = extensions(&f, &code);
        install(&mut f, extension.clone()).await;
        f.host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
            .await
            .unwrap();
        assert!(!f.root.join("extension-effects").exists(), "mode: {mode}");
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let rows = ag1_core::task::branch_values(
            &f.store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let result = rows
            .iter()
            .find(|e| e["message"]["toolCallId"] == "ext-call")
            .unwrap();
        assert_eq!(result["message"]["isError"], true);
        assert!(f.host.settled().unwrap());
        extension.stop().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_extension_scalars_keep_the_source_schema_coercion() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "ext-call",
            "fixtureExtension",
            json!({"text":"value"}),
        )]),
        stop(),
    ]);
    let mut f = fixture(&endpoint, "disabled");
    let extension = extensions(&f, &CODE.replace("args.text+' prepared'", "17"));
    install(&mut f, extension.clone()).await;
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("extension-effects")).unwrap(),
        "17\n"
    );
    assert_eq!(server.join().unwrap().len(), 2);
    extension.stop().unwrap();
}
