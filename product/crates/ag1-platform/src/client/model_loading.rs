//! Explicit, journaled model preparation. Shares generation admission across chats.
use super::*;
use crate::provider::http::endpoint_origin;

impl Host {
    pub(super) async fn load_local_model(
        self: &Arc<Self>,
        envelope: w::CommandEnvelope,
    ) -> Result<w::BridgeFrame> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Selection {
            provider: String,
            model_id: String,
        }
        let w::Command::Runtime { args, .. } = &envelope.command else {
            return Err(DomainError::NotFound);
        };
        let selection: Selection = serde_json::from_value(args.clone()).map_err(json_error)?;
        if selection.provider != "llamacpp" {
            return Err(DomainError::InvalidInput(
                "Only local models can be loaded".into(),
            ));
        }
        let model = self
            .config
            .models
            .iter()
            .find(|m| m["provider"] == selection.provider && m["id"] == selection.model_id)
            .ok_or(DomainError::NotFound)?;
        if model
            .get("runtimeConflict")
            .is_some_and(|v| v != false && !v.is_null())
        {
            return Err(DomainError::Conflict);
        }
        let runtime = self
            .local_runtime
            .clone()
            .ok_or_else(|| DomainError::Unavailable("Local model runtime is unavailable".into()))?;
        let origin = endpoint_origin(model["baseUrl"].as_str().ok_or(DomainError::NotFound)?)?;
        if self.commands.client_transition_pending()? {
            return Err(DomainError::Conflict);
        }
        let key = format!(
            "native-model-load-{}",
            ag1_storage::digest(json!([envelope.scope, envelope.id]).to_string().as_bytes())
        );
        let admission = self.store.admit_client_command(
            envelope.scope.session.as_str(),
            &key,
            envelope.expected_revision.get(),
            &json!({"kind":"model-load","envelope":envelope}),
        )?;
        if !admission.inserted {
            return admission
                .record
                .result
                .map(|bytes| serde_json::from_slice(&bytes).map_err(json_error))
                .unwrap_or(Err(DomainError::DurabilityUnknown));
        }
        // Keep the accepted operation alive if its renderer/request disconnects.
        let owner = self.clone();
        tokio::spawn(async move {
            let result = async {
                let scheduler = &owner.services.scheduler;
                let ticket = scheduler.submit(&origin, ag1_core::OperationId::new(key.clone())?)?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(900);
                let permit = loop {
                    match scheduler.start(&ticket) {
                        Ok(Some(permit)) => break permit,
                        Ok(None) => (),
                        Err(error) => { let _ = scheduler.cancel(&ticket); return Err(error); }
                    }
                    if tokio::time::Instant::now() >= deadline || scheduler.status(&origin)?.quarantined > 0 {
                        scheduler.cancel(&ticket)?;
                        return Err(DomainError::Unavailable("Model is busy or needs recovery; try loading again when the response finishes".into()));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                };
                // No inference is sent. Successful preparation leaves the model resident.
                let result = match owner.check_authority() {
                    Ok(()) => runtime.load_model(&selection.model_id).await,
                    Err(error) => { permit.terminal()?; return Err(error); }
                };
                if result.is_ok() || runtime.cancel_generation(&origin).await.is_ok() {
                    permit.terminal()?;
                } // Otherwise dropping the permit quarantines uncertain local custody.
                result
            }.await;
            let (status, data) = match result {
                Ok(()) => ("completed", json!({"model":selection.model_id,"phase":"ready"})),
                Err(error) => ("rejected", json!({"error":format!("{error:?}")})),
            };
            let frame = w::BridgeFrame::Reply { id: envelope.id, scope: envelope.scope, revision: owner.commands.snapshot()?.revision, durable: true, status: status.into(), data };
            owner.store.settle_intent(&key, &admission.record.intent, &serde_json::to_vec(&frame).map_err(json_error)?)?;
            Ok(frame)
        }).await.map_err(|_| DomainError::DurabilityUnknown)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::http::{GenerationRuntime, HttpRequest};
    use std::sync::atomic::AtomicUsize;
    #[derive(Default)]
    struct Probe {
        loads: AtomicUsize,
        fail: AtomicBool,
        stops: AtomicUsize,
    }
    impl GenerationRuntime for Probe {
        fn prepare<'a>(&'a self, _: &'a HttpRequest) -> PortFuture<'a, ()> {
            panic!("Manual load must not issue inference")
        }
        fn load_model<'a>(&'a self, model: &'a str) -> PortFuture<'a, ()> {
            Box::pin(async move {
                assert_eq!(model, "local");
                self.loads.fetch_add(1, Ordering::SeqCst);
                if self.fail.load(Ordering::SeqCst) {
                    Err(DomainError::Unavailable("fixture load failed".into()))
                } else {
                    Ok(())
                }
            })
        }
        fn shutdown(&self) -> PortFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn cancel_generation<'a>(&'a self, _: &'a str) -> PortFuture<'a, ()> {
            Box::pin(async move {
                self.stops.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }
    async fn fixture() -> (tempfile::TempDir, Arc<Host>, Arc<Probe>) {
        let d = tempfile::tempdir().unwrap();
        let workspace = d.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let registry = crate::cutover::Registry::at(&d.path().join("authority")).unwrap();
        registry.initialize_isolated(&workspace).unwrap();
        let config: Config = serde_json::from_value(json!({"models":[{"provider":"llamacpp","id":"local","api":"openai-completions","baseUrl":"http://127.0.0.1:18087/v1","contextWindow":16000,"maxTokens":512}],"selected":"local","system_prompt":"Test","fixture":true})).unwrap();
        let runtime = Arc::new(Probe::default());
        let host = Host::open_rehearsal_with_runtime(
            &workspace,
            &d.path().join("state"),
            config,
            &registry,
            runtime.clone(),
        )
        .await
        .unwrap();
        (d, host, runtime)
    }
    fn envelope(host: &Host, id: &str) -> w::CommandEnvelope {
        let s = host.commands.snapshot().unwrap();
        w::CommandEnvelope {
            version: Default::default(),
            id: id.to_owned().try_into().unwrap(),
            scope: s.scope,
            expected_revision: s.revision,
            command: w::Command::Runtime {
                action: "load_model".into(),
                args: json!({"provider":"llamacpp","model_id":"local"}),
            },
            extension: None,
        }
    }
    #[tokio::test]
    async fn manual_load_is_message_free_and_receipt_replay_does_not_reload() {
        let (_d, host, runtime) = fixture().await;
        let before = host.commands.snapshot().unwrap();
        let request = envelope(&host, "manual");
        for _ in 0..2 {
            assert!(
                matches!(host.dispatch(request.clone()).await.unwrap(), w::BridgeFrame::Reply {status, ..} if status=="completed")
            );
        }
        assert_eq!(runtime.loads.load(Ordering::SeqCst), 1);
        let after = host.commands.snapshot().unwrap();
        assert_eq!(before.watermark, after.watermark);
        assert_eq!(before.revision, after.revision);
        let mut bad = envelope(&host, "bad");
        bad.command = w::Command::Runtime {
            action: "load_model".into(),
            args: json!({"provider":"remote","model_id":"local"}),
        };
        assert!(host.dispatch(bad).await.is_err());
        assert_eq!(runtime.loads.load(Ordering::SeqCst), 1);
        host.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn manual_load_waits_for_shared_generation_and_releases_its_slot() {
        let (_d, host, runtime) = fixture().await;
        let scheduler = host.services.scheduler.clone();
        let ticket = scheduler
            .submit(
                "http://127.0.0.1:18087",
                ag1_core::OperationId::new("generation").unwrap(),
            )
            .unwrap();
        let permit = scheduler.start(&ticket).unwrap().unwrap();
        let other = host.open_conversation(None).await.unwrap();
        let request = envelope(&other, "queued-load");
        let task = tokio::spawn(async move { other.dispatch(request).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(runtime.loads.load(Ordering::SeqCst), 0);
        assert_eq!(
            scheduler.status("http://127.0.0.1:18087").unwrap().waiting,
            1
        );
        permit.terminal().unwrap();
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(runtime.loads.load(Ordering::SeqCst), 1);
        assert_eq!(
            scheduler.status("http://127.0.0.1:18087").unwrap().active,
            0
        );
        host.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn failed_load_proves_stop_before_releasing_slot_and_can_retry() {
        let (_d, host, runtime) = fixture().await;
        runtime.fail.store(true, Ordering::SeqCst);
        assert!(
            matches!(host.dispatch(envelope(&host, "failed")).await.unwrap(), w::BridgeFrame::Reply { status, .. } if status=="rejected")
        );
        assert_eq!(runtime.stops.load(Ordering::SeqCst), 1);
        runtime.fail.store(false, Ordering::SeqCst);
        assert!(
            matches!(host.dispatch(envelope(&host, "retry")).await.unwrap(), w::BridgeFrame::Reply { status, .. } if status=="completed")
        );
        host.shutdown().await.unwrap();
    }
}
