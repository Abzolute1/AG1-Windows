//! Original UTF-8 line/byte truncation behavior; raw evidence is retained by the result stage.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub max_lines: usize,
    pub max_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_lines: 2000,
            max_bytes: 50 * 1024,
        }
    }
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Truncation {
    pub content: String,
    pub truncated: bool,
    pub truncated_by: Option<String>,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub output_lines: usize,
    pub output_bytes: usize,
    pub last_line_partial: bool,
    pub first_line_exceeds_limit: bool,
    pub max_lines: usize,
    pub max_bytes: usize,
}
pub fn truncate(content: &str, limit: Limits, tail: bool) -> Truncation {
    let line_count = content.split('\n').count();
    let mut out = Truncation {
        content: String::new(),
        truncated: false,
        truncated_by: None,
        total_lines: line_count,
        total_bytes: content.len(),
        output_lines: line_count,
        output_bytes: content.len(),
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines: limit.max_lines,
        max_bytes: limit.max_bytes,
    };
    if line_count <= limit.max_lines && content.len() <= limit.max_bytes {
        out.content = content.into();
        return out;
    }
    out.truncated = true;
    out.truncated_by = Some("lines".into());
    let mut selected = vec![];
    let mut used = 0;
    if !tail && content.split('\n').next().unwrap_or("").len() > limit.max_bytes {
        out.first_line_exceeds_limit = true;
        out.truncated_by = Some("bytes".into());
    } else {
        let iter: Box<dyn Iterator<Item = &str>> = if tail {
            Box::new(content.rsplit('\n'))
        } else {
            Box::new(content.split('\n'))
        };
        for line in iter.take(limit.max_lines) {
            let bytes = line.len() + usize::from(!selected.is_empty());
            if used + bytes > limit.max_bytes {
                out.truncated_by = Some("bytes".into());
                if tail && selected.is_empty() {
                    let mut start = line.len().saturating_sub(limit.max_bytes);
                    while !line.is_char_boundary(start) {
                        start += 1;
                    }
                    selected.push(&line[start..]);
                    used = line.len() - start;
                    out.last_line_partial = true;
                }
                break;
            }
            selected.push(line);
            used += bytes;
        }
        if selected.len() >= limit.max_lines && used <= limit.max_bytes {
            out.truncated_by = Some("lines".into());
        }
    }
    if tail {
        selected.reverse();
    }
    out.content = selected.join("\n");
    out.output_lines = selected.len();
    out.output_bytes = out.content.len();
    out
}
