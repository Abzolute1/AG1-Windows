//! Mode policy is a product guardrail. Shell/extension privileges require explicit host containment.
use crate::{DomainError, context::slice_utf16};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::LazyLock;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Disabled,
    Interactive,
    Planning,
    Autonomous,
    Headless,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Interactive => "interactive",
            Self::Planning => "planning",
            Self::Autonomous => "autonomous",
            Self::Headless => "headless",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub mode: Mode,
    pub cwd: String,
    #[serde(default)]
    pub planning_read_only_tools: Option<String>,
    #[serde(default)]
    pub trusted_tools: Vec<String>,
    #[serde(default)]
    pub extension_tools: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub allow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_request: Option<Value>,
}
fn yes() -> Decision {
    Decision {
        allow: true,
        reason: None,
        approval_request: None,
    }
}
fn no(reason: impl Into<String>) -> Decision {
    Decision {
        allow: false,
        reason: Some(reason.into()),
        approval_request: None,
    }
}
fn ask(policy: &Policy, tool: &str, risk: &str, summary: String, details: String) -> Decision {
    Decision {
        allow: true,
        reason: None,
        approval_request: Some(
            json!({"mode":policy.mode.name(),"toolName":tool,"risk":risk,"summary":summary,"details":details}),
        ),
    }
}
fn display(s: &str, cap: usize) -> String {
    let s = s.trim();
    if s.encode_utf16().count() <= cap {
        s.into()
    } else {
        format!(
            "{}...\n[truncated — full command will execute]",
            slice_utf16(s, cap.saturating_sub(3))
        )
    }
}
pub fn dialog(request: &Value) -> Value {
    let details = request["details"].as_str().unwrap_or("").trim();
    json!({"title":request["summary"],"message":if request["risk"]=="critical" && !details.is_empty() && !details.to_lowercase().contains("outside"){format!("{details}\n\nThis is outside the current project.")}else{details.into()}})
}
const READ: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "repo",
    "sourceContext",
    "artifact",
    "editPreview",
    "diff",
];
const RESEARCH: &[&str] = &["webSearch", "webFetch"];
const SUPPORT: &[&str] = &[
    "runtimeObservation",
    "check",
    "verify",
    "verify_ui",
    "screenshot_url",
    "interact_then_screenshot",
    "todo",
    "memory",
    "taskState",
];
const HEADLESS: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "repo",
    "sourceContext",
    "artifact",
    "memory",
    "taskState",
    "todo",
    "job",
    "check",
    "verify",
    "webSearch",
    "webFetch",
    "task",
    "explore",
    "screenshot_url",
    "interact_then_screenshot",
    "verify_ui",
];
const GUIDANCE: &str = "Use read, grep, find, ls, repo, webSearch, or webFetch instead.";
pub fn headless(active: &[String], available: &[String]) -> Vec<String> {
    let selected: Vec<_> = active
        .iter()
        .filter(|n| HEADLESS.contains(&n.as_str()) && available.contains(n))
        .cloned()
        .collect();
    if selected.is_empty() {
        HEADLESS
            .iter()
            .filter(|n| available.iter().any(|v| v == *n))
            .map(|n| n.to_string())
            .collect()
    } else {
        selected
    }
}
/// Narrow capability list for read-only subagents; mutation and delegation never enter the registry.
pub fn read_only_profile(available: &[String]) -> Vec<String> {
    available
        .iter()
        .filter(|n| READ.contains(&n.as_str()) || RESEARCH.contains(&n.as_str()))
        .cloned()
        .collect()
}
fn regex(pattern: &str, flags: &str) -> Regex {
    Regex::new(&format!(
        "{}{}",
        if flags.contains('i') { "(?i)" } else { "" },
        pattern.replace(r"\b", r"(?-u:\b)")
    ))
    .expect("audited static policy regex")
}
static PATTERNS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../resources/shell-policy-patterns.json")).unwrap()
});
static FORBIDDEN: LazyLock<Vec<(Regex, String)>> = LazyLock::new(|| {
    PATTERNS["forbidden"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                regex(r["pattern"].as_str().unwrap(), r["flags"].as_str().unwrap()),
                r["reason"].as_str().unwrap().into(),
            )
        })
        .collect()
});
static DESTRUCTIVE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    PATTERNS["destructive"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| !r["pattern"].as_str().unwrap().contains("(?!"))
        .map(|r| regex(r["pattern"].as_str().unwrap(), r["flags"].as_str().unwrap()))
        .collect()
});
static SAFE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    PATTERNS["safe"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| regex(r["pattern"].as_str().unwrap(), r["flags"].as_str().unwrap()))
        .collect()
});
static REDIRECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\d*|&)>\s*(?:&\d+\s*)?/dev/null|\d*>\s*&\d+").unwrap());
static SEGMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&&|\|\||[;|]").unwrap());
static MUTATING_GIT: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"\bgit\s+(add|commit|push|pull|merge|rebase|reset|checkout|switch|restore|cherry-pick|revert|tag|init|clone|clean|apply|am|bisect\s+(start|good|bad|reset)|worktree\s+(add|remove)|branch\s+-[dD]|branch\s+-m)",
        "i",
    )
});
static GIT_SPECIAL: LazyLock<Regex> =
    LazyLock::new(|| regex(r"\bgit\s+(stash|config)\s+(\S*)", "i"));
/// Matches the reference heuristic; it is deliberately not described as an OS sandbox.
pub fn planning_shell(command: &str) -> bool {
    if command.trim().is_empty()
        || command.contains("$(")
        || command.contains('`')
        || command.contains("${")
    {
        return false;
    }
    if REDIRECT.replace_all(command, "").contains('>')
        || DESTRUCTIVE.iter().any(|r| r.is_match(command))
        || MUTATING_GIT.is_match(command)
    {
        return false;
    }
    for c in GIT_SPECIAL.captures_iter(command) {
        let allowed = if c[1].eq_ignore_ascii_case("stash") {
            "list"
        } else {
            "--get"
        };
        let token = c[2].to_lowercase();
        if !token.starts_with(allowed)
            || token[allowed.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return false;
        }
    }
    SEGMENT
        .split(command)
        .all(|s| !s.trim().is_empty() && SAFE.iter().any(|r| r.is_match(s.trim())))
}
/// Lexical resolution for display/policy. Host workspace capabilities still validate symlinks/inodes.
pub fn resolve(path: &str, cwd: &str) -> String {
    let combined = if path.starts_with('/') {
        path.into()
    } else {
        format!("{cwd}/{path}")
    };
    let mut parts = vec![];
    for p in combined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}
pub fn contained(path: &str, cwd: &str) -> bool {
    let root = resolve(cwd, "/");
    let path = resolve(path, &root);
    root == "/" || path == root || path.starts_with(&(root + "/"))
}
fn home(s: &str) -> bool {
    ["~", "$HOME", "${HOME}"]
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}/")))
}
fn trim_token(s: &str) -> &str {
    s.trim_start_matches(['(', '"', '\'', '`'])
        .trim_end_matches([')', '"', '\'', '`', ',', ':', ';'])
}
static PATH_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r##"(?:^|[\s=:(<>'\["`])((?:/|~(?:/|$)|(?:\$HOME|\$\{HOME\})(?:/|$)|(?:\.\./)+)[^\s"'`|&;<>)]*)"##).unwrap()
});
static CD: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r##"(?:^|[\s;&|])cd\b(?:\s+("[^"]*"|'[^']*'|[^\s;&|"']+))?"##,
        "",
    )
});
fn outside_shell(command: &str, cwd: &str, folders: &[String]) -> Option<String> {
    let contained_here =
        |path: &str| contained(path, cwd) || folders.iter().any(|root| contained(path, root));
    for c in PATH_TOKEN.captures_iter(command) {
        let raw = c.get(1).expect("path capture");
        // The legacy token heuristic splits on spaces. Recognize a complete
        // quoted literal in a host-granted spaced path, without accepting a
        // concatenated suffix such as '/granted folder'-outside.
        let quoted = raw.start().checked_sub(1).and_then(|start| {
            let quote = command.as_bytes()[start];
            if !matches!(quote, b'\'' | b'"') {
                return None;
            }
            let rest = &command[raw.start()..];
            let end = rest.find(quote as char)?;
            let path = &rest[..end];
            let suffix = &rest[end + 1..];
            if !suffix.is_empty()
                && !suffix.starts_with(|c: char| c.is_whitespace() || ";|&<>".contains(c))
            {
                return None;
            }
            folders
                .iter()
                .any(|folder| folder.contains(' ') && contained(path, folder))
                .then_some(path)
        });
        let token = trim_token(quoted.unwrap_or(raw.as_str()));
        if token.is_empty() || token == "/" {
            continue;
        }
        if home(token) {
            return Some(token.into());
        }
        if token.starts_with('/') {
            if token == "/dev/null" {
                continue;
            }
            let resolved = resolve(token, "/");
            if [
                "/bin",
                "/usr/bin",
                "/usr/local/bin",
                "/nix/store",
                "/opt/homebrew/bin",
                "/snap/bin",
            ]
            .iter()
            .any(|p| resolved == *p || resolved.starts_with(&format!("{p}/")))
            {
                continue;
            }
            if !contained_here(token) {
                return Some(token.into());
            }
        }
        if token.starts_with("../") && !contained_here(&resolve(token, cwd)) {
            return Some(token.into());
        }
    }
    for c in CD.captures_iter(command) {
        let Some(raw) = c.get(1) else {
            return Some("~".into());
        };
        let token = trim_token(raw.as_str());
        if home(token) || token.starts_with('/') && !contained_here(token) {
            return Some(token.into());
        }
    }
    None
}
fn file(tool: &str, args: &Value, p: &Policy) -> Decision {
    let path = args
        .get("path")
        .filter(|v| !v.is_null())
        .or_else(|| args.get("file_path"))
        .and_then(Value::as_str);
    let verb = if [
        "edit",
        "editApply",
        "append",
        "notebook_edit",
        "NotebookEdit",
    ]
    .contains(&tool)
    {
        "Edit"
    } else {
        "Create or overwrite"
    };
    if let Some(path) = path
        && !contained(path, &p.cwd)
    {
        let absolute = resolve(path, &p.cwd);
        return if p.mode == Mode::Interactive {
            ask(
                p,
                tool,
                "critical",
                format!("{verb} a file outside this project"),
                format!(
                    "File: {path}\nWhich resolves to: {absolute}\nProject: {}",
                    p.cwd
                ),
            )
        } else {
            no(format!(
                "Blocking {tool} outside the session workspace in {} mode: {absolute}",
                p.mode.name()
            ))
        };
    }
    match p.mode {
        Mode::Autonomous => yes(),
        Mode::Interactive => ask(
            p,
            tool,
            "high",
            format!("{verb} {}", path.unwrap_or("a file")),
            format!("File: {}", path.unwrap_or("(unknown path)")),
        ),
        Mode::Planning => no(format!(
            "{tool} is blocked in planning mode. Planning mode is read-only and does not allow file edits."
        )),
        _ => no(format!(
            "{tool} requires interactive approval and is blocked in headless mode. {GUIDANCE}"
        )),
    }
}
fn bash(args: &Value, p: &Policy, folders: &[String]) -> Decision {
    let Some(command) = args["command"].as_str().filter(|s| !s.is_empty()) else {
        return if p.mode == Mode::Interactive {
            yes()
        } else {
            no("Bash command is required.")
        };
    };
    for (pattern, reason) in FORBIDDEN.iter() {
        if pattern.is_match(command) {
            return no(reason.clone());
        }
    }
    if [Mode::Autonomous, Mode::Headless].contains(&p.mode)
        && let Some(path) = outside_shell(command, &p.cwd, folders)
    {
        return no(format!(
            "Blocking bash access outside the session workspace in {} mode: {path}",
            p.mode.name()
        ));
    }
    match p.mode {
        Mode::Autonomous => yes(),
        Mode::Interactive => ask(
            p,
            "bash",
            "high",
            "Run this command".into(),
            format!("{}\n\nin {}", display(command, 600), p.cwd),
        ),
        Mode::Planning => {
            if planning_shell(command) {
                yes()
            } else {
                no(
                    "Only read-only shell commands are allowed in planning mode (e.g. git status/log/diff, cat, grep, wc, jq). Mutating commands, redirection, and command substitution are blocked.",
                )
            }
        }
        _ => no(format!(
            "Bash execution requires interactive approval and is blocked in headless mode. {GUIDANCE}"
        )),
    }
}
pub fn evaluate(
    tool: &str,
    args: &Value,
    p: &Policy,
    untrusted_extensions: bool,
) -> Result<Decision, DomainError> {
    evaluate_in_folders(tool, args, p, untrusted_extensions, &[])
}
/// Additional directories are captured host capabilities, never model arguments.
/// This remains a policy heuristic; the host must enforce filesystem effects.
pub fn evaluate_in_folders(
    tool: &str,
    args: &Value,
    p: &Policy,
    untrusted_extensions: bool,
    folders: &[String],
) -> Result<Decision, DomainError> {
    if folders.len() > 16
        || folders
            .iter()
            .any(|p| !p.starts_with('/') || p.len() > 4096 || p.contains('\0'))
    {
        return Err(DomainError::InvalidInput("host folder policy".into()));
    }
    if !p.cwd.starts_with('/')
        || p.cwd.contains('\0')
        || p.trusted_tools.len() > 256
        || p.extension_tools.len() > 256
        || p.planning_read_only_tools
            .as_ref()
            .is_some_and(|s| !["allow", "block"].contains(&s.as_str()))
    {
        return Err(DomainError::InvalidInput("captured tool policy".into()));
    }
    if tool == "capability" && matches!(args["action"].as_str(), Some("list" | "schema")) {
        return Ok(yes());
    }
    // Explicit user-selected bypass suppresses GitHub tool approval prompts.
    // Trusted-tool lists and autonomous mode still require confirmation;
    // native preparation/execution retain account, target and scope fences.
    if tool == "github" {
        return Ok(
            if ["catalog", "schema"].contains(&args["operation"].as_str().unwrap_or(""))
                || p.mode == Mode::Disabled
            {
                yes()
            } else if matches!(p.mode, Mode::Planning | Mode::Headless) {
                no("GitHub account operations require interactive confirmation.")
            } else {
                ask(p,tool,"high","Review GitHub account operation".into(),"Review the native account, repository, request and consequences before approving.".into())
            },
        );
    }
    if tool == "preview" {
        return Ok(if args["action"] == "status" {
            yes()
        } else {
            match p.mode {
                Mode::Planning | Mode::Headless => {
                    no("Starting or stopping a preview requires execution mode.")
                }
                Mode::Interactive => ask(
                    p,
                    tool,
                    "high",
                    "Manage owned project preview".into(),
                    "Review the native target and launch command or exact operation to stop."
                        .into(),
                ),
                _ => yes(),
            }
        });
    }
    if p.mode == Mode::Disabled {
        return Ok(yes());
    }
    if p.mode == Mode::Planning
        && p.planning_read_only_tools.as_deref() == Some("block")
        && (READ.contains(&tool) || RESEARCH.contains(&tool))
    {
        return Ok(no(format!(
            "{tool} is blocked for this planning turn. Planning must clarify or plan first before exploring files or the web."
        )));
    }
    if tool == "edit" && args["mode"] == "preview" {
        return Ok(
            if p.mode == Mode::Planning && p.planning_read_only_tools.as_deref() == Some("block") {
                no("File inspection is blocked for this planning turn.")
            } else {
                yes()
            },
        );
    }
    if READ.contains(&tool)
        || RESEARCH.contains(&tool)
        || [
            "task",
            "explore",
            "todo",
            "memory",
            "taskState",
            "submit_plan",
        ]
        .contains(&tool)
    {
        return Ok(yes());
    }
    if [
        "write",
        "edit",
        "editApply",
        "append",
        "notebook_edit",
        "NotebookEdit",
    ]
    .contains(&tool)
    {
        return Ok(file(tool, args, p));
    }
    if tool == "bash" {
        return Ok(bash(args, p, folders));
    }
    if tool == "episode"
        && (["list", "search", "show"].contains(&args["action"].as_str().unwrap_or(""))
            || [Mode::Interactive, Mode::Autonomous].contains(&p.mode))
    {
        return Ok(yes());
    }
    if tool == "job" {
        return Ok(
            if ["list", "poll", "log"].contains(&args["action"].as_str().unwrap_or("")) {
                yes()
            } else {
                bash(
                    &json!({"command":args["command"].as_str().unwrap_or("job")}),
                    p,
                    folders,
                )
            },
        );
    }
    if SUPPORT.contains(&tool) && p.mode == Mode::Autonomous
        || p.trusted_tools.iter().any(|n| n == tool)
    {
        return Ok(yes());
    }
    Ok(match p.mode {
        Mode::Interactive => ask(
            p,
            tool,
            "high",
            format!("Run tool {tool}"),
            format!(
                "This is not a built-in AG1 tool.\n\n{}",
                display(&args.to_string(), 800)
            ),
        ),
        Mode::Planning => no(format!(
            "{tool} is blocked in planning mode because it is not a built-in read-only planning tool."
        )),
        Mode::Headless => no(format!(
            "{tool} is not covered by the headless safety policy and is blocked in headless mode. Add it to trustedTools only if the embedding runtime has audited it."
        )),
        Mode::Autonomous => {
            if !untrusted_extensions && p.extension_tools.iter().any(|n| n == tool) {
                yes()
            } else {
                no(format!(
                    "{tool} is not covered by the autonomous safety policy and is blocked in autonomous mode. Add it to trustedTools only if the runtime has audited it for autonomous execution."
                ))
            }
        }
        Mode::Disabled => yes(),
    })
}
