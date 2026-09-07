//! Thin non-GUI client. NativeSession retains all mode completion decisions.
use ag1_platform::client::{Host, actions, nonce};
use ag1_protocol as w;
use serde_json::json;
use std::sync::Arc;
pub type Result<T> = std::result::Result<T, String>;
pub fn envelope(host: &Host, text: &str) -> Result<w::CommandEnvelope> {
    let s = host.commands.snapshot().map_err(|e| format!("{e:?}"))?;
    Ok(w::CommandEnvelope {
        version: Default::default(),
        id: nonce().map_err(|e| format!("{e:?}"))?.try_into()?,
        scope: s.scope,
        expected_revision: s.revision,
        command: actions::parse(text).map_err(|e| format!("{e:?}"))?,
        extension: None,
    })
}
pub async fn print(host: &Arc<Host>, text: &str) -> Result<(String, u8)> {
    let mut command = envelope(host, "unused")?;
    command.command = w::Command::Session {
        action: "print".into(),
        args: json!({"input":{"kind":"inline","text":text}}),
    };
    let reply = host.dispatch(command).await.map_err(|e| format!("{e:?}"))?;
    if let w::BridgeFrame::Reply { status, data, .. } = reply
        && status == "rejected"
    {
        return Err(data.to_string());
    }
    loop {
        let view = host.settled_view().await.map_err(|e| format!("{e:?}"))?;
        if let Some(error) = view.error {
            return Err(error);
        }
        if matches!(
            view.snapshot.status.as_str(),
            "recovery_required" | "persistence_failure"
        ) {
            return Err("Native runtime requires recovery".into());
        }
        if !view.snapshot.pending.is_empty() {
            host.shutdown().await.map_err(|e| format!("{e:?}"))?;
            return Err("Print cannot answer an interactive approval; use JSONL".into());
        }
        if view.snapshot.status == "idle" {
            let selected = host.registry.selected().map_err(|e| format!("{e:?}"))?;
            let records = selected
                .services
                .runtime()
                .client_records()
                .map_err(|e| format!("{e:?}"))?;
            let text = records
                .iter()
                .rev()
                .find(|r| r["type"] == "message" && r["message"]["role"] == "assistant")
                .map(|r| {
                    let content = &r["message"]["content"];
                    if let Some(text) = content.as_str() {
                        text.to_owned()
                    } else {
                        content
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|b| b["type"] == "text")
                            .filter_map(|b| b["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                })
                .unwrap_or_default();
            let failed = host
                .commands
                .snapshot()
                .map_err(|e| format!("{e:?}"))?
                .last_terminal
                .is_some_and(|op| op.status != w::OperationStatus::Completed);
            return Ok((text, u8::from(failed)));
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
