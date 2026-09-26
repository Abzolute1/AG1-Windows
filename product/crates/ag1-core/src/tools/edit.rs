//! Exact simultaneous edits with explicit opt-in NFKC fuzzy matching.
use crate::DomainError;
use icu_normalizer::ComposingNormalizer;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Edit {
    pub old_text: String,
    pub new_text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppliedEdits {
    pub base_content: String,
    pub new_content: String,
    pub fuzzy_match_used: bool,
}
fn err(s: impl Into<String>) -> DomainError {
    DomainError::InvalidInput(s.into())
}
/// Preserve ordinary non-overlapping counts while detecting an otherwise
/// hidden second overlapping location. Two linear searches suffice for the
/// ambiguous single-count case; matching must never guess a target location.
pub(super) fn occurrences(content: &str, old: &str) -> usize {
    let count = content.matches(old).count();
    if count == 1 && !old.is_empty() {
        let at = content.find(old).expect("one counted match");
        let next = at + old.chars().next().unwrap().len_utf8();
        if content[next..].contains(old) {
            return 2;
        }
    }
    count
}
pub fn normalize_lf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}
pub fn line_ending(s: &str) -> &'static str {
    match (s.find("\r\n"), s.find('\n')) {
        (Some(a), Some(b)) if a < b => "\r\n",
        _ => "\n",
    }
}
pub fn fuzzy(s: &str) -> String {
    let normalized = ComposingNormalizer::new_nfkc().normalize(s).into_owned();
    normalized
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .map(|c| match c {
            '\u{2018}'..='\u{201b}' => '\'',
            '\u{201c}'..='\u{201f}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{a0}' | '\u{2002}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => ' ',
            _ => c,
        })
        .collect()
}
pub fn apply(
    content: &str,
    edits: &[Edit],
    path: &str,
    allow_fuzzy: bool,
) -> Result<AppliedEdits, DomainError> {
    if content.len() > 8 * 1024 * 1024
        || edits.is_empty()
        || edits.len() > 256
        || edits
            .iter()
            .map(|e| e.old_text.len() + e.new_text.len())
            .sum::<usize>()
            > 8 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("edit input".into()));
    }
    let edits: Vec<_> = edits
        .iter()
        .map(|e| Edit {
            old_text: normalize_lf(&e.old_text),
            new_text: normalize_lf(&e.new_text),
        })
        .collect();
    for (i, e) in edits.iter().enumerate() {
        if e.old_text.is_empty() {
            return Err(err(if edits.len() == 1 {
                format!("oldText must not be empty in {path}.")
            } else {
                format!("edits[{i}].oldText must not be empty in {path}.")
            }));
        }
    }
    let fuzzy_content = if allow_fuzzy {
        Some(fuzzy(content))
    } else {
        None
    };
    let use_fuzzy = edits.iter().any(|e| {
        !content.contains(&e.old_text)
            && fuzzy_content
                .as_ref()
                .is_some_and(|s| s.contains(&fuzzy(&e.old_text)))
    });
    let base = if use_fuzzy {
        fuzzy_content.as_ref().unwrap().as_str()
    } else {
        content
    };
    let mut matched = vec![];
    for (i, e) in edits.iter().enumerate() {
        let old = if base.contains(&e.old_text) {
            e.old_text.clone()
        } else if allow_fuzzy {
            fuzzy(&e.old_text)
        } else {
            e.old_text.clone()
        };
        let space = if base.contains(&e.old_text) {
            std::borrow::Cow::Borrowed(base)
        } else if allow_fuzzy {
            std::borrow::Cow::Owned(fuzzy(base))
        } else {
            std::borrow::Cow::Borrowed(base)
        };
        let Some(index) = space.find(&old) else {
            return Err(err(if edits.len() == 1 {
                format!(
                    "Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines."
                )
            } else {
                format!(
                    "Could not find edits[{i}] in {path}. The oldText must match exactly including all whitespace and newlines."
                )
            }));
        };
        let occurrences = if allow_fuzzy {
            let normalized_old = fuzzy(&e.old_text);
            let normalized_base = fuzzy(base);
            if normalized_old.is_empty() {
                normalized_base.encode_utf16().count().saturating_sub(1)
            } else {
                occurrences(&normalized_base, &normalized_old)
            }
        } else {
            occurrences(base, &e.old_text)
        };
        if occurrences > 1 {
            return Err(err(if edits.len() == 1 {
                format!(
                    "Found {occurrences} occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique."
                )
            } else {
                format!(
                    "Found {occurrences} occurrences of edits[{i}] in {path}. Each oldText must be unique. Please provide more context to make it unique."
                )
            }));
        }
        matched.push((index, old.len(), i, &e.new_text));
    }
    matched.sort_by_key(|m| m.0);
    for pair in matched.windows(2) {
        if pair[0].0 + pair[0].1 > pair[1].0 {
            return Err(err(format!(
                "edits[{}] and edits[{}] overlap in {path}. Merge them into one edit or target disjoint regions.",
                pair[0].2, pair[1].2
            )));
        }
    }
    let mut next = base.to_string();
    for (at, len, _, text) in matched.into_iter().rev() {
        if !next.is_char_boundary(at) || !next.is_char_boundary(at + len) {
            return Err(err("fuzzy edit boundary"));
        }
        next.replace_range(at..at + len, text);
    }
    if next == base {
        return Err(err(if edits.len() == 1 {
            format!(
                "No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected."
            )
        } else {
            format!("No changes made to {path}. The replacements produced identical content.")
        }));
    }
    if next.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("edited file bytes".into()));
    }
    Ok(AppliedEdits {
        base_content: base.into(),
        new_content: next,
        fuzzy_match_used: use_fuzzy,
    })
}
pub fn restore(original: &str, edited: &str) -> String {
    let text = if line_ending(original) == "\r\n" {
        edited.replace('\n', "\r\n")
    } else {
        edited.into()
    };
    if original.starts_with('\u{feff}') {
        format!("\u{feff}{text}")
    } else {
        text
    }
}
