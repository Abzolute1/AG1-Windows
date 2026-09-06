use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handled_wire_prompt_settles_once_without_inference_or_user_history() {
    use ag1_platform::session::{
        commands::{RunSettings, SessionCommands},
        registry::{PreparedRuntime, Selection},
    };
    use ag1_protocol::wire as w;
    let mut f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('input',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/handled-inputs',e.source+'\n');return {action:'handled'};});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(false));
    let commands = SessionCommands::new(
        Selection {
            epoch: 1,
            services: Arc::new(PreparedRuntime::from_shared(
                f.host.clone(),
                resolver.clone(),
                f.files.clone(),
            )),
        },
        "input-client".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    let envelope = w::CommandEnvelope {
        version: Default::default(),
        id: "handled".to_owned().try_into().unwrap(),
        scope: commands.scope().clone(),
        expected_revision: w::Sequence::new(0),
        command: w::Command::Prompt {
            input: w::PromptInput::Inline {
                text: "Local extension command".into(),
            },
        },
        extension: None,
    };
    assert!(matches!(
        commands.dispatch(envelope.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    let operation = commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(operation.status, w::OperationStatus::Completed);
    assert!(matches!(
        commands.dispatch(envelope.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    let mut query = envelope;
    query.id = "query-handled".to_owned().try_into().unwrap();
    query.command = w::Command::CommandResult {
        command: "handled".to_owned().try_into().unwrap(),
    };
    assert!(
        matches!(commands.dispatch(query).await.result,w::CommandResult::Settled {operation} if operation.status==w::OperationStatus::Completed)
    );
    assert!(commands.run_next(&ignore).await.unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(f.root.join("handled-inputs")).unwrap(),
        "rpc\n"
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.host.journal_revision().unwrap(), 0);
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_callbacks_transform_before_user_and_task_publication() {
    let (endpoint, server) = server(vec![stop()]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{
      api.on('input',(e,ctx)=>{
        fs.writeFileSync(ctx.cwd+'/input-event',JSON.stringify({event:e,branch:ctx.sessionManager.getBranch()}));
        return {action:'transform',text:'Inspect the native runtime'};
      });
      api.on('input',e=>({action:'transform',text:e.text+' carefully'}));
      api.on('input',()=>{throw Error('isolated input error');});
    };"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    let event: Value =
        serde_json::from_str(&std::fs::read_to_string(f.root.join("input-event")).unwrap())
            .unwrap();
    assert_eq!(event["event"]["source"], "interactive");
    assert!(event["event"].get("images").is_none());
    assert_eq!(event["branch"], json!([]));
    let journal = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let entries = journal
        .entries
        .iter()
        .map(|e| serde_json::from_slice::<Value>(&e.normalized).unwrap())
        .collect::<Vec<_>>();
    let user = entries
        .iter()
        .find(|e| e["message"]["role"] == "user")
        .unwrap();
    assert_eq!(
        user["message"]["content"][0]["text"],
        "Inspect the native runtime carefully"
    );
    assert!(entries.iter().any(|e| {
        e["customType"] == "task_state"
            && e.to_string()
                .contains("Inspect the native runtime carefully")
    }));
    assert!(
        server.join().unwrap()[0]["messages"]
            .to_string()
            .contains("Inspect the native runtime carefully")
    );
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handled_and_malformed_inputs_do_not_append_or_resolve_a_provider() {
    for handled in [true, false] {
        let mut f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1);
        let code = if handled {
            "export default api=>{api.on('input',()=>({action:'handled'}));api.on('input',()=>{throw Error('must not run');});};"
        } else {
            "export default api=>{api.on('input',()=>({action:'transform',text:42}));};"
        };
        let extension = extensions(&f, code);
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let resolver = Resolver::new(false);
        let result = f
            .host
            .run(input(), &resolver, f.files.clone(), &ignore)
            .await;
        if handled {
            let result = result.unwrap();
            assert_eq!(result.generations, 0);
            assert!(result.assistant.is_null());
        } else {
            assert!(matches!(result, Err(DomainError::InvalidInput(_))));
        }
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.host.journal_revision().unwrap(), 0);
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
}
