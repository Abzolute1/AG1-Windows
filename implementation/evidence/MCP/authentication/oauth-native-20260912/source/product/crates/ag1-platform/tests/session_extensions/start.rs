use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planning_callbacks_receive_and_preserve_the_native_planning_prompt() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "plan",
            "submit_plan",
            json!({"summary":"Inspect the runtime","steps":["Read the current source"]}),
        )]),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let original = f.host.configuration().unwrap().system_prompt;
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('before_agent_start',(e,ctx)=>{fs.writeFileSync(ctx.cwd+'/planning-start',e.systemPrompt);return {systemPrompt:e.systemPrompt+'\nExtension plan instruction'};});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let result = ag1_platform::session::planning::run(
        f.host.clone(),
        input(),
        Arc::new(Resolver::new(false)),
        f.files.clone(),
        Arc::new(ignore),
        ag1_platform::session::planning::Options {
            goal: Some("Inspect the current source".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(result.plan.is_some());
    let prompt = std::fs::read_to_string(f.root.join("planning-start")).unwrap();
    assert!(prompt.contains("<planning-mode-guidance>"));
    for request in server.join().unwrap() {
        let text = request["messages"].to_string();
        assert!(text.contains("planning-mode-guidance"));
        assert!(text.contains("Extension plan instruction"));
        assert!(
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["function"]["name"] != "write" && t["function"]["name"] != "edit")
        );
    }
    assert_eq!(f.host.configuration().unwrap().system_prompt, original);
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn before_agent_callbacks_chain_prompts_and_commit_custom_messages_once_per_prompt() {
    let (endpoint, server) = server(vec![
        calls(vec![("inspect", "read", json!({"path":"fixture.txt"}))]),
        stop(),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    std::fs::write(f.root.join("fixture.txt"), "native result").unwrap();
    let original = f.host.configuration().unwrap().system_prompt;
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{
      api.on('before_agent_start',(e,ctx)=>{
        fs.appendFileSync(ctx.cwd+'/before-start',JSON.stringify({event:e,branch:ctx.sessionManager.getBranch()})+'\n');
        return {systemPrompt:e.systemPrompt+'\nFirst turn instruction',message:{customType:'first',content:'First custom message',display:false,details:{opaque:['preserved',7]}}};
      });
      api.on('before_agent_start',e=>({systemPrompt:e.systemPrompt+'\nSecond turn instruction',message:{customType:'second',content:'Second custom message',display:true}}));
    };"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    let mut next = input();
    next.command = "second-prompt".into();
    f.host
        .run(next, &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    for request in requests {
        let text = request["messages"].to_string();
        assert!(text.contains("First turn instruction"));
        assert!(text.contains("Second turn instruction"));
        assert!(text.contains("First custom message"));
        assert!(text.contains("Second custom message"));
    }
    let events = std::fs::read_to_string(f.root.join("before-start"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["branch"], json!([]));
    assert_eq!(events[0]["event"]["systemPrompt"], original);
    assert_eq!(events[1]["event"]["systemPrompt"], original);
    let entries = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap()
        .entries
        .iter()
        .map(|e| serde_json::from_slice::<Value>(&e.normalized).unwrap())
        .collect::<Vec<_>>();
    let first = entries
        .iter()
        .filter(|e| e["message"]["customType"] == "first")
        .collect::<Vec<_>>();
    assert_eq!(first.len(), 2);
    assert_eq!(
        first[0]["message"]["details"],
        json!({"opaque":["preserved",7]})
    );
    assert_eq!(f.host.configuration().unwrap().system_prompt, original);
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn before_agent_messages_require_journal_acknowledgement_before_inference() {
    struct Fault(ag1_storage::Boundary);
    impl ag1_storage::FaultInjector for Fault {
        fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            Ok(())
        }
        fn check_journal_command(
            &self,
            command: &str,
            point: ag1_storage::Boundary,
        ) -> ag1_storage::StorageResult<()> {
            if command.ends_with("-before-agent-message-0") && point == self.0 {
                return Err(ag1_storage::StorageError::Injected(point));
            }
            Ok(())
        }
    }
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let mut f = fixture_configured(
            "http://127.0.0.1:1",
            "disabled",
            vec![],
            Some(Arc::new(Fault(point))),
            None,
            1,
        );
        let extension = extensions(
            &f,
            r#"import fs from 'node:fs';export default api=>{api.on('before_agent_start',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/before-count','once\n');return {message:{customType:'fixture',content:'Retained extension message',display:true}};});};"#,
        );
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let resolver = Resolver::new(false);
        assert!(
            f.host
                .run(input(), &resolver, f.files.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        assert!(!f.host.persistence_healthy().unwrap());
        let mut retry = input();
        retry.command = "after-failed-custody".into();
        assert!(
            f.host
                .run(retry, &resolver, f.files.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join("before-count")).unwrap(),
            "once\n"
        );
        let entries = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap()
            .entries;
        let found = entries.iter().any(|e| {
            serde_json::from_slice::<Value>(&e.normalized).unwrap()["message"]["customType"]
                == "fixture"
        });
        assert_eq!(found, point == ag1_storage::Boundary::BeforeAck);
        extension.stop().unwrap();
    }
}
