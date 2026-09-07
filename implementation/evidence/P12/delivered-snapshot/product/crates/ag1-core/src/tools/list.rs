//! Locale-aware directory presentation, independently tested against the pinned SDK.
use super::truncate::{self, Limits};
use crate::DomainError;
use icu_collator::{Collator, CollatorBorrowed, options::CollatorOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::LazyLock;
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(Default::default(), CollatorOptions::default())
        .expect("compiled root collation")
});
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    COLLATOR.compare(a, b)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub directory: bool,
    pub symlink: bool,
}
pub fn project(
    path: Option<&str>,
    entries: &[Entry],
    limit: Option<usize>,
) -> Result<Value, DomainError> {
    let limit = limit.unwrap_or(500);
    if limit == 0
        || limit > 10000
        || entries.len() > 10000
        || entries.iter().map(|e| e.name.len()).sum::<usize>() > 4 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("directory listing".into()));
    }
    let mut entries: Vec<_> = entries.iter().map(|e| (e, e.name.to_lowercase())).collect();
    // libuv readdir's lexical order supplies the stable order of case-insensitive ties.
    entries.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    entries.sort_by(|a, b| compare(&a.1, &b.1));
    let results: Vec<_> = entries
        .iter()
        .take(limit)
        .map(|(e, _)| format!("{}{}", e.name, if e.directory { "/" } else { "" }))
        .collect();
    if results.is_empty() {
        return Ok(json!({"content":[{"type":"text","text":"(empty)"}]}));
    }
    let reached = entries.len() > limit;
    let truncation = truncate::truncate(
        &results.join("\n"),
        Limits {
            max_lines: 9_007_199_254_740_991,
            max_bytes: 50 * 1024,
        },
        false,
    );
    let mut output = truncation.content.clone();
    let mut details = json!({"directoryPath":path.unwrap_or("."),"entryCount":results.len(),"entriesSample":results.iter().take(8).collect::<Vec<_>>()});
    let mut notices = vec![];
    if reached {
        details["entryLimitReached"] = json!(limit);
        notices.push(format!("{limit} entries limit; next limit={}", limit * 2));
    }
    if truncation.truncated {
        notices.push("50.0KB byte limit".into());
        details["truncation"] = json!(truncation);
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join("; ")));
    }
    Ok(json!({"content":[{"type":"text","text":output}],"details":details}))
}
