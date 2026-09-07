use super::slice_utf16;
use regex::Regex;
use std::{collections::BTreeSet, sync::LazyLock};
static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
    r"(?i)(?-u:\b)(?:no|without)\s+(?:(?:actionable|active|additional|relevant)\s+)?(?:contract|requirement|rule|evidence)(?-u:\b)",
    r"(?-u:\b)(?:COMPATIBILITY_CONTRACT|CURRENT_CONTRACT|AUTHORITATIVE_CONTRACT|TEST_FAILURE|ROOT_CAUSE|IMPORTANT|INVARIANT|REQUIREMENT)(?-u:\b)",
    r"(?-u:\b)(?:OBSOLETE_CONTRACT|DEPRECATED_CONTRACT|STALE_CONTRACT)(?-u:\b)",
    r"(?i)(?-u:\b)(?:error|failed|failure|panic|exception|fatal)(?-u:\b)",
    r"(?i)(?-u:\b)(?:contract|required|requirement|expected|actual|invariant|root cause)(?-u:\b)",
    r"(?i)(?-u:\b)(?:authoritative|non-authoritative|superseded|obsolete|deprecated|stale)(?-u:\b)",
    r"(?i)(?-u:\b)warning(?-u:\b)",
].into_iter().map(|p|Regex::new(p).unwrap()).collect()
});
pub fn score(line: &str) -> u32 {
    let s = line.trim();
    let bytes = s.as_bytes();
    let standalone = !s.contains(['\t', ' '])
        && (s.starts_with('/')
            || s.starts_with("./")
            || s.starts_with("../")
            || bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && b"\\/".contains(&bytes[2]));
    if standalone || PATTERNS[0].is_match(s) {
        return 0;
    }
    [12, 9, 7, 4, 4, 2]
        .iter()
        .zip(PATTERNS.iter().skip(1))
        .map(|(score, p)| if p.is_match(line) { *score } else { 0 })
        .sum()
}
pub fn select(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut candidates = vec![];
    for (index, line) in text.split('\n').enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let normalized = super::normalize_whitespace(line);
        if !seen.insert(normalized) {
            continue;
        }
        let score = score(line);
        if score > 0 {
            candidates.push((index, score, line));
        }
    }
    let floor = if candidates.iter().any(|(_, score, _)| *score >= 12) {
        7
    } else {
        1
    };
    candidates.retain(|(_, s, _)| *s >= floor);
    candidates.sort_by_key(|(i, s, _)| (std::cmp::Reverse(*s), *i));
    candidates.truncate(6);
    candidates.sort_by_key(|(i, _, _)| *i);
    candidates
        .into_iter()
        .map(|(_, _, line)| {
            if line.encode_utf16().count() > 700 {
                format!(
                    "{} [line clipped; rehydrate artifact for exact continuation]",
                    slice_utf16(line, 700)
                )
            } else {
                line.into()
            }
        })
        .collect()
}
