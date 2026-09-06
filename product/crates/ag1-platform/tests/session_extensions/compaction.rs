use super::*;
use ag1_platform::session::compaction::Reason;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_compaction_veto_preserves_history_and_skips_inference() {
    for reason in [Reason::Manual, Reason::Threshold] {
        let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
        let extension = extensions(
            &f,
            r#"export default api=>{api.on('session_before_compact',e=>{if(!(e.signal instanceof AbortSignal))throw Error('missing signal');return {cancel:true};});};"#,
        );
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let revision = f.host.journal_revision().unwrap();
        let resolver = Resolver::new(false);
        let mut input = compact_input();
        input.reason = reason;
        let result = f
            .host
            .compact_with_callbacks(input, &resolver, f.files.clone(), &ignore)
            .await;
        if reason == Reason::Manual {
            assert!(matches!(result, Err(DomainError::Cancelled)));
        } else {
            assert!(result.unwrap().is_none());
        }
        assert_eq!(f.host.journal_revision().unwrap(), revision);
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        assert!(f.host.settled().unwrap());
        extension.stop().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_compaction_retains_trajectory_and_notification_crash_cannot_undo_commit() {
    for crash in [false, true] {
        let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
        let after = if crash {
            "process.exit(23);"
        } else {
            "fs.writeFileSync(ctx.cwd+'/after-compaction',JSON.stringify({event:e,branch:ctx.sessionManager.getBranch()}));"
        };
        let code = format!(
            "import fs from 'node:fs';export default api=>{{api.on('session_before_compact',e=>({{compaction:{{summary:'Extension working summary',firstKeptEntryId:e.preparation.firstKeptEntryId,tokensBefore:e.preparation.tokensBefore,details:{{opaque:7}}}}}}));api.on('session_compact',(e,ctx)=>{{{after}}});}};"
        );
        let extension = extensions(&f, &code);
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let resolver = Resolver::new(false);
        let notices = Mutex::new(Vec::new());
        let listener = |event: &SessionEvent| {
            if let SessionEvent::Recovery(value) = event {
                notices.lock().unwrap().push(value.clone());
            }
        };
        let result = f
            .host
            .compact_with_callbacks(compact_input(), &resolver, f.files.clone(), &listener)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.context_epoch, 2);
        assert!(
            result.result["summary"]
                .as_str()
                .unwrap()
                .contains("Extension working summary")
        );
        assert_eq!(result.result["details"]["opaque"], 7);
        assert_eq!(result.result["details"]["summaryOutputs"], json!([]));
        assert!(result.result["details"]["trajectoryArtifactId"].is_string());
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        let journal = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        assert_eq!(
            journal
                .entries
                .iter()
                .filter(
                    |e| serde_json::from_slice::<Value>(&e.normalized).unwrap()["type"]
                        == "compaction"
                )
                .count(),
            1
        );
        if crash {
            assert!(
                notices
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "extension_notification_error")
            );
        } else {
            let event: Value = serde_json::from_str(
                &std::fs::read_to_string(f.root.join("after-compaction")).unwrap(),
            )
            .unwrap();
            assert_eq!(event["event"]["fromExtension"], true);
            assert!(
                event["branch"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["id"] == event["event"]["compactionEntry"]["id"])
            );
        }
        assert!(f.host.settled().unwrap());
        assert!(f.host.persistence_healthy().unwrap());
        extension.stop().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_compaction_uses_extension_summary_before_resuming_the_parent() {
    let (endpoint, server) = server(vec![stop()]);
    let mut f = fixture_seeded(&endpoint, "disabled", pressure_history());
    f.host
        .select_tools("tools", "tools", NOW, vec![], &ignore)
        .await
        .unwrap();
    let mut model = f.model.clone();
    model["contextWindow"] = json!(10000);
    f.host
        .select_model("model", "model", NOW, model, &ignore)
        .await
        .unwrap();
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('session_before_compact',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/auto-compaction','once\n');return {compaction:{summary:'Automatic extension summary',firstKeptEntryId:e.preparation.firstKeptEntryId,tokensBefore:e.preparation.tokensBefore}};});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let mut turn = input();
    turn.compaction = compact_input().settings;
    let resolver = Resolver::new(false);
    let result = f
        .host
        .run(turn, &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert!(
        server.join().unwrap()[0]["messages"]
            .to_string()
            .contains("Automatic extension summary")
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("auto-compaction")).unwrap(),
        "once\n"
    );
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_extension_compaction_cannot_select_an_unrelated_cut_point() {
    let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
    let extension = extensions(
        &f,
        "export default api=>{api.on('session_before_compact',()=>({compaction:{summary:'Invalid boundary',firstKeptEntryId:'unrelated-entry',tokensBefore:17}}));};",
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let revision = f.host.journal_revision().unwrap();
    let resolver = Resolver::new(false);
    assert!(matches!(
        f.host
            .compact_with_callbacks(compact_input(), &resolver, f.files.clone(), &ignore)
            .await,
        Err(DomainError::InvalidInput(_))
    ));
    assert_eq!(f.host.journal_revision().unwrap(), revision);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
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
async fn dropped_compaction_callback_releases_only_its_reservation_after_owned_worker_drain() {
    let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
    let extension = extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('session_before_compact',async(e,ctx)=>{fs.writeFileSync(ctx.cwd+'/compact-entered','yes');await new Promise(()=>{});});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(false));
    let provider = resolver.clone();
    let host = f.host.clone();
    let files = f.files.clone();
    let revision = f.host.journal_revision().unwrap();
    let task = tokio::spawn(async move {
        host.compact_with_callbacks(compact_input(), provider.as_ref(), files, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.root.join("compact-entered").exists() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.host.settled().unwrap() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.host.journal_revision().unwrap(), revision);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    extension.stop().unwrap();
}
