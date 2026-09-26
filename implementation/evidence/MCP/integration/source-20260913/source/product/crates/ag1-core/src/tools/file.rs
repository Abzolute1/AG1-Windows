//! File transformations are pure. Publication and revision checks belong to the platform adapter.
use super::{diff, edit};
use crate::DomainError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use serde_json::{Value, json};

pub struct Transformation {
    /// None denotes a read-only result or a rejected transformation; never publish it.
    pub bytes: Option<Vec<u8>>,
    pub result: Value,
    pub create_parents: bool,
}
fn text(s: impl Into<String>) -> Value {
    json!({"type":"text","text":s.into()})
}
pub fn structured<T: Serialize>(value: &T) -> Result<Value, DomainError> {
    let details =
        serde_json::to_value(value).map_err(|_| DomainError::InvalidInput("tool result".into()))?;
    let mut result = json!({"content":[text(serde_json::to_string_pretty(value).map_err(|_| DomainError::InvalidInput("tool result".into()))?)],"details":details});
    // An unsuccessful transformation is a failed tool result even when the
    // adapter completed normally. Consumers must not infer success from Ok.
    if !result["details"]["error"].is_null() {
        result["isError"] = json!(true);
    }
    Ok(result)
}
fn field<'a>(args: &'a Value, name: &str) -> Result<&'a str, DomainError> {
    args[name]
        .as_str()
        .ok_or_else(|| DomainError::InvalidInput(format!("{name} must be a string")))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppendResult<'a> {
    total_lines: usize,
    bytes_appended: usize,
    was_created: bool,
    content: &'a str,
}
#[derive(Serialize)]
pub struct ApplyResult<'a> {
    pub path: &'a str,
    pub applied: usize,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
/// Inputs have already passed registry validation; bounds are also enforced here for direct hosts.
pub fn transform(
    name: &str,
    args: &Value,
    original: Option<&[u8]>,
) -> Result<Transformation, DomainError> {
    super::schema::bounded(args, 8 * 1024 * 1024, 64)?;
    if original.is_some_and(|s| s.len() > 8 * 1024 * 1024) {
        return Err(DomainError::LimitExceeded("file bytes".into()));
    }
    let preview = name == "edit" && args["mode"] == "preview";
    if name == "edit" {
        if args.get("mode").is_some_and(|mode| mode != "preview" && mode != "apply") {
            return Err(DomainError::InvalidInput("edit mode must be preview or apply".into()));
        }
        if let Some(expected) = args.get("expectedSha256") {
            let hash = original.map(|bytes| format!("{:x}", Sha256::digest(bytes)));
            if expected.as_str().is_none() || hash.as_deref() != expected.as_str() {
                return Err(DomainError::InvalidInput("The file changed since this preview. Read or preview the current file before applying.".into()));
            }
        }
    }
    let path = field(args, "path")?;
    let old = String::from_utf8_lossy(original.unwrap_or_default());
    let (new, result, parents) = match name {
        "write" => {
            let new = field(args, "content")?.to_string();
            let d = diff::generate(&old, &new, 4)?;
            let count = if new.is_empty() {
                0
            } else {
                new.split('\n').count()
            };
            let mut details = json!({"diffSummary":d.summary});
            if let Some(line) = d.first_changed_line {
                details["firstChangedLine"] = json!(line);
            }
            let result = json!({"content":[text(format!("Wrote {}B ({count} line{}) to {path}.",new.len(),if count==1{""}else{"s"})),text(diff::format_summary(&d.summary)),text(d.diff)],"details":details});
            (new, result, true)
        }
        "edit" => {
            let edits: Vec<edit::Edit> = serde_json::from_value(args["edits"].clone())
                .map_err(|_| DomainError::InvalidInput("edits".into()))?;
            if edits.is_empty() {
                return Err(DomainError::InvalidInput(
                    "Edit tool input is invalid. edits must contain at least one replacement."
                        .into(),
                ));
            }
            if original.is_none() {
                return Err(DomainError::InvalidInput(format!("File not found: {path}")));
            }
            let content = edit::normalize_lf(old.strip_prefix('\u{feff}').unwrap_or(&old));
            let changed = edit::apply(
                &content,
                &edits,
                path,
                args["allowFuzzyMatch"].as_bool().unwrap_or(false),
            )?;
            let d = diff::generate(&changed.base_content, &changed.new_content, 4)?;
            let mut details = json!({"diff":d.diff,"diffSummary":d.summary,"sourceSha256":format!("{:x}", Sha256::digest(original.unwrap())),"mode":if preview {"preview"} else {"apply"}});
            if let Some(line) = d.first_changed_line {
                details["firstChangedLine"] = json!(line);
            }
            if changed.fuzzy_match_used {
                details["fuzzyMatchUsed"] = json!(true);
            }
            let note = if changed.fuzzy_match_used {
                " Fuzzy match used; review the diff carefully."
            } else {
                ""
            };
            let mut result = json!({"content":[text(format!("{} {} block(s) in {path}.{note}", if preview {"Previewed"} else {"Patched"}, edits.len())),text(diff::format_summary(&d.summary)),text(d.diff)],"details":details});
            if preview {
                let hash = result["details"]["sourceSha256"].as_str().unwrap();
                let receipt = text(format!("Preview only; no file was written. Source SHA-256: {hash}. Apply the same edits with mode=apply and expectedSha256 set to this exact 64-character hash."));
                result["content"].as_array_mut().unwrap().push(receipt);
            }
            (edit::restore(&old, &changed.new_content), result, false)
        }
        "append" => {
            let appended = field(args, "content")?;
            let separator = if old.is_empty() || old.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            let new = format!("{old}{separator}{appended}");
            let data = AppendResult {
                total_lines: new.split('\n').count(),
                bytes_appended: appended.len(),
                was_created: original.is_none(),
                content: &new,
            };
            let mut result = structured(&data)?;
            result["details"].as_object_mut().unwrap().remove("content");
            (new, result, false)
        }
        "editApply" => {
            if original.is_none() {
                return Ok(Transformation {
                    bytes: None,
                    result: structured(&ApplyResult {
                        path,
                        applied: 0,
                        verified: false,
                        error: Some(format!("file not found: {path}")),
                    })?,
                    create_parents: false,
                });
            }
            let edits: Vec<edit::Edit> = serde_json::from_value(args["edits"].clone())
                .map_err(|_| DomainError::InvalidInput("edits".into()))?;
            if edits.len() > 256 {
                return Err(DomainError::LimitExceeded("edit count".into()));
            }
            let mut new = old.into_owned();
            for e in &edits {
                // Empty oldText is valid for editApply: the reference inserts at the start.
                let first = new.find(&e.old_text);
                // Search after the first character, not after the whole match:
                // overlapping occurrences ("aa" within "aaa") are ambiguous too.
                let ambiguous = !e.old_text.is_empty()
                    && first.is_some_and(|at| {
                        let next = at + e.old_text.chars().next().unwrap().len_utf8();
                        new[next..].contains(&e.old_text)
                    });
                if ambiguous {
                    return Ok(Transformation {
                        bytes: None,
                        result: structured(&ApplyResult {
                            path,
                            applied: 0,
                            verified: false,
                            error: Some(format!(
                                "oldText matches multiple locations in {path}. No changes from this call were written. Read the current target and include enough surrounding text to identify one unique location."
                            )),
                        })?,
                        create_parents: false,
                    });
                }
                let Some(at) = first else {
                    let mut units = 0;
                    let prefix: String = e
                        .old_text
                        .chars()
                        .take_while(|c| {
                            units += c.len_utf16();
                            units <= 40
                        })
                        .collect();
                    return Ok(Transformation {
                        bytes: None,
                        result: structured(&ApplyResult {
                            path,
                            applied: 0,
                            verified: false,
                            error: Some(format!(
                                "oldText not found in {path}: {prefix}. No changes from this call were written."
                            )),
                        })?,
                        create_parents: false,
                    });
                };
                let size = new.len() - e.old_text.len() + e.new_text.len();
                if size > 8 * 1024 * 1024 {
                    return Err(DomainError::LimitExceeded("edited file bytes".into()));
                }
                new.replace_range(at..at + e.old_text.len(), &e.new_text);
            }
            let result = structured(&ApplyResult {
                path,
                applied: edits.len(),
                verified: true,
                error: None,
            })?;
            (new, result, false)
        }
        _ => {
            return Err(DomainError::Unavailable(format!(
                "file transformation {name}"
            )));
        }
    };
    if new.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("edited file bytes".into()));
    }
    Ok(Transformation {
        bytes: if preview { None } else { Some(new.into_bytes()) },
        result,
        create_parents: parents,
    })
}
