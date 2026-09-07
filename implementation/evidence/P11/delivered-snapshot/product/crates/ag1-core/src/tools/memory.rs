//! Memory tool actions and model-facing presentation reuse P07's reducers.
//! The host publishes `next` with CAS only after authorization; rendering cannot
//! write memory or decide which notes actually reached a provider request.
use crate::{
    DomainError,
    evidence::Freshness,
    memory::{MemoryStore, relevance},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub struct MemoryPlan {
    pub next: Option<MemoryStore>,
    pub result: Value,
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn date(timestamp: i64) -> Result<String, DomainError> {
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .ok_or_else(|| DomainError::InvalidInput("memory date".into()))
}
fn line(
    entry: &Value,
    loaded: bool,
    validate: &mut impl FnMut(&Value) -> Freshness,
) -> Result<String, DomainError> {
    let id = string(entry, "id");
    let kind = string(entry, "kind");
    let priority = if entry["priority"] == "core" {
        "·core"
    } else {
        ""
    };
    let component = entry["component"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(|c| format!("@{c}"))
        .unwrap_or_default();
    let created = entry["createdAt"].as_i64().unwrap();
    let updated = entry["updatedAt"].as_i64().unwrap_or(created);
    let created_suffix = if updated != created {
        format!(" (created {})", date(created)?)
    } else {
        String::new()
    };
    let sources = entry["sources"].as_array().filter(|s| !s.is_empty());
    let source_suffix = sources
        .map(|sources| {
            format!(
                " {{sources: {}}} {{source_status: {}}}",
                sources
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                validate(entry)
            )
        })
        .unwrap_or_default();
    Ok(format!(
        "{id} [{kind}{priority}{component}] {} {}{source_suffix}{created_suffix}{}",
        date(updated)?,
        string(entry, "text"),
        if loaded { " *" } else { "" }
    ))
}
pub fn format_entries(
    store: &MemoryStore,
    mut validate: impl FnMut(&Value) -> Freshness,
) -> Result<String, DomainError> {
    store.value()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| line(entry, false, &mut validate))
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n"))
}
fn list(
    store: &MemoryStore,
    loaded: &BTreeSet<String>,
    validate: &mut impl FnMut(&Value) -> Freshness,
) -> Result<(String, usize), DomainError> {
    let all = store.all();
    if all.is_empty() {
        return Ok(("No memories stored yet.".into(), 0));
    }
    let hot = store.value()["entries"].as_array().unwrap();
    let archived = all.len() - hot.len();
    let remaining = 40usize.saturating_sub(hot.len());
    let near = if remaining < 5 {
        format!(" — near capacity ({remaining} slots left)")
    } else {
        String::new()
    };
    let injected = hot
        .iter()
        .filter(|entry| loaded.contains(string(entry, "id")))
        .count();
    let body = hot
        .iter()
        .map(|entry| line(entry, loaded.contains(string(entry, "id")), validate))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    Ok((
        format!(
            "Project memory: {} stored/retained; {} hot / 40 hot max; {archived} archived{near}\nInjected into model context on the last turn: {injected}. Entries marked * were injected.\n{body}{}\n(* = injected last turn)",
            all.len(),
            hot.len(),
            if archived > 0 {
                "\nUse memory search with a focused query to inspect archived entries."
            } else {
                ""
            }
        ),
        all.len(),
    ))
}
pub fn prepare(
    store: &MemoryStore,
    args: &Value,
    loaded: &BTreeSet<String>,
    now: i64,
    mut validate: impl FnMut(&Value) -> Freshness,
) -> Result<MemoryPlan, DomainError> {
    let action = string(args, "action");
    let invalid = |message: &str| DomainError::InvalidInput(message.into());
    let (next, text, count) = match action {
        "list" => {
            let (text, count) = list(store, loaded, &mut validate)?;
            (None, text, count)
        }
        "search" => {
            let query = string(args, "query").trim();
            if query.is_empty() {
                return Err(invalid("memory search requires a non-empty query."));
            }
            let mut matches = store
                .all()
                .into_iter()
                .map(|entry| (entry, relevance(entry, query)))
                .filter(|(_, score)| *score > 0)
                .collect::<Vec<_>>();
            matches.sort_by(|(a, sa), (b, sb)| {
                sb.cmp(sa)
                    .then_with(|| b["createdAt"].as_i64().cmp(&a["createdAt"].as_i64()))
            });
            matches.truncate(20);
            let text = if matches.is_empty() {
                format!("No project memory matched: {query}")
            } else {
                let mut lines = vec![format!(
                    "Project memory search: {query} ({} matches; model-authored notes, verify provenance)",
                    matches.len()
                )];
                for (entry, score) in &matches {
                    lines.push(format!(
                        "{} {{score: {score}}}",
                        line(entry, false, &mut validate)?
                    ));
                }
                lines.join("\n")
            };
            (None, text, matches.len())
        }
        "clear" => (Some(MemoryStore::default()), "Memory cleared.".into(), 0),
        "remove" => {
            let id = string(args, "id");
            if id.is_empty() {
                return Err(invalid("Pass id to remove a memory."));
            }
            let next = store.remove(id)?;
            let (text, count) = list(&next, loaded, &mut validate)?;
            (Some(next), format!("Removed {id}.\n{text}"), count)
        }
        "update" => {
            let id = string(args, "id");
            if id.is_empty() {
                return Err(invalid("Pass id to update a memory."));
            }
            let patch = ["text", "kind", "priority", "component", "sources"]
                .into_iter()
                .filter_map(|key| args.get(key).map(|value| (key.to_owned(), value.clone())))
                .collect::<serde_json::Map<_, _>>();
            if patch.is_empty() {
                return Err(invalid(
                    "Pass text, kind, priority, component, or sources to update.",
                ));
            }
            let changed = store.update(id, &Value::Object(patch), now)?;
            let text = format!(
                "Updated {id}: {}",
                line(&changed.entry, false, &mut validate)?
            );
            let count = changed.store.all().len();
            (Some(changed.store), text, count)
        }
        "add" => {
            let kind = string(args, "kind");
            let text = string(args, "text");
            if kind.is_empty() || text.is_empty() {
                return Err(invalid("add requires kind and text."));
            }
            let priority = args["priority"].as_str().unwrap_or("normal");
            let changed = store.add(kind, text, now, priority, args)?;
            let id = string(&changed.entry, "id");
            let kind = string(&changed.entry, "kind");
            let mut lines = vec![if changed.duplicate {
                format!("Already stored as {id} [{kind}] (identical text; nothing changed).")
            } else {
                format!(
                    "Stored {id} [{kind}{}]: {}",
                    if priority == "core" { "·core" } else { "" },
                    string(&changed.entry, "text")
                )
            }];
            for entry in &changed.archived {
                lines.push(format!(
                    "Archived {} [{}] from the hot set (still searchable): {}",
                    string(entry, "id"),
                    string(entry, "kind"),
                    string(entry, "text")
                ));
            }
            if changed.near_limit {
                lines.push(
                    "Note: memory store is near capacity. Prune stale entries with memory remove."
                        .into(),
                );
            }
            let count = changed.store.all().len();
            (Some(changed.store), lines.join("\n"), count)
        }
        _ => return Err(invalid("memory action")),
    };
    Ok(MemoryPlan {
        next,
        result: json!({"content":[{"type":"text","text":text}],"details":{"action":action,"count":count}}),
    })
}
