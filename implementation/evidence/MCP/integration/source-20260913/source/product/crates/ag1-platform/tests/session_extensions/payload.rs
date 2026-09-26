use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_payload_callbacks_chain_after_formatting_without_rewriting_native_history() {
    let (endpoint, server) = server(vec![stop()]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{
      api.on('before_provider_request',(e,ctx)=>{fs.writeFileSync(ctx.cwd+'/payload-event',JSON.stringify({keys:Object.keys(e),model:ctx.model.id,payload:e.payload}));return {...e.payload,temperature:0.37,messages:[...e.payload.messages,{role:'user',content:'Payload-only instruction 🔍'}]};});
      api.on('before_provider_request',e=>({...e.payload,top_p:0.83}));
      api.on('before_provider_request',()=>{throw Error('isolated payload failure');});
    };"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    let request = &server.join().unwrap()[0];
    assert_eq!(request["temperature"], 0.37);
    assert_eq!(request["top_p"], 0.83);
    assert!(
        request["messages"]
            .to_string()
            .contains("Payload-only instruction 🔍")
    );
    let event: Value =
        serde_json::from_str(&std::fs::read_to_string(f.root.join("payload-event")).unwrap())
            .unwrap();
    let mut keys = event["keys"].as_array().unwrap().clone();
    keys.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(keys, vec![json!("payload"), json!("type")]);
    assert_eq!(event["model"], request["model"]);
    assert!(!event.to_string().contains("owned-fixture"));
    let journal = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert!(journal.entries.iter().all(|e| {
        !std::str::from_utf8(&e.normalized)
            .unwrap()
            .contains("Payload-only instruction")
    }));
    let outputs = f
        .store
        .artifact_catalog("session", None, 10, Some("runtime-extension-output"))
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_callbacks_cannot_replace_native_model_tools_or_stream_authority() {
    for expression in [
        "({...e.payload,model:'different'})",
        "({...e.payload,tools:[]})",
        "({...e.payload,stream:false})",
        "null",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
        let extension = extensions(
            &f,
            &format!("export default api=>{{api.on('before_provider_request',e=>{expression});}};"),
        );
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let result = f
            .host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
            .await;
        assert!(matches!(
            result,
            Err(DomainError::PermissionDenied | DomainError::InvalidInput(_))
        ));
        assert!(
            matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
        );
        assert!(f.host.settled().unwrap());
        extension.stop().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_growth_must_pass_the_same_hard_context_margin_before_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let mut model = f.model.clone();
    model["contextWindow"] = json!(8192);
    f.host
        .select_model("window", "window", NOW, model, &ignore)
        .await
        .unwrap();
    let extension = extensions(
        &f,
        "export default api=>{api.on('before_provider_request',e=>({...e.payload,messages:[...e.payload.messages,{role:'user',content:'additional payload evidence '.repeat(4000)}]}));};",
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Resolver::new(false);
    assert!(
        matches!(f.host.run(input(),&resolver,f.files.clone(),&ignore).await,Err(DomainError::LimitExceeded(message)) if message=="request context admission margin")
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
    assert!(f.host.settled().unwrap());
    assert_eq!(
        f.store
            .artifact_catalog("session", None, 10, Some("runtime-extension-output"))
            .unwrap()
            .len(),
        1
    );
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_changes_while_payload_callback_waits_rebuild_before_any_http_dispatch() {
    let (endpoint, server) = server(vec![stop()]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 2);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{let first=true;api.on('before_provider_request',async(e,ctx)=>{if(first){first=false;fs.writeFileSync(ctx.cwd+'/payload-entered','yes');while(!fs.existsSync(ctx.cwd+'/payload-release'))await new Promise(r=>setTimeout(r,2));}return {...e.payload,metadata:{observedModel:ctx.model.id}};});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(false));
    let provider = resolver.clone();
    let host = f.host.clone();
    let files = f.files.clone();
    let task =
        tokio::spawn(async move { host.run(input(), provider.as_ref(), files, &ignore).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.root.join("payload-entered").exists() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let mut model = f.model.clone();
    model["id"] = json!("replacement");
    f.host
        .select_model("switch", "switch", NOW, model, &ignore)
        .await
        .unwrap();
    std::fs::write(f.root.join("payload-release"), "yes").unwrap();
    task.await.unwrap().unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "replacement");
    assert_eq!(requests[0]["metadata"]["observedModel"], "replacement");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}
