//! Stable system prefix assembled from captured session resources, with no filesystem or clock reads.
use super::string;
use crate::{
    DomainError,
    task::{invalid, units},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptInput {
    pub date: String,
    pub cwd: String,
    pub custom_prompt: Option<String>,
    pub selected_tools: Option<Vec<String>>,
    #[serde(default)]
    pub tool_snippets: BTreeMap<String, String>,
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
    pub append_system_prompt: Option<String>,
    pub workspace_snapshot: Option<String>,
    #[serde(default)]
    pub context_files: Vec<ContextFile>,
    #[serde(default)]
    pub skills: Vec<PromptSkill>,
    pub model_provider: Option<String>,
    pub context_window: Option<u64>,
    pub ag1_docs: Option<Docs>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSkill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    #[serde(default)]
    pub disable_model_invocation: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Docs {
    pub readme: String,
    pub docs: String,
    pub examples: String,
}
pub fn compact(provider: Option<&str>) -> bool {
    matches!(provider, Some("local" | "llamacpp" | "llama.cpp"))
}
pub fn routing(tools: &[String]) -> String {
    let has = |s: &str| tools.iter().any(|t| t == s);
    let mut lines = vec![];
    let inspect: Vec<_> = ["read", "grep", "find", "ls"]
        .into_iter()
        .filter(|s| has(s))
        .collect();
    if !inspect.is_empty() {
        lines.push(format!("- inspect files: {}", inspect.join(", ")));
    }
    let mut change = vec![];
    if has("edit") {
        change.push("edit (surgical)");
    }
    if has("write") {
        change.push("write (new or full rewrite)");
    }
    if !change.is_empty() {
        lines.push(format!("- change files: {}", change.join("; ")));
    }
    for (name, line) in [
        (
            "repo",
            "- git status / repo map: repo (bash git only for log, blame, commit, checkout)",
        ),
        (
            "sourceContext",
            "- build/list/validate durable multi-file source capsules with revision provenance: sourceContext",
        ),
        (
            "artifact",
            "- recover externalized raw evidence: unknown id → artifact {\"action\":\"search\",\"query\":\"distinctive text\"}; known id → grep, then read if surrounding bytes are needed",
        ),
        (
            "memory",
            "- remember across sessions: memory add (decision | style | breakage | fact)",
        ),
        (
            "taskState",
            "- maintain current objective, constraints, and typed task knowledge: taskState",
        ),
        (
            "episode",
            "- bracket substantial long-task phases and preserve exact trajectories: episode",
        ),
        (
            "check",
            "- compile / typecheck: check (no args; uses this project's compiler)",
        ),
        (
            "job",
            "- long / background commands: job (start/poll/wait/log/kill); do not bash-sleep on a job",
        ),
        ("task", "- delegate a bounded subtask: task"),
        (
            "explore",
            "- coordinated read-only search: explore (workers share findings in sequence)",
        ),
    ] {
        if has(name) {
            lines.push(line.into());
        }
    }
    let web: Vec<_> = ["webSearch", "webFetch"]
        .into_iter()
        .filter(|s| has(s))
        .collect();
    if !web.is_empty() {
        lines.push(format!("- online docs: {}", web.join(" then ")));
    }
    if has("todo") {
        lines.push("- multi-step work: todo".into());
    }
    if ["screenshot_url", "interact_then_screenshot", "verify_ui"]
        .iter()
        .any(|s| has(s))
    {
        lines.push("- verify UI you built: verify_ui (concrete claim) or screenshot_url (HTML path or localhost URL); interact_then_screenshot after clicks/fills. Do not start a server just to screenshot static HTML.".into());
    } else if has("verify") {
        lines.push("- verify a claim: verify".into());
    }
    if has("bash") {
        lines.push("- shell pipelines / tests / jq: bash — do not use bash as a substitute for the named tools above".into());
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!(
            "Tool routing — call the named tool. Do not invent a substitute.\n{}",
            lines.join("\n")
        )
    }
}
pub fn catalog(tools: &[String], snippets: &BTreeMap<String, String>) -> String {
    let visible: Vec<_> = tools
        .iter()
        .filter_map(|name| {
            snippets
                .get(name)
                .filter(|s| !s.is_empty())
                .map(|s| format!("- {name}: {s}"))
        })
        .collect();
    let list = if visible.is_empty() {
        "(none)".into()
    } else {
        visible.join("\n")
    };
    let routing = routing(tools);
    if routing.is_empty() {
        format!("Available tools:\n{list}")
    } else {
        format!("Available tools:\n{list}\n\n{routing}")
    }
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn skills(skills: &[PromptSkill]) -> String {
    let visible: Vec<_> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines=vec!["\n\nThe following skills provide specialized instructions for specific tasks.".into(),"AG1 deterministically embeds a skill when its activation trigger matches the user request; an embedded skill block is already loaded and must not be read again.".into(),"If no skill was embedded but a listed skill clearly applies, use the read tool to load its file before other task tools.".into(),"When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".into(),"".into(),"<available_skills>".into()];
    for s in visible {
        lines.extend([
            "  <skill>".into(),
            format!("    <name>{}</name>", xml(&s.name)),
            format!("    <description>{}</description>", xml(&s.description)),
            format!("    <location>{}</location>", xml(&s.file_path)),
            "  </skill>".into(),
        ]);
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}
pub fn build(input: &PromptInput) -> Result<String, DomainError> {
    if chrono::NaiveDate::parse_from_str(&input.date, "%Y-%m-%d").is_err()
        || input.cwd.is_empty()
        || input.cwd.contains(['\r', '\n', '\0'])
    {
        return Err(invalid("captured prompt date/workspace"));
    }
    let default = vec!["read".into(), "bash".into(), "edit".into(), "write".into()];
    let tools = input.selected_tools.as_ref().unwrap_or(&default);
    let has = |s: &str| tools.iter().any(|t| t == s);
    let catalog = catalog(tools, &input.tool_snippets);
    let mut prompt = if let Some(custom) = input.custom_prompt.as_ref().filter(|s| !s.is_empty()) {
        let mut p = custom.clone();
        let lower = custom.to_lowercase();
        let pattern =
            regex::Regex::new(r"(?-u:\b)(?:available tools:|tool routing(?-u:\b))").unwrap();
        if !pattern.is_match(&lower) {
            p.push_str(&format!("\n\n{catalog}"));
        }
        p
    } else {
        let mut guidelines = vec![];
        if has("bash") && !["grep", "find", "ls"].iter().any(|s| has(s)) {
            guidelines.push("Use bash for file operations such as ls, rg, and find".into());
        } else if has("bash") {
            guidelines.push("Prefer grep/find/ls tools over bash for file exploration (faster, respects .gitignore)".into());
        }
        if has("webSearch") && has("webFetch") {
            guidelines.extend(["For online research, use webSearch for candidate sources and webFetch for the best URL".into(),"Prefer authoritative sources such as official docs, vendor docs, specs, and release notes".into(),"Keep source URLs available when online information affects the answer".into()]);
        } else if has("webSearch") {
            guidelines.push("Use webSearch when the task depends on current online information instead of guessing".into());
        }
        for g in input.prompt_guidelines.iter().map(|s| s.trim()).chain([
            "Be concise in your responses",
            "Show file paths clearly when working with files",
        ]) {
            if !g.is_empty() && !guidelines.iter().any(|s| s == g) {
                guidelines.push(g.to_string());
            }
        }
        let mutating = ["bash", "edit", "write"].iter().any(|s| has(s));
        let readonly = ["read", "grep", "find", "ls", "webSearch", "webFetch"]
            .iter()
            .any(|s| has(s));
        let role = match (compact(input.model_provider.as_deref()), mutating, readonly) {
            (true, true, _) => {
                "You are a coding assistant inside AG1. Use the named tools, inspect evidence, and answer precisely."
            }
            (true, false, true) => {
                "You are a coding assistant inside AG1. Use the named read-only/research tools and answer precisely."
            }
            (true, false, false) => {
                "You are a coding assistant inside AG1. Answer precisely using only the available context."
            }
            (false, true, _) => {
                "You are an expert coding assistant inside AG1. Use the available tools to inspect code, run commands, edit files, and answer precisely."
            }
            (false, false, true) => {
                "You are an expert coding assistant inside AG1. Use the available read-only and research tools to inspect evidence and answer precisely."
            }
            _ => {
                "You are an expert coding assistant inside AG1. Answer precisely using only the available context."
            }
        };
        let mut p = format!(
            "{role}\n\n{catalog}\n\nGuidelines:\n{}",
            guidelines
                .iter()
                .map(|g| format!("- {g}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        if let Some(d) = &input.ag1_docs {
            p.push_str(&format!("\n\n{}",if compact(input.model_provider.as_deref()){format!("AG1 docs only for AG1-specific work: README {} | Docs {} | Examples {}",d.readme,d.docs,d.examples)}else{format!("AG1 docs: only for AG1-specific work.\nREADME {}\nDocs {}\nExamples {}\nFollow markdown cross-references before implementing.",d.readme,d.docs,d.examples)}));
        }
        p
    };
    if let Some(append) = &input.append_system_prompt
        && !append.is_empty()
    {
        prompt.push_str(&format!("\n\n{append}"));
    }
    if !input.context_files.is_empty() {
        prompt.push_str("\n\n# Project Context\n\nLoaded context files:\n\n");
        for file in &input.context_files {
            prompt.push_str(&format!("## {}\n\n{}\n\n", file.path, file.content));
        }
    }
    if has("read") {
        prompt.push_str(&skills(&input.skills));
    }
    prompt.push_str(&format!(
        "\nCurrent date: {}\nCurrent working directory: {}",
        input.date,
        input.cwd.replace('\\', "/")
    ));
    if let Some(snapshot) = &input.workspace_snapshot
        && !snapshot.is_empty()
    {
        prompt.push_str(&format!(
            "\nTop-level workspace entries (snapshot at session start):\n{snapshot}"
        ));
    }
    if prompt.len() > 4 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("system prefix".into()));
    }
    Ok(prompt)
}
fn short(v: &Value) -> String {
    format!("{:x}", Sha256::digest(v.to_string().as_bytes()))[..16].into()
}
pub fn prefix_shape(system: &str, tools: &[Value], rewrite: u64) -> Value {
    let mut tools: Vec<_> = tools
        .iter()
        .map(|t| {
            let mut o = json!({"name":t["name"]});
            for k in ["description", "parameters"] {
                if let Some(v) = t.get(k) {
                    o[k] = v.clone();
                }
            }
            o
        })
        .collect();
    tools.sort_by(|a, b| {
        string(a, "name")
            .cmp(string(b, "name"))
            .then_with(|| string(a, "description").cmp(string(b, "description")))
            .then_with(|| {
                a.get("parameters")
                    .map(Value::to_string)
                    .unwrap_or("undefined".into())
                    .cmp(
                        &b.get("parameters")
                            .map(Value::to_string)
                            .unwrap_or("undefined".into()),
                    )
            })
    });
    let encoded = json!(tools).to_string();
    json!({"systemHash":short(&json!(system)),"toolsHash":short(&json!(encoded)),"prefixHash":short(&json!({"system":system,"tools":encoded})),"logRewriteVersion":rewrite,"toolSchemaTokens":units(&encoded).div_ceil(3)})
}
