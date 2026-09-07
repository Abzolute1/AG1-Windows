use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_callbacks_transform_each_current_request_without_rewriting_history() {
    let (endpoint, server) = server(vec![
        calls(vec![("inspect", "read", json!({"path":"fixture.txt"}))]),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    std::fs::write(f.root.join("fixture.txt"), "Fresh native evidence").unwrap();
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs'; export default api=>{
      api.on('context',(e,ctx)=>{
        fs.appendFileSync(ctx.cwd+'/context-log',JSON.stringify({messages:e.messages,branch:ctx.sessionManager.getBranch(),model:ctx.model.id,idle:ctx.isIdle()})+'\n');
        return {messages:[...e.messages,{role:'custom',customType:'fixture_context',content:'First projection',display:false,timestamp:0}]};
      });
      api.on('context',e=>{if(e.messages.at(-1).content!=='First projection')throw Error('chain order');return {messages:[...e.messages,{role:'user',content:'Second projection',timestamp:0}]};});
      api.on('context',()=>{throw Error('isolated callback failure');});
    };"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        let text = request["messages"].to_string();
        assert!(text.contains("First projection"));
        assert!(text.contains("Second projection"));
    }
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("Fresh native evidence")
    );
    let events = std::fs::read_to_string(f.root.join("context-log"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["idle"], false);
    assert!(
        events[1]["messages"]
            .to_string()
            .contains("Fresh native evidence")
    );
    assert!(
        events[1]["branch"]
            .to_string()
            .contains("Fresh native evidence")
    );
    for event in events {
        assert!(!event["branch"].to_string().contains("First projection"));
    }
    let journal = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert!(journal.entries.iter().all(|entry| {
        !std::str::from_utf8(&entry.normalized)
            .unwrap()
            .contains("First projection")
    }));
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_extension_context_stops_before_provider_resolution() {
    let mut f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        "export default api=>{api.on('context',()=>({messages:[null]}));};",
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Resolver::new(false);
    assert!(matches!(
        f.host
            .run(input(), &resolver, f.files.clone(), &ignore)
            .await,
        Err(DomainError::InvalidInput(_))
    ));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert!(f.host.settled().unwrap());
    assert!(f.host.persistence_healthy().unwrap());
    let records = f
        .store
        .artifact_catalog("session", None, 10, Some("runtime-extension-output"))
        .unwrap();
    assert_eq!(records.len(), 1);
    let artifact: ag1_core::context::ContextArtifact =
        serde_json::from_value(records[0].record.clone()).unwrap();
    let artifacts = ag1_storage::context::ContextStore::new(
        &f.store,
        SessionId::new("session").unwrap(),
        "session",
        "inspect-context",
        NOW,
        Default::default(),
    )
    .unwrap();
    use ag1_core::tools::executor::ExecutionArtifacts;
    assert_eq!(
        artifacts.read_json(&artifact).unwrap()["result"],
        json!([null])
    );
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_callback_cannot_publish_a_projection_after_the_model_changes() {
    let (endpoint, server) = server(vec![stop()]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 2);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{
      api.on('before_agent_start',()=>({systemPrompt:'Obsolete turn override'}));
      let first=true;api.on('context',async(e,ctx)=>{
        if(first){first=false;fs.writeFileSync(ctx.cwd+'/context-entered','yes');
          while(!fs.existsSync(ctx.cwd+'/context-release'))await new Promise(r=>setTimeout(r,2));}
        return {messages:[...e.messages,{role:'user',content:'Callback model: '+ctx.model.id,timestamp:0}]};
      });};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let host = f.host.clone();
    let files = f.files.clone();
    let resolver = Arc::new(Resolver::new(false));
    let provider = resolver.clone();
    let task =
        tokio::spawn(async move { host.run(input(), provider.as_ref(), files, &ignore).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.root.join("context-entered").exists() {
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
    std::fs::write(f.root.join("context-release"), "yes").unwrap();
    task.await.unwrap().unwrap();
    let requests = server.join().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(requests[0]["model"], "replacement");
    assert!(
        !requests[0]["messages"]
            .to_string()
            .contains("Obsolete turn override")
    );
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("Callback model: replacement")
    );
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_context_callback_waiter_retains_worker_until_cancellation_drains() {
    let mut f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('context',async(e,ctx)=>{fs.writeFileSync(ctx.cwd+'/context-entered','yes');await new Promise(()=>{});});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(false));
    let host = f.host.clone();
    let files = f.files.clone();
    let provider = resolver.clone();
    let task =
        tokio::spawn(async move { host.run(input(), provider.as_ref(), files, &ignore).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.root.join("context-entered").exists() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    // The caller has left; accepted callback ownership must still drain before
    // another prompt or registry replacement can claim the native session.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.host.settled().unwrap() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    extension.stop().unwrap();
}
