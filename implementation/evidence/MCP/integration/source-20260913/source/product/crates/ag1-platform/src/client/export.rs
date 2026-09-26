use super::*;
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub fn html(view: &w::ClientView) -> Result<String> {
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>AG1 session</title><style>body{max-width:70ch;margin:3rem auto;font:16px system-ui}pre{white-space:pre-wrap;overflow-wrap:anywhere}article{border-bottom:1px solid #bbb;padding:1rem}</style></head><body><h1>AG1 session</h1><p>Bounded session projection</p>",
    );
    for m in &view.details.messages {
        out.push_str(&format!(
            "<article><h2>{}</h2><pre>{}</pre></article>",
            escape(&m.role),
            escape(&serde_json::to_string_pretty(&m.blocks).map_err(json_error)?)
        ));
    }
    out.push_str("<footer><pre>");
    out.push_str(&escape(include_str!("../../../../LICENSE")));
    out.push_str(
        "</pre><p>No third-party JavaScript is embedded in this export.</p></footer></body></html>",
    );
    Ok(out)
}

/// Complete active-branch text, with original tool JSON. Binary attachment bytes
/// stay in journal custody; exports label their references instead of leaking them.
/// Exceeding the explicit 8 MiB file limit is an error, never a shortened export.
pub fn full_html(records: &[Value]) -> Result<String> {
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>AG1 session</title><style>body{max-width:80ch;margin:3rem auto;font:16px system-ui}pre{white-space:pre-wrap;overflow-wrap:anywhere}article{border-bottom:1px solid #bbb;padding:1rem}</style></head><body><h1>AG1 session</h1><p>Complete active-branch text. Attachment bytes remain in the native journal.</p>",
    );
    for entry in records.iter().filter(|r| r["type"] == "message") {
        let mut content = entry["message"]["content"].clone();
        if let Some(blocks) = content.as_array_mut() {
            for block in blocks {
                if block["type"] == "image" {
                    *block = json!({"type":"attachment","mimeType":block["mimeType"],"journalEntry":entry["id"],"binary":"Retained in journal"});
                }
            }
        }
        out.push_str(&format!(
            "<article><h2>{}</h2><small>{}</small><pre>{}</pre></article>",
            escape(entry["message"]["role"].as_str().unwrap_or("unknown")),
            escape(entry["id"].as_str().unwrap_or("")),
            escape(&serde_json::to_string_pretty(&content).map_err(json_error)?)
        ));
        if out.len() > 8 * 1024 * 1024 - 4096 {
            return Err(DomainError::LimitExceeded(
                "HTML export exceeds 8 MiB; use the lossless storage archive API".into(),
            ));
        }
    }
    out.push_str("<footer><pre>");
    out.push_str(&escape(include_str!("../../../../LICENSE")));
    out.push_str("</pre><p>No third-party JavaScript is embedded.</p></footer></body></html>");
    Ok(out)
}
