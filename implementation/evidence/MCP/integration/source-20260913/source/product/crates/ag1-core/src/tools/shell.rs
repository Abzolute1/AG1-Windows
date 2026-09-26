//! Shell guards and output presentation. These heuristics do not provide OS containment.
use super::truncate::{self, Limits};
use crate::{DomainError, task::units};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
fn re(pattern: &str) -> Regex {
    Regex::new(&pattern.replace(r"\b", r"(?-u:\b)")).expect("audited shell pattern")
}
fn normalize(command: &str) -> String {
    command
        .split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn static_guard(command: &str) -> Option<String> {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"(?i)\bpython3?\b[^\n;&|]*\b-m\s+http\.server\b",
            r"(?i)\bpython3?\b[^\n;&|]*\b-m\s+SimpleHTTPServer\b",
            r"(?i)\bpython3?\b[^\n;&|]*\b-m\s+RangeHTTPServer\b",
            r"\bSimpleHTTPServer\b",
            r"\bhttp\.server(?:\.ThreadingHTTPServer)?\b",
            r"\bphp\b[^\n;&|]*\s-S\b",
            r"(?i)\bnpx\b[^\n;&|]*\b(?:serve|http-server|live-server)\b",
            r"(?i)\bbusybox\b[^\n;&|]*\bhttpd\b",
            r"(?i)\bruby\b[^\n;&|]*\b-run\b[^\n;&|]*\bhttpd\b",
            r"(?i)\bcaddy\b[^\n;&|]*\bfile-server\b",
        ]
        .map(re)
        .into()
    });
    PATTERNS.iter().any(|p|p.is_match(&normalize(command))).then(|| "Refused: do not start a static file server just to take screenshots. Browser verification requires a ready AG1-owned web target started through preview start or the host Runtime UI; bash/job servers and local HTML paths do not grant browser access. Use read to preview local images, including simple vector SVG. Use the editor Preview for static HTML. Start a server only for apps that actually need one (Vite, Next, an API).".into())
}
pub fn archive_guard(command: &str) -> Option<String> {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"\bmkdir\b[^\n;&|]*\b[Ss]napshots?\b",
            r"(?i)\b(screenshot|verify_ui|png|jpeg|webp)\b",
            r"(?i)\b(cp|mv|install|curl|wget|convert|magick)\b",
            r"\b[Ss]napshots?\b",
        ]
        .map(re)
        .into()
    });
    let command = normalize(command);
    if PATTERNS[0].is_match(&command) {
        Some("Refused: verification screenshots are already in the transcript. Do not create a Snapshots folder. Look at the image and continue.".into())
    } else if PATTERNS[1..].iter().all(|p| p.is_match(&command)) {
        Some("Refused: do not archive verification screenshots. The image is already in the transcript — judge it and keep working.".into())
    } else {
        None
    }
}
pub fn wait_guard(command: &str) -> Option<String> {
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?i)(?:^|[;&|]\s*)sleep\s+(?:--\s*)?(\d+(?:\.\d+)?)\b"));
    let command = normalize(command);
    let captures = PATTERN.captures(&command)?;
    let seconds = captures[1].parse::<f64>().ok()?;
    (seconds.is_finite()&&seconds>=15.).then(||format!("Refused: do not bash-sleep ({seconds}s) to wait on a background command. Use job wait with the job id (optional timeoutSeconds). job poll/log if you only need a status snapshot."))
}
pub fn install_guard(command: &str) -> Option<String> {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [r"(?i)\b(?:apt-get|apt|pacman|dnf|yum|zypper|apk|brew|nix-env|flatpak)\b[\s\S]{0,80}(?:\binstall\b|--install|-S\b)",r"(?i)\b(?:dotnet(?:-sdk)?|gcc|g\+\+|clang(?:\+\+)?|rustc|rustup|cargo|golang|mypy|pyright|typescript|openjdk|jdk|cmake|ninja)\b",r"(?i)\b(?:pip3?|python3?\s+-m\s+pip)\s+install\b[\s\S]{0,80}\b(?:mypy|pyright|dotnet)\b"].map(re).into()
    });
    let command = normalize(command);
    (PATTERNS[2].is_match(&command)||(PATTERNS[0].is_match(&command)&&PATTERNS[1].is_match(&command))).then(||"Refused: do not install compilers or SDKs from the agent (apt/pacman/pip/brew). Read the source files check pointed at and fix the error there. If the toolchain is missing, tell the user — do not try to provision the machine.".into())
}
pub fn guard(command: &str, job: bool) -> Result<(), DomainError> {
    if command.len() > 60 * 1024 || command.contains('\0') {
        return Err(DomainError::LimitExceeded("shell command".into()));
    }
    let reason = static_guard(command)
        .or_else(|| archive_guard(command))
        .or_else(|| (!job).then(|| wait_guard(command)).flatten())
        .or_else(|| (!job).then(|| install_guard(command)).flatten());
    reason.map_or(Ok(()), |reason| Err(DomainError::InvalidInput(reason)))
}
/// Literal outer-shell destinations for advisory post-command file hints.
/// This is not an exhaustive effect tracker: do not interpret quoted programs,
/// substitutions or here-document bodies as commands of the outer shell.
pub fn write_targets(command: &str) -> Vec<String> {
    let bytes = command.as_bytes();
    let mut targets = vec![];
    let (mut at, mut quote, mut here_document) = (0, None, false);
    while at < bytes.len() {
        let byte = bytes[at];
        if quote == Some(b'\'') {
            if byte == b'\'' {
                quote = None;
            }
            at += 1;
            continue;
        }
        if byte == b'\\' {
            at = (at + 2).min(bytes.len());
            continue;
        }
        // Expansion bodies can contain nested quotes and literal `>` bytes.
        // Their grammar cannot be inferred from the outer quote state.
        if bytes[at..].starts_with(b"$(")
            || bytes[at..].starts_with(b"${")
            || bytes[at..].starts_with(b"$'")
            || bytes[at..].starts_with(b"$\"")
            || byte == b'`'
        {
            break;
        }
        if let Some(delimiter) = quote {
            if byte == delimiter {
                quote = None;
            }
            at += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            at += 1;
            continue;
        }
        if here_document && byte == b'\n' {
            break;
        }
        if byte == b'#'
            && (at == 0 || bytes[at - 1].is_ascii_whitespace() || b";|&()".contains(&bytes[at - 1]))
        {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        // These constructs need a shell grammar to distinguish comparisons and
        // nested programs from redirects. Omitting an advisory hint is safer
        // than inventing a destination. Nothing here authorizes execution.
        if bytes[at..].starts_with(b"<(")
            || bytes[at..].starts_with(b">(")
            || bytes[at..].starts_with(b"((")
            || bytes[at..].starts_with(b"[[")
        {
            break;
        }
        if bytes[at..].starts_with(b"<<") {
            here_document = true;
        }
        if byte != b'>' {
            at += 1;
            continue;
        }
        let append = bytes.get(at + 1) == Some(&b'>');
        let prefixed = at > 0 && (bytes[at - 1].is_ascii_digit() || bytes[at - 1] == b'&');
        at += if append { 2 } else { 1 };
        let (target, end) = literal_redirect_target(bytes, at);
        at = end;
        // Preserve the existing descriptor-redirection scope of these hints.
        if !append && prefixed {
            continue;
        }
        if let Some(target) = target
            && target != "/dev/null"
            && !targets.contains(&target)
        {
            targets.push(target);
        }
    }
    targets
}

fn literal_redirect_target(bytes: &[u8], mut at: usize) -> (Option<String>, usize) {
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let (mut text, mut quote, mut dynamic) = (vec![], None, false);
    while let Some(&byte) = bytes.get(at) {
        if quote == Some(b'\'') {
            if byte == b'\'' {
                quote = None;
            } else {
                text.push(byte);
            }
        } else if byte == b'\\' {
            let Some(&next) = bytes.get(at + 1) else {
                return (None, bytes.len());
            };
            if quote != Some(b'"') || b"$`\"\\\n".contains(&next) {
                at += 1;
                if next != b'\n' {
                    text.push(next);
                }
            } else {
                text.push(byte);
            }
        } else if quote == Some(b'"') {
            if byte == b'"' {
                quote = None;
            } else {
                if matches!(byte, b'$' | b'`') {
                    return (None, bytes.len());
                }
                text.push(byte);
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte.is_ascii_whitespace() || b";&|<>()".contains(&byte) {
            break;
        } else {
            if matches!(byte, b'$' | b'`') {
                return (None, bytes.len());
            }
            dynamic |= matches!(byte, b'*' | b'?' | b'[' | b'~');
            text.push(byte);
        }
        at += 1;
    }
    let target = (!dynamic && quote.is_none() && !text.is_empty())
        .then(|| String::from_utf8(text).ok())
        .flatten();
    (target, at)
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn sandbox_template(template: &str, command: &str, cwd: &str) -> Result<String, DomainError> {
    if template.len() + command.len() + cwd.len() > 60 * 1024 {
        return Err(DomainError::LimitExceeded("shell template".into()));
    }
    let rendered = template
        .replace("{command}", &quote(command))
        .replace("{cwd}", &quote(cwd));
    Ok(if template.contains("{command}") {
        rendered
    } else {
        format!("{rendered} {}", quote(command))
    })
}
/// ANSI pattern adapted from ansi-regex 6.2.2 (MIT), see product/licenses/ansi-regex-port.
pub fn sanitize(raw: &[u8]) -> Result<String, DomainError> {
    if raw.len() > 64 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("shell output".into()));
    }
    static ANSI: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:\x1b\][\s\S]*?(?:\x07|\x1b\\|\x{009C}))|[\x1b\x{009B}][\[\]()#;?]*(?:[0-9]{1,4}(?:[;:][0-9]{0,4})*)?[0-9A-PR-TZcf-nq-uy=><~]").unwrap()
    });
    let decoded = String::from_utf8_lossy(raw);
    Ok(ANSI
        .replace_all(&decoded, "")
        .chars()
        .filter(|c| {
            *c == '\t' || *c == '\n' || (*c > '\u{1f}' && !matches!(*c, '\u{fff9}'..='\u{fffb}'))
        })
        .collect())
}
#[derive(Clone, Copy, Debug)]
pub enum Exit {
    Code(i32),
    Signal,
    Cancelled,
    Timeout(f64),
    OutputLimit,
}
pub struct FullOutput<'a> {
    pub id: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
}
fn size(n: usize) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}KB", n as f64 / 1024.)
    } else {
        format!("{:.1}MB", n as f64 / (1024. * 1024.))
    }
}
pub fn render(
    command: &str,
    raw: &[u8],
    exit: Exit,
    artifact: Option<FullOutput<'_>>,
) -> Result<Value, DomainError> {
    static INSPECTION: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"(^|[;&|]\s*)(ls\b|find\b|fd\b|rg\s+--files\b|pwd\b|git\s+status(\s|$)|git\s+branch(\s|$)|which\b)",
        )
    });
    let full = sanitize(raw)?;
    let mut output = full.clone();
    if matches!(exit, Exit::Code(0)) && INSPECTION.is_match(&normalize(command)) {
        let count = full
            .split('\n')
            .filter(|s| !s.trim_end().is_empty())
            .count();
        if count > 40 {
            output = format!(
                "{}\n\n[{} more lines omitted from inspection output]",
                full.split('\n')
                    .map(str::trim_end)
                    .filter(|s| !s.is_empty())
                    .take(25)
                    .collect::<Vec<_>>()
                    .join("\n"),
                count - 25
            );
        }
    }
    let truncation = truncate::truncate(&output, Limits::default(), true);
    let mut text = if truncation.content.is_empty() {
        "(no output)".into()
    } else {
        truncation.content.clone()
    };
    let mut details = serde_json::Map::new();
    if let Some(artifact) = &artifact {
        details.insert(
            "fullOutputArtifact".into(),
            json!({"id":artifact.id,"bytes":artifact.bytes,"sha256":artifact.sha256}),
        );
    }
    if truncation.truncated {
        let start = truncation.total_lines - truncation.output_lines + 1;
        let end = truncation.total_lines;
        let notice = if truncation.last_line_partial {
            format!(
                "[last {} of line {end}; line size {}]",
                size(truncation.output_bytes),
                size(output.rsplit('\n').next().unwrap_or("").len())
            )
        } else if truncation.truncated_by.as_deref() == Some("lines") {
            format!("[tail lines {start}-{end}/{end}]")
        } else {
            format!("[tail lines {start}-{end}/{end}; byte limit hit]")
        };
        text.push_str(&format!("\n\n{notice}"));
        if let Some(artifact) = &artifact {
            text.push_str(&format!(" [full output artifact: {}]", artifact.id));
        }
        details.insert("truncation".into(), json!(truncation));
    } else if let Some(artifact) = &artifact {
        text.push_str(&format!(
            "\n\n[complete pre-compaction output: artifact {}]",
            artifact.id
        ));
    }
    let error = !matches!(exit, Exit::Code(0));
    match exit {
        Exit::Code(0) => {}
        Exit::Code(code) => text.push_str(&format!("\n\nCommand exited with code {code}")),
        Exit::Signal => text.push_str("\n\nCommand was killed by a signal"),
        Exit::Cancelled | Exit::Timeout(_) => {
            text = full;
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&match exit {
                Exit::Cancelled => "Command aborted".into(),
                Exit::Timeout(seconds) => format!("Command timed out after {seconds} seconds"),
                _ => unreachable!(),
            });
            if let Some(artifact) = &artifact {
                text.push_str(&format!("\n\n[full output artifact: {}]", artifact.id));
            }
        }
        Exit::OutputLimit => {
            text.push_str("\n\nCommand stopped at the output quota; retained output is incomplete")
        }
    }
    // B01 applies its aggregate budget after this adapter projection, including error output.
    let mut result = json!({"content":[{"type":"text","text":text}]});
    if !details.is_empty() {
        result["details"] = Value::Object(details);
    }
    if error {
        result["isError"] = json!(true);
    }
    Ok(result)
}
pub fn job_command(command: &str) -> String {
    if units(command) <= 80 {
        command.into()
    } else {
        format!(
            "{}...",
            String::from_utf16_lossy(&command.encode_utf16().take(77).collect::<Vec<_>>())
        )
    }
}
