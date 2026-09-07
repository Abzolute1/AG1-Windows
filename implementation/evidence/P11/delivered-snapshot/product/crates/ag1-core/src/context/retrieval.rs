use super::{
    code::{self, CodeFile, Relation, Symbol},
    slice_utf16,
};
use crate::{
    DomainError,
    evidence::{Freshness, SourceReference, SourceSnapshot},
    task::{invalid, units},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentAccess {
    pub path: String,
    pub kind: String,
    pub count: u64,
    pub distance: u64,
}
#[derive(Clone, Debug, Default)]
pub struct WorkingTree {
    pub inside_repository: bool,
    pub changed_files: BTreeSet<String>,
    pub changed_lines: BTreeMap<String, BTreeSet<usize>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetrievalOptions {
    pub query: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    pub context_lines: Option<usize>,
    pub max_chars: Option<usize>,
}
pub trait SourceRetrievalPort {
    fn capture(&self, path: &str) -> Result<SourceSnapshot, DomainError>;
    fn lexical(&self, terms: &[String]) -> Result<BTreeMap<String, BTreeSet<usize>>, DomainError>;
    fn working_tree(&self) -> Result<WorkingTree, DomainError>;
    fn recent_accesses(&self) -> Result<Vec<RecentAccess>, DomainError>;
    fn cancelled(&self) -> bool;
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapsuleFile {
    pub path: String,
    pub reference: String,
    pub ranges: Vec<SourceRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<Symbol>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleDraft {
    pub id: String,
    pub query: String,
    pub body: String,
    pub files: Vec<CapsuleFile>,
    pub relations: Vec<Relation>,
    pub chars: usize,
    pub truncated: bool,
    pub retrieval: Value,
}
struct Candidate {
    path: String,
    lines: BTreeSet<usize>,
    score: i64,
    source: Vec<String>,
    reference: String,
    symbols: Vec<Symbol>,
    signals: Vec<String>,
}
pub fn query_terms(input: &RetrievalOptions) -> Vec<String> {
    let stop = [
        "about",
        "after",
        "before",
        "could",
        "current",
        "does",
        "from",
        "have",
        "into",
        "investigate",
        "need",
        "should",
        "source",
        "that",
        "this",
        "what",
        "when",
        "where",
        "which",
        "with",
    ];
    let regex = Regex::new(r"[A-Za-z_$][A-Za-z0-9_$.-]{2,}").unwrap();
    let values = input
        .symbols
        .iter()
        .map(String::as_str)
        .chain(regex.find_iter(&input.query).map(|m| m.as_str()));
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for value in values {
        let term = value.trim();
        let key = term.to_lowercase();
        if units(term) < 3 || stop.contains(&key.as_str()) || !seen.insert(key) {
            continue;
        }
        out.push(slice_utf16(term, 200));
        if out.len() == 12 {
            break;
        }
    }
    out
}
pub fn recent_score(path: &str, accesses: &[RecentAccess]) -> u64 {
    accesses
        .iter()
        .filter(|a| {
            if a.kind == "file" {
                a.path == path
            } else {
                a.path == path || path.starts_with(&format!("{}/", a.path))
            }
        })
        .map(|a| {
            16u64.saturating_sub(a.distance.saturating_mul(3))
                + (a.count.saturating_add(1) as f64).log2().ceil().min(3.) as u64
        })
        .max()
        .unwrap_or(0)
        .min(18)
}
fn find_lines(lines: &[String], terms: &[String]) -> BTreeSet<usize> {
    let terms: Vec<_> = terms.iter().map(|t| t.to_lowercase()).collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| terms.iter().any(|t| l.to_lowercase().contains(t)))
        .take(20)
        .map(|(i, _)| i + 1)
        .collect()
}
fn ranges(seeds: &BTreeSet<usize>, count: usize, context: usize) -> Vec<SourceRange> {
    if seeds.is_empty() {
        return vec![SourceRange {
            start: 1,
            end: count.min(context * 2 + 1),
        }];
    }
    let mut out: Vec<SourceRange> = vec![];
    for line in seeds.iter().take(8) {
        let next = SourceRange {
            start: line.saturating_sub(context).max(1),
            end: count.min(line.saturating_add(context)),
        };
        if let Some(last) = out.last_mut()
            && next.start <= last.end + 1
        {
            last.end = last.end.max(next.end);
        } else {
            out.push(next);
        }
    }
    out
}
fn score(path: &str, hits: usize, terms: &[String], explicit: bool) -> i64 {
    let p = path.to_lowercase();
    let mut score = if explicit { 10000 } else { hits as i64 * 20 };
    for t in terms {
        if p.contains(&t.to_lowercase()) {
            score += 100;
        }
    }
    if ["src", "lib", "app", "core"]
        .iter()
        .any(|s| format!("/{p}").contains(&format!("/{s}/")))
    {
        score += 30;
    }
    if p.contains(".test.") || p.contains(".spec.") {
        score -= 10;
    }
    if [".md", ".txt", ".log"].iter().any(|s| p.ends_with(s)) {
        score -= 40;
    }
    score
}
fn symbol_score(symbols: &[Symbol], terms: &[String]) -> i64 {
    symbols
        .iter()
        .map(|s| {
            terms
                .iter()
                .map(|t| {
                    let name = s.name.to_lowercase();
                    let term = t.to_lowercase();
                    if name == term {
                        500
                    } else if name.contains(&term) || term.contains(&name) {
                        120
                    } else {
                        0
                    }
                })
                .sum::<i64>()
        })
        .sum()
}
pub fn build_capsule(
    input: &RetrievalOptions,
    host: &impl SourceRetrievalPort,
) -> Result<CapsuleDraft, DomainError> {
    let query = input.query.trim();
    if query.is_empty()
        || input.query.len() > 8192
        || input.paths.len() > 8
        || input.symbols.len() > 12
    {
        return Err(invalid("source query limits"));
    }
    if host.cancelled() {
        return Err(DomainError::Cancelled);
    }
    let terms = query_terms(input);
    let mut paths = vec![];
    for p in &input.paths {
        SourceReference::new(p, &"0".repeat(64))?;
        if !paths.contains(p) {
            paths.push(p.clone());
        }
    }
    let recent = host.recent_accesses()?;
    if recent.len() > 100 {
        return Err(invalid("recent source cap"));
    }
    let discovered = if paths.is_empty() {
        host.lexical(&terms)?
    } else {
        BTreeMap::new()
    };
    let tree = host.working_tree()?;
    if discovered.len() > 100 || tree.changed_files.len() > 500 {
        return Err(invalid("retrieval input caps"));
    }
    let mut candidates = paths.clone();
    for p in discovered.keys() {
        if !candidates.contains(p) {
            candidates.push(p.clone());
        }
    }
    if paths.is_empty() {
        for a in recent
            .iter()
            .filter(|a| a.kind == "file" && a.distance <= 80)
            .take(2)
        {
            if !candidates.contains(&a.path) {
                candidates.push(a.path.clone());
            }
        }
        if discovered.is_empty() {
            for path in tree.changed_files.iter().take(4) {
                if !candidates.contains(path) {
                    candidates.push(path.clone());
                }
            }
        }
    }
    let mut selected = vec![];
    for path in candidates {
        if host.cancelled() {
            return Err(DomainError::Cancelled);
        }
        let source = match host.capture(&path) {
            Ok(s) => s,
            Err(error) => {
                if paths.contains(&path) && matches!(error, DomainError::PermissionDenied) {
                    return Err(error);
                }
                continue;
            }
        };
        let bytes = &source.snapshot().bytes;
        if bytes.len() > 2 * 1024 * 1024 || bytes.contains(&0) {
            continue;
        }
        let Ok(raw) = std::str::from_utf8(bytes) else {
            continue;
        };
        let lines: Vec<_> = raw
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .split('\n')
            .map(str::to_owned)
            .collect();
        let mut found = discovered
            .get(&path)
            .cloned()
            .unwrap_or_else(|| find_lines(&lines, &terms));
        let query_matched = !found.is_empty();
        if let Some(changed) = tree.changed_lines.get(&path) {
            found.extend(changed);
        }
        let symbols = code::symbols(&path, &lines);
        let mut symbol_matched = false;
        for s in &symbols {
            if terms
                .iter()
                .any(|t| s.name.to_lowercase().contains(&t.to_lowercase()))
            {
                found.insert(s.line);
                symbol_matched = true;
            }
        }
        let access = recent_score(&path, &recent);
        let explicit = paths.contains(&path);
        let mut signals = vec![];
        for (yes, signal) in [
            (explicit, "explicit"),
            (query_matched, "query-match"),
            (tree.changed_files.contains(&path), "working-tree-change"),
            (access > 0, "recent-access"),
            (symbol_matched, "symbol-match"),
        ] {
            if yes {
                signals.push(signal.into());
            }
        }
        let score = score(&path, found.len(), &terms, explicit)
            + if explicit {
                0
            } else {
                symbol_score(&symbols, &terms)
                    + access as i64
                    + if tree.changed_files.contains(&path) {
                        12
                    } else {
                        0
                    }
            };
        selected.push(Candidate {
            path,
            lines: found,
            score,
            source: lines,
            reference: source.reference().reference(),
            symbols,
            signals,
        });
    }
    selected.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    if selected.is_empty() {
        return Err(DomainError::NotFound);
    }
    let total = selected.len();
    let mut truncated = total > 8;
    let max = input.max_chars.unwrap_or(12000).clamp(4000, 24000);
    let context = input.context_lines.unwrap_or(6).clamp(2, 20);
    let mut sections = vec!["CONTEXT CAPSULE".into(), format!("reason_loaded: {query}")];
    let mut files = vec![];
    let mut included = vec![];
    for candidate in selected.iter().take(8) {
        let ranges = ranges(&candidate.lines, candidate.source.len(), context);
        let mut block = vec![
            format!("\nFILE {}", candidate.path),
            format!("provenance: {}", candidate.reference),
        ];
        if !candidate.signals.is_empty() {
            block.push(format!(
                "retrieval_signals: {}",
                candidate.signals.join(", ")
            ));
        }
        let symbols: Vec<_> = candidate
            .symbols
            .iter()
            .filter(|s| {
                terms
                    .iter()
                    .any(|t| s.name.to_lowercase().contains(&t.to_lowercase()))
            })
            .take(12)
            .collect();
        if !symbols.is_empty() {
            block.push(format!(
                "symbols: {}",
                symbols
                    .iter()
                    .map(|s| format!("{} {}@{}", s.kind, s.name, s.line))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let mut included_ranges = vec![];
        let mut reached = false;
        for range in ranges {
            let header = block.len();
            block.push(format!("lines: {}-{}", range.start, range.end));
            let mut end = range.start - 1;
            for line in range.start..=range.end {
                let raw = candidate
                    .source
                    .get(line - 1)
                    .map(String::as_str)
                    .unwrap_or("");
                let rendered = if units(raw) <= 2000 {
                    raw.into()
                } else {
                    format!(
                        "{} [line clipped; use read for exact content]",
                        slice_utf16(raw, 2000)
                    )
                };
                let next = format!("{line:6}  {rendered}");
                if units(&sections.join("\n")) + units(&block.join("\n")) + units(&next) + 1200
                    > max
                {
                    reached = true;
                    break;
                }
                block.push(next);
                end = line;
            }
            if end >= range.start {
                block[header] = format!("lines: {}-{end}", range.start);
                included_ranges.push(SourceRange {
                    start: range.start,
                    end,
                });
            } else {
                block.pop();
            }
            if reached {
                break;
            }
        }
        if included_ranges.is_empty() {
            truncated = true;
            break;
        }
        sections.push(block.join("\n"));
        files.push(CapsuleFile {
            path: candidate.path.clone(),
            reference: candidate.reference.clone(),
            ranges: included_ranges,
            symbols: candidate.symbols.iter().take(24).cloned().collect(),
            signals: candidate.signals.clone(),
        });
        included.push(CodeFile {
            path: &candidate.path,
            lines: &candidate.source,
            symbols: &candidate.symbols,
        });
        if reached {
            truncated = true;
            break;
        }
    }
    if files.is_empty() {
        return Err(DomainError::LimitExceeded("source capsule budget".into()));
    }
    let relations: Vec<_> = code::relations(&included).into_iter().take(32).collect();
    if !relations.is_empty() {
        let mut lines = vec!["\nRELATIONSHIPS".into()];
        for relation in relations.iter().take(12) {
            let line = format!(
                "- {} {} {}{}",
                relation.from,
                relation.kind,
                relation.to,
                relation
                    .label
                    .as_ref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default()
            );
            if units(&sections.join("\n")) + units(&lines.join("\n")) + units(&line) + 180 > max {
                truncated = true;
                break;
            }
            lines.push(line);
        }
        if lines.len() > 1 {
            sections.push(lines.join("\n"));
        }
    }
    if truncated {
        sections.push(
            "\n[capsule truncated; narrow query or explicit paths to retrieve omitted source]"
                .into(),
        );
    }
    let body = sections.join("\n");
    let id = format!(
        "cap_{}",
        &format!("{:x}", Sha256::digest(body.as_bytes()))[..16]
    );
    let body = format!("{body}\ncapsule_id: {id}");
    let chars = units(&body);
    let retrieval = json!({"lexicalCandidates":discovered.len(),"totalCandidates":total,"selectedFiles":files.len(),"workingTreeChangedFiles":tree.changed_files.len(),"recentAccessPaths":recent.len()});
    Ok(CapsuleDraft {
        id,
        query: query.into(),
        body,
        files,
        relations,
        chars,
        truncated,
        retrieval,
    })
}
pub fn capsule_status(
    files: &[CapsuleFile],
    artifact_available: bool,
    validate: impl Fn(&str) -> Freshness,
) -> Freshness {
    if !artifact_available {
        return Freshness::Invalid;
    }
    let statuses: Vec<_> = files.iter().map(|f| validate(&f.reference)).collect();
    [
        Freshness::Invalid,
        Freshness::Missing,
        Freshness::Stale,
        Freshness::Unverified,
        Freshness::Current,
    ]
    .into_iter()
    .find(|s| statuses.contains(s))
    .unwrap_or(Freshness::Invalid)
}
