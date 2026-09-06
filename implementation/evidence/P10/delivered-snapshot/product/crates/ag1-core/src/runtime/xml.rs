//! Pinned XML fallback syntax and request-tail recovery guidance. Parsed calls
//! are only candidates: the native owner must retain source, admit the batch
//! and apply the same tool policy/receipts as structured provider calls.
use crate::DomainError;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub name: String,
    pub id: String,
    pub arguments: BTreeMap<String, String>,
    pub raw_xml: String,
}
struct Tag<'a> {
    start: usize,
    end: usize,
    name: String,
    body: &'a str,
    raw: &'a str,
}
fn regex(pattern: &str) -> Result<Regex, DomainError> {
    Regex::new(pattern).map_err(|_| DomainError::InvalidInput("XML tool syntax".into()))
}
/// Emulates the SDK's non-greedy, case-insensitive matching backreference
/// without requiring a backtracking regex engine. Names and offsets retain the
/// original bytes; ASCII folding cannot change UTF-8 positions.
fn tags<'a>(
    text: &'a str,
    opening: &'a Regex,
    closing: impl Fn(&Captures<'_>) -> String + 'a,
) -> impl Iterator<Item = Tag<'a>> + 'a {
    let folded = text.to_ascii_lowercase();
    let mut offset = 0;
    std::iter::from_fn(move || {
        while let Some(capture) = opening.captures_at(text, offset) {
            let matched = capture.get(0).expect("whole tag match");
            let end_tag = closing(&capture).to_ascii_lowercase();
            if let Some(relative) = folded[matched.end()..].find(&end_tag) {
                let body_end = matched.end() + relative;
                let end = body_end + end_tag.len();
                offset = end;
                return Some(Tag {
                    start: matched.start(),
                    end,
                    name: capture.get(1).map_or("", |m| m.as_str()).into(),
                    body: &text[matched.end()..body_end],
                    raw: &text[matched.start()..end],
                });
            } else {
                offset = matched.end();
            }
        }
        None
    })
}
fn trim(value: &str) -> String {
    value.trim_matches(|c: char| matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')).into()
}
fn argument(tag: Tag<'_>) -> (String, String) {
    (tag.name, trim(tag.body))
}
fn named_parameters(text: &str) -> BTreeMap<String, String> {
    static OPEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)<param\s+name="([^"]+)"[^>]*>"#).unwrap());
    tags(text, &OPEN, |_| "</param>".into())
        .map(argument)
        .collect()
}
fn parameters(text: &str, format: &str, tool: &str) -> BTreeMap<String, String> {
    static HYPHEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<parameter-([^>\s/]+)[^>]*>").unwrap());
    static EQUALS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<parameter=([^>\s/]+)[^>]*>").unwrap());
    static CHILD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<([^>\s/]+)[^>]*>").unwrap());
    let mut args: BTreeMap<_, _> = match format {
        "hyphen" => tags(text, &HYPHEN, |c| format!("</parameter-{}>", &c[1]))
            .map(argument)
            .collect(),
        "equals" => tags(text, &EQUALS, |_| "</parameter>".into())
            .map(argument)
            .collect(),
        _ => {
            return {
                let named = named_parameters(text);
                if !named.is_empty() {
                    named
                } else {
                    tags(text, &CHILD, |c| format!("</{}>", &c[1]))
                        .filter(|tag| tag.name != tool)
                        .map(|tag| (tag.name, trim(tag.body)))
                        .collect()
                }
            };
        }
    };
    if args.is_empty() {
        args = named_parameters(text);
    }
    args
}
pub fn parse(text: &str, known: &[String]) -> Result<Vec<Call>, DomainError> {
    if text.len() > 4 * 1024 * 1024
        || known.len() > 128
        || known.iter().any(|name| name.is_empty() || name.len() > 128)
    {
        return Err(DomainError::LimitExceeded("XML fallback input".into()));
    }
    if known.is_empty() {
        return Ok(vec![]);
    }
    static FENCES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
    static WRAPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<tool_call>").unwrap());
    let stripped = FENCES.replace_all(text, "");
    let names = known
        .iter()
        .map(|name| regex::escape(name))
        .collect::<Vec<_>>()
        .join("|");
    let mut calls = vec![];
    let mut claimed = vec![];
    let mut add =
        |tag: &Tag<'_>, name: &str, body: &str, format: &str| -> Result<(), DomainError> {
            if calls.len() == 128 {
                return Err(DomainError::LimitExceeded("XML fallback batch".into()));
            }
            calls.push(Call {
                name: name.into(),
                id: format!("xml-call-{}", calls.len()),
                arguments: parameters(body, format, name),
                raw_xml: tag.raw.into(),
            });
            Ok(())
        };
    for (format, opening, fixed_close) in [
        (
            "hyphen",
            format!(r"(?i)<function-({names})(?:\s[^>]*)?>"),
            false,
        ),
        (
            "equals",
            format!(r"(?i)<function=({names})(?:\s[^>]*)?>"),
            true,
        ),
    ] {
        let opening = regex(&opening)?;
        for wrapped in tags(&stripped, &WRAPPER, |_| "</tool_call>".into()) {
            let mut matches = tags(wrapped.body, &opening, move |c| {
                if fixed_close {
                    "</function>".into()
                } else {
                    format!("</function-{}>", &c[1])
                }
            });
            let Some(function) = matches.next() else {
                continue;
            };
            if claimed
                .iter()
                .any(|(start, end)| wrapped.start < *end && wrapped.end > *start)
            {
                continue;
            }
            add(&wrapped, &function.name, function.body, format)?;
            claimed.push((wrapped.start, wrapped.end));
        }
    }
    let bare = regex(&format!(r"(?i)<({names})(?:\s[^>]*)?>"))?;
    for tag in tags(&stripped, &bare, |c| format!("</{}>", &c[1])) {
        if claimed
            .iter()
            .any(|(start, end)| tag.start < *end && tag.end > *start)
        {
            continue;
        }
        add(&tag, &tag.name, tag.body, "bare")?;
        claimed.push((tag.start, tag.end));
    }
    Ok(calls)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub call_count: usize,
    pub assistant_text_length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_stop_reason: Option<String>,
    pub iteration: usize,
    pub severity: String,
    pub should_inject_recovery_prompt: bool,
    pub reasons: Vec<String>,
    pub continuation_mode: String,
}
pub fn analyze(count: usize, text_length: usize, stop: Option<&str>, iteration: usize) -> Analysis {
    let mut severity = "normal";
    let mut reasons = vec![];
    if stop.is_some_and(|reason| {
        ["max_tokens", "length"].contains(&reason.to_ascii_lowercase().as_str())
    }) {
        severity = "critical";
        reasons.push("truncated_turn".into());
    }
    if text_length >= 12000 {
        severity = "critical";
        reasons.push("very_long_output".into());
    } else if text_length >= 4000 && severity != "critical" {
        severity = "elevated";
        reasons.push("long_output".into());
    }
    if count > 1 {
        if severity == "normal" {
            severity = "elevated";
        }
        reasons.push("multiple_xml_calls".into());
    }
    if iteration >= 1 {
        if severity == "normal" {
            severity = "elevated";
        }
        reasons.push("repeated_xml_iteration".into());
    }
    Analysis {
        call_count: count,
        assistant_text_length: text_length,
        assistant_stop_reason: stop.map(str::to_owned),
        iteration,
        severity: severity.into(),
        should_inject_recovery_prompt: severity != "normal",
        reasons,
        continuation_mode: if severity == "normal" {
            "default"
        } else {
            "guided"
        }
        .into(),
    }
}
pub fn guidance(analysis: &Analysis, tools: &[String]) -> Option<String> {
    let tools: Vec<_> = tools.iter().map(|name| name.to_ascii_lowercase()).collect();
    let mutation = tools.iter().any(|name| {
        ["edit", "write", "str_replace_editor", "notebook_edit"].contains(&name.as_str())
    });
    if !analysis.should_inject_recovery_prompt {
        return mutation.then(|| "XML fallback recovery mode: Continue only from the appended tool result. If the edit/write succeeded, do not repeat or revise it; run the requested verification next. If it failed, make exactly one corrected native tool call. Keep the next response terse.".into());
    }
    let local = !tools.is_empty()
        && tools
            .iter()
            .all(|name| ["ls", "find", "grep", "read"].contains(&name.as_str()));
    let hint = if local {
        "If the appended local tool results already reveal the requested file or directory, answer directly now and do not call more tools."
    } else {
        "If the appended tool results already answer the user's question, answer directly now and avoid another tool call."
    };
    Some(if analysis.severity == "critical" {
        format!(
            "XML fallback recovery mode: The previous assistant turn degraded into a long or truncated XML fallback. Continue only from the appended tool results. Keep the next response terse. {hint} If another tool is needed, emit exactly one tool call with minimal arguments. Do not restate prior analysis, and do not repeat large XML blocks."
        )
    } else {
        format!(
            "XML fallback recovery mode: The previous assistant turn is drifting into XML fallback. Continue from the appended tool results only. Keep the next response concise. {hint} If another tool is needed, emit exactly one tool call. Do not repeat previous analysis or XML verbosely."
        )
    })
}
