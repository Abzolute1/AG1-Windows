use ag1_core::{DomainError, storage::StoredJournal};
use serde_json::{Value, json};

/// Full physical history supplies inspection getters; the active branch remains
/// a distinct view. No legacy JSONL file path is invented for native storage.
pub(crate) fn session(journal: &StoredJournal, branch: &[Value]) -> Result<Value, DomainError> {
    let mut bytes = journal.header.len();
    for entry in &journal.entries {
        bytes = bytes
            .checked_add(entry.normalized.len())
            .ok_or(DomainError::Conflict)?;
    }
    if bytes > 8 * 1024 * 1024 || journal.entries.len() > 100_000 {
        return Err(DomainError::LimitExceeded(
            "extension session inspection".into(),
        ));
    }
    let header: Value =
        serde_json::from_slice(&journal.header).map_err(|_| DomainError::Conflict)?;
    let entries = journal
        .entries
        .iter()
        .map(|entry| {
            serde_json::from_slice::<Value>(&entry.normalized).map_err(|_| DomainError::Conflict)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut labels = serde_json::Map::new();
    let mut label_times = serde_json::Map::new();
    let mut name = None;
    for entry in &entries {
        if entry["type"] == "label" {
            let id = entry["targetId"].as_str().ok_or(DomainError::Conflict)?;
            if let Some(label) = entry["label"].as_str().filter(|s| !s.is_empty()) {
                labels.insert(id.into(), json!(label));
                label_times.insert(id.into(), entry["timestamp"].clone());
            } else {
                labels.remove(id);
                label_times.remove(id);
            }
        } else if entry["type"] == "session_info" {
            name = entry["name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty());
        }
    }
    let mut session = json!({"id":header["id"],"header":header,"cwd":header["cwd"],
        "entries":entries,"branch":branch,"leafId":journal.leaf,"labels":labels,"labelTimestamps":label_times});
    if let Some(name) = name {
        session["name"] = json!(name);
    }
    Ok(session)
}

/// Current host configuration, shared by request, lifecycle and tool callbacks.
pub(in crate::session) fn configuration(
    configuration: &ag1_core::runtime::session::Configuration,
    session: Value,
    idle: bool,
    pending: bool,
) -> Value {
    json!({"model":configuration.model,"systemPrompt":configuration.system_prompt,
        "thinkingLevel":configuration.thinking,
        "activeTools":configuration.tools.iter().filter_map(|t|t["name"].as_str()).collect::<Vec<_>>(),
        "sessionName":session.get("name"),"hasUI":false,"isIdle":idle,
        "hasPendingMessages":pending,"session":session})
}
