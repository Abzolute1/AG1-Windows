//! Repository orientation is data, not shell execution or session mutation.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    pub inside_repo: bool,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub dirty: Vec<String>,
    pub conflicts: Vec<String>,
    pub diff_stat: Option<String>,
    pub head: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct StackHint {
    pub kind: String,
    pub manifest: String,
    pub name: Option<String>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct RepoMap {
    pub stack: Vec<StackHint>,
    pub entries: Vec<String>,
    pub readme_preview: Option<String>,
    pub layout: Vec<String>,
}
pub fn format_git(git: &GitState) -> String {
    if !git.inside_repo {
        return "Git: not a repository.".into();
    }
    let mut title = format!("Git: {}", git.branch.as_deref().unwrap_or("DETACHED"));
    if let Some(upstream) = &git.upstream {
        title.push_str(&format!(" → {upstream}"));
    }
    let mut lines = vec![title];
    if git.ahead > 0 || git.behind > 0 {
        lines.push(format!("ahead {} / behind {}", git.ahead, git.behind));
    }
    if let Some(head) = &git.head {
        lines.push(format!("HEAD {head}"));
    }
    lines.push(if git.conflicts.is_empty() {
        "conflicts: none".into()
    } else {
        format!(
            "conflicts ({}): {}",
            git.conflicts.len(),
            git.conflicts
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    if git.dirty.is_empty() {
        lines.push("dirty: clean".into());
    } else {
        lines.push(format!("dirty ({}):", git.dirty.len()));
        lines.extend(git.dirty.iter().take(30).map(|file| format!("  {file}")));
        if git.dirty.len() > 30 {
            lines.push(format!("  +{} more", git.dirty.len() - 30));
        }
    }
    if let Some(stat) = &git.diff_stat {
        lines.push(stat.clone());
    }
    lines.join("\n")
}
pub fn format_map(map: &RepoMap) -> String {
    let mut lines = vec!["Map:".into()];
    if map.stack.is_empty() {
        lines.push("stack: unknown".into());
    } else {
        lines.push(format!(
            "stack: {}",
            map.stack
                .iter()
                .map(|hint| hint
                    .name
                    .as_ref()
                    .map(|name| format!("{} ({name})", hint.kind))
                    .unwrap_or_else(|| hint.kind.clone()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.extend(map.stack.iter().filter_map(|hint| {
            hint.detail
                .as_ref()
                .map(|detail| format!("  {}: {detail}", hint.manifest))
        }));
    }
    if !map.entries.is_empty() {
        lines.push(format!(
            "entry: {}",
            map.entries
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !map.layout.is_empty() {
        lines.push("layout:".into());
        lines.extend(map.layout.iter().map(|line| format!("  {line}")));
    }
    if let Some(readme) = &map.readme_preview {
        lines.push("readme:".into());
        lines.extend(readme.split('\n').map(|line| format!("  {line}")));
    }
    lines.join("\n")
}
