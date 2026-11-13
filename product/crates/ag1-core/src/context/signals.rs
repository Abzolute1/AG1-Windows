//! Retrieval hints reconstructed from the selected branch and explicit captured Git output.
use super::{
    blocks,
    retrieval::{RecentAccess, WorkingTree},
    string,
};
use crate::{DomainError, task::invalid};
use regex::Regex;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Component, Path},
    sync::LazyLock,
};
pub fn workspace_path(root: &Path, value: &str) -> Option<String> {
    // Windows allows a rooted path without a drive prefix. The context layer
    // also uses Unix-style virtual roots, so accept either absolute spelling.
    let root_is_absolute = root.is_absolute() || root.has_root();
    if !root_is_absolute
        || value.trim().is_empty()
        || value.contains(['\r', '\n', '\0', '*', '?', '{', '}', '[', ']'])
    {
        return None;
    }
    let input = Path::new(value);
    let full = if input.is_absolute() || input.has_root() {
        input.to_path_buf()
    } else {
        root.join(value)
    };
    let mut normalized = std::path::PathBuf::new();
    for part in full.components() {
        match part {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    let relative = normalized
        .strip_prefix(root)
        .ok()?
        .to_str()?
        .replace('\\', "/");
    if relative.is_empty() || relative == ".." || relative.starts_with("../") {
        None
    } else {
        Some(relative)
    }
}
/// `is_file` is captured filesystem classification; missing paths use the reference extension heuristic.
pub fn recent(
    root: &Path,
    entries: &[Value],
    exclude: Option<&str>,
    mut is_file: impl FnMut(&str) -> Option<bool>,
) -> Vec<RecentAccess> {
    let mut accesses: BTreeMap<String, RecentAccess> = BTreeMap::new();
    for (distance, entry) in entries.iter().rev().take(500).enumerate() {
        if entry["type"] != "message" {
            continue;
        }
        let m = &entry["message"];
        let mut paths = vec![];
        if m["role"] == "assistant" {
            for b in blocks(&m["content"]) {
                if b["type"] != "toolCall" || exclude.is_some_and(|id| b["id"] == id) {
                    continue;
                }
                let args = &b["arguments"];
                match string(b, "name") {
                    "read" | "inspect" | "edit" | "write" | "append" | "grep" | "find" | "ls" => {
                        paths.push(&args["path"])
                    }
                    "sourceContext" => paths.extend(blocks(&args["paths"])),
                    "diff" => {
                        paths.extend(blocks(&args["paths"]));
                        if !truthy(&args["againstBranch"]) && !truthy(&args["againstCommit"]) {
                            paths.extend([&args["from"], &args["to"]]);
                        }
                    }
                    _ => {}
                }
            }
        } else if m["role"] == "toolResult" && !exclude.is_some_and(|id| m["toolCallId"] == id) {
            let details = &m["details"];
            match crate::tools::surface::message_tool(m) {
                "grep" => paths.push(&details["firstMatchPath"]),
                "sourceContext" => {
                    paths.extend(blocks(&details["files"]).iter().map(|f| &f["path"]))
                }
                "diff" => paths.extend(blocks(&details["files"])),
                _ => {}
            }
        }
        for raw in paths {
            let Some(path) = raw.as_str().and_then(|s| workspace_path(root, s)) else {
                continue;
            };
            let kind = if is_file(&path).unwrap_or_else(|| Path::new(&path).extension().is_some()) {
                "file"
            } else {
                "prefix"
            };
            let key = format!("{kind}:{path}");
            if let Some(previous) = accesses.get_mut(&key) {
                previous.count += 1;
            } else if accesses.len() < 100 {
                accesses.insert(
                    key,
                    RecentAccess {
                        path,
                        kind: kind.into(),
                        count: 1,
                        distance: distance as u64,
                    },
                );
            }
        }
    }
    let mut rows: Vec<_> = accesses.into_values().collect();
    rows.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.path.cmp(&b.path))
    });
    rows
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.),
        _ => true,
    }
}
pub struct GitOutput<'a> {
    pub stdout: &'a str,
    pub code: i32,
}
/// Only hints: changed paths and HEAD never establish source freshness.
pub fn working_tree(
    inside: GitOutput<'_>,
    tracked: GitOutput<'_>,
    untracked: GitOutput<'_>,
    diff: GitOutput<'_>,
) -> Result<WorkingTree, DomainError> {
    if [inside.stdout, tracked.stdout, untracked.stdout, diff.stdout]
        .iter()
        .any(|s| s.len() > 8 * 1024 * 1024)
    {
        return Err(invalid("retrieval Git output limit"));
    }
    if inside.code != 0 || inside.stdout.trim() != "true" {
        return Ok(WorkingTree::default());
    }
    let mut tree = WorkingTree {
        inside_repository: true,
        ..Default::default()
    };
    let mut accepted = 0;
    for output in [tracked, untracked] {
        if output.code != 0 {
            continue;
        }
        for path in output
            .stdout
            .split('\0')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('\\', "/"))
        {
            if !path.starts_with("../") && accepted < 500 {
                tree.changed_files.insert(path);
                accepted += 1;
            }
        }
    }
    static HUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+(\d+)(?:,(\d+))?").unwrap());
    if diff.code == 0 {
        let mut current = None;
        for line in diff.stdout.split('\n') {
            if let Some(path) = line.strip_prefix("+++ ") {
                let path = path.trim();
                current = if path == "/dev/null" {
                    None
                } else {
                    Some(path.strip_prefix("b/").unwrap_or(path).replace('\\', "/"))
                };
                continue;
            }
            let Some(path) = &current else {
                continue;
            };
            if !line.starts_with("@@ ") {
                continue;
            }
            let Some(cap) = HUNK.captures(line) else {
                continue;
            };
            let start = cap[1]
                .parse::<usize>()
                .map_err(|_| invalid("Git hunk line"))?
                .max(1);
            let count = cap
                .get(2)
                .map_or(Ok(1), |m| m.as_str().parse::<usize>())
                .map_err(|_| invalid("Git hunk count"))?
                .clamp(1, 20);
            let target = tree.changed_lines.entry(path.clone()).or_default();
            for offset in 0..count {
                target.insert(
                    start
                        .checked_add(offset)
                        .ok_or_else(|| invalid("Git hunk overflow"))?,
                );
            }
        }
    }
    Ok(tree)
}
