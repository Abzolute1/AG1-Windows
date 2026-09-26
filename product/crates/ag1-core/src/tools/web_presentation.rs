//! Bounded web-result presentation, shared by native clients independently of
//! execution definitions. Text semantics port the pinned ToolExecutionComponent;
//! terminal framing, colors and interaction belong to the client.
use crate::DomainError;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct Presentation {
    pub heading: String,
    pub input: String,
    pub summary: String,
    pub body: String,
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}
fn coverage(details: &Value) -> (usize, usize) {
    (
        array(&details["coverage"], "present").len(),
        array(&details["coverage"], "missing").len(),
    )
}
fn preview(text: &str, expanded: bool) -> String {
    let lines: Vec<_> = text
        .lines()
        .map(str::trim_end)
        .filter(|s| !s.trim().is_empty())
        .collect();
    let maximum = if expanded { 6 } else { 2 };
    let mut shown: Vec<_> = lines
        .iter()
        .take(maximum)
        .map(|line| {
            if line.encode_utf16().count() <= 160 {
                return (*line).to_owned();
            }
            let mut used = 0;
            let mut text: String = line
                .chars()
                .take_while(|c| {
                    used += c.len_utf16();
                    used <= 159
                })
                .collect();
            text.push('…');
            text
        })
        .collect();
    if !expanded && lines.len() > shown.len() {
        shown.push(format!("… {} more lines", lines.len() - shown.len()));
    }
    shown.join("\n")
}
pub fn render(
    tool: &str,
    args: &Value,
    result: &Value,
    expanded: bool,
) -> Result<Presentation, DomainError> {
    super::schema::bounded(args, 64 * 1024, 16)?;
    super::schema::bounded(result, 128 * 1024, 32)?;
    let details = &result["details"];
    if matches!(tool, "webSearch" | "webFetch")
        && (result["isError"] == true || details["error"].is_string())
    {
        let reason = details["error"]
            .as_str()
            .unwrap_or("The web request failed.");
        return Ok(Presentation {
            heading: tool.to_ascii_uppercase(),
            input: super::shell::sanitize(
                string(args, if tool == "webSearch" { "query" } else { "url" }).as_bytes(),
            )?,
            summary: if details["status"] == "blocked" {
                "Search blocked".into()
            } else {
                "Web request failed".into()
            },
            body: super::shell::sanitize(reason.as_bytes())?,
        });
    }

    let (input, summary, body) = match tool {
        "webSearch" => {
            let input = string(args, "query").to_owned();
            let rows = array(details, "results");
            let count = details["resultCount"].as_u64();
            let suffix = if count == Some(1) { "" } else { "s" };
            let provider = string(details, "searchProvider");
            let provider = if provider.is_empty() {
                String::new()
            } else {
                format!(" · {provider}")
            };
            let fallback = if details["fallbackUsed"] == true {
                " · fallback"
            } else {
                ""
            };
            let kind = rows
                .first()
                .map(|r| string(r, "sourceKind"))
                .unwrap_or_default();
            let kind = if kind.is_empty() {
                String::new()
            } else {
                format!(" · top {kind}")
            };
            let summary = count
                .map(|n| format!("Search returned {n} result{suffix}{provider}{fallback}{kind}"))
                .unwrap_or_else(|| "Search completed".into());
            let mut lines = vec![];
            if let Some(n) = count {
                lines.push(format!("{n} result{suffix} found{provider}{fallback}"));
            }
            let maximum = if expanded { 5 } else { 2 };
            for row in rows.iter().take(maximum) {
                let title = row["title"]
                    .as_str()
                    .or_else(|| row["url"].as_str())
                    .unwrap_or("Untitled result");
                let mut labels = vec![];
                let kind = string(row, "sourceKind");
                if !kind.is_empty() {
                    labels.push(kind.to_owned());
                }
                if row["authoritative"] == true {
                    labels.push("authoritative".into());
                }
                let risk = string(row, "fetchRisk");
                if !risk.is_empty() && risk != "low" {
                    labels.push(format!("{risk} fetch risk"));
                }
                let labels = if labels.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", labels.join(", "))
                };
                lines.push(format!("- {title}{labels}"));
            }
            if !expanded && rows.len() > maximum {
                lines.push(format!("+{} more results", rows.len() - maximum));
            }
            (input, summary, lines.join("\n"))
        }
        "webFetch" => {
            let input = string(args, "url").to_owned();
            let title = string(details, "title");
            let title_or_url = if title.is_empty() { &input } else { title };
            let status = string(details, "status");
            let (present, missing) = coverage(details);
            let mut suffix = vec![];
            if matches!(status, "placeholder" | "thin" | "rich") {
                suffix.push(status.to_owned());
            }
            if present + missing > 0 {
                suffix.push(format!("coverage {present}/{}", present + missing));
            }
            let summary = format!(
                "Fetched {title_or_url}{}",
                if suffix.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", suffix.join(" · "))
                }
            );
            let mut lines = vec![];
            if !title.is_empty() {
                lines.push(title.to_owned());
            }
            let mut status_line = vec![];
            match status {
                "placeholder" => status_line.push("placeholder/interstitial".into()),
                "thin" => status_line.push("thin extract".into()),
                "rich" => status_line.push("rich source".into()),
                _ => {}
            }
            if present + missing > 0 {
                let kind = string(&details["coverage"], "kind");
                let kind = if kind.is_empty() {
                    String::new()
                } else {
                    format!("{kind} ")
                };
                status_line.push(format!("{kind}coverage {present}/{}", present + missing));
            }
            if expanded && missing > 0 {
                status_line.push(format!(
                    "missing {}",
                    array(&details["coverage"], "missing")
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if expanded && !string(details, "statusReason").is_empty() {
                status_line.push(string(details, "statusReason").into());
            }
            if !status_line.is_empty() {
                lines.push(status_line.join(" · "));
            }
            lines.extend(
                array(details, "facts")
                    .iter()
                    .take(if expanded { 4 } else { 2 })
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
            let preview = preview(string(details, "preview"), expanded);
            if !preview.is_empty() {
                lines.push(preview);
            }
            if details["truncated"] == true && !expanded {
                lines.push("Preview truncated. Expand for more.".into());
            }
            (input, summary, lines.join("\n"))
        }
        _ => return Err(DomainError::InvalidInput("web presentation tool".into())),
    };
    if input.len() + summary.len() + body.len() > 64 * 1024 {
        return Err(DomainError::LimitExceeded("web presentation".into()));
    }
    Ok(Presentation {
        heading: tool.to_ascii_uppercase(),
        input: super::shell::sanitize(input.as_bytes())?,
        summary: super::shell::sanitize(summary.as_bytes())?,
        body: super::shell::sanitize(body.as_bytes())?,
    })
}
