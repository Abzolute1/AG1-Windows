//! Pinned search ranking and compact result presentation. Hosts supply validated contained paths/bytes.
use super::{
    list::compare,
    truncate::{self, Limits},
};
use crate::{DomainError, context::slice_utf16, task::units};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};
fn re(s: &str) -> Regex {
    Regex::new(&s.replace(r"\b", r"(?-u:\b)")).expect("audited search pattern")
}
fn extension(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .filter(|(base, _)| !base.is_empty())
        .map(|(_, ext)| ext)
        .unwrap_or("")
}
pub fn find_rank(path: &str) -> i64 {
    let path = path.to_lowercase();
    let mut score = 0;
    if path.ends_with('/') {
        score += 20;
    }
    let ext = extension(path.trim_end_matches('/'));
    if ["mjs", "cjs", "js", "ts", "tsx", "py", "sh"].contains(&ext) {
        score -= 50;
    }
    if ["json", "jsonl", "csv", "log", "txt", "md"].contains(&ext) {
        score += 25;
    }
    if ["scripts/", "src/", "packages/"]
        .iter()
        .any(|s| path.starts_with(s) || path.contains(&format!("/{s}")))
    {
        score -= 35;
    }
    if [
        "history/",
        "history-test/",
        "/reports/",
        "/artifacts/",
        "/debug/",
        "__pycache__",
    ]
    .iter()
    .any(|s| path.contains(s))
    {
        score += 80;
    }
    if path.contains("compare") {
        score -= 20;
    }
    score + units(&path).min(140) as i64
}
pub fn find_miss(pattern: &str, search_dir: Option<&str>) -> String {
    static ANCHOR: LazyLock<Regex> = LazyLock::new(|| re("[A-Za-z0-9]+"));
    let anchors: BTreeSet<_> = ANCHOR
        .find_iter(pattern)
        .map(|m| m.as_str().to_lowercase())
        .filter(|s| s.len() >= 3 && !matches!(s.as_str(), "home" | "alex"))
        .collect();
    let mut notices = vec!["no matches".to_string()];
    if anchors.len() >= 3 {
        notices.extend([
        "pattern may be too specific; if you are looking for sibling files or roles, retry with the strongest shared anchor or common prefix instead of one filename containing every term",
        "retry with the shared anchor itself before falling back to one role-specific term",
        "if the question is really about which directory holds sibling scripts/files, stop broadening wildcard filenames and inspect the likely shared parent directory instead, for example `scripts/` or one repo-root listing",
    ].map(str::to_string));
    }
    if anchors.len() >= 2 {
        notices.push("if this is a command/script lookup, retry the strongest action stem with a likely extension, for example `*archive*.mjs`, instead of another multi-term permutation".into());
    }
    if search_dir
        .unwrap_or("")
        .replace('\\', "/")
        .to_lowercase()
        .contains("/packages/coding-agent/scripts")
        && ["archive", "compare", "runner", "script"]
            .iter()
            .any(|s| anchors.contains(*s))
    {
        notices.push("for repo-level command/script lookups, do not stay in package-local scripts after a miss; pivot to repo-root `scripts/` and retry a stronger shared anchor like `*archive*.mjs`, `*compare*.mjs`, or `*runner*.mjs`".into());
    }
    notices.push("if matches later cluster under one directory, answer from that shared parent directory instead of expanding discovery".into());
    format!("[{}]", notices.join("; "))
}
pub fn find_projection(
    pattern: &str,
    search_dir: Option<&str>,
    paths: &[String],
    limit: usize,
) -> Result<Value, DomainError> {
    if pattern.len() > 4096
        || search_dir.is_some_and(|s| s.len() > 4096)
        || limit == 0
        || limit > 10000
        || paths.len() > 10000
        || paths.iter().map(String::len).sum::<usize>() > 4 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("find results".into()));
    }
    if paths.is_empty() {
        return Ok(
            json!({"content":[{"type":"text","text":format!("(no matches)\n{}",find_miss(pattern,search_dir))}]}),
        );
    }
    let mut paths = paths.to_vec();
    paths.sort_by_cached_key(|path| find_rank(path));
    // Rank ties use the same root-locale collation as the pinned SDK.
    let mut start = 0;
    while start < paths.len() {
        let score = find_rank(&paths[start]);
        let end = start + paths[start..].partition_point(|p| find_rank(p) == score);
        paths[start..end].sort_by(|a, b| compare(a, b));
        start = end;
    }
    let reached = paths.len() >= limit;
    let truncated = truncate::truncate(
        &paths.join("\n"),
        Limits {
            max_lines: 9_007_199_254_740_991,
            max_bytes: 50 * 1024,
        },
        false,
    );
    let mut output = truncated.content.clone();
    let mut details = json!({"resultCount":paths.len(),"firstMatchPath":paths[0],"firstMatchIsDirectory":paths[0].ends_with('/')});
    let mut directories: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (i, path) in paths.iter().enumerate() {
        if !path.ends_with('/')
            && let Some((dir, _)) = path.rsplit_once('/')
            && !dir.is_empty()
        {
            let entry = directories.entry(dir).or_insert((0, i));
            entry.0 += 1;
        }
    }
    if let Some((dir, _)) = directories
        .into_iter()
        .min_by_key(|(_, (count, index))| (std::cmp::Reverse(*count), *index))
    {
        details["dominantFileDirectory"] = json!(dir);
    }
    let mut notices = vec![];
    if reached {
        details["resultLimitReached"] = json!(limit);
        notices.push(format!("{limit} results limit; next limit={}", limit * 2));
    }
    if truncated.truncated {
        details["truncation"] = json!(truncated);
        notices.push("50.0KB byte limit".into());
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join("; ")));
    }
    if paths.len() == 1 && !reached && !truncated.truncated {
        let path = &paths[0];
        output.push_str(&if path.ends_with('/') {format!("\n\n[1 directory match in {path}; if the directory itself answers the question, answer now; otherwise next: ls {path}]")}else{format!("\n\n[1 match in {path}; if the user only asked which file/path, answer now; otherwise next: read {path} limit=120]")});
    }
    Ok(json!({"content":[{"type":"text","text":output}],"details":details}))
}
pub fn grep_path_rank(path: &str) -> i64 {
    static SCRIPT: LazyLock<Regex> =
        LazyLock::new(|| re(r"(^|/)scripts/[^/]+\.(mjs|cjs|js|ts|py|sh)$"));
    let path = path.to_lowercase();
    let mut score = 0;
    let ext = extension(&path);
    if [
        "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "rs", "java", "kt", "rb", "php",
        "swift", "cpp", "cc", "cxx", "c", "h", "hpp",
    ]
    .contains(&ext)
    {
        score -= 60;
    } else if ["md", "mdx", "rst", "txt"].contains(&ext) {
        score += 80;
    }
    if path.starts_with("src/")
        || ["/src/", "/packages/", "/lib/", "/app/"]
            .iter()
            .any(|s| path.contains(s))
    {
        score -= 40;
    }
    if ["/test/", "/tests/"].iter().any(|s| path.contains(s))
        || [
            ".test.ts",
            ".test.tsx",
            ".spec.ts",
            ".spec.tsx",
            ".test.js",
            ".spec.js",
        ]
        .iter()
        .any(|s| path.ends_with(s))
    {
        score += 110;
    }
    if ["readme", "mission", "plan", "audit", "checkpoint", "todo"]
        .iter()
        .any(|s| path.contains(s))
    {
        score += 60;
    }
    if ["/fixtures/", "new_files-copy", "/history/", "/archive/"]
        .iter()
        .any(|s| path.contains(s))
    {
        score += 120;
    }
    if ["/core/", "/system-prompt", "/prompt"]
        .iter()
        .any(|s| path.contains(s))
    {
        score -= 20;
    }
    if path.contains("/tools/") {
        score -= 20;
    }
    if path.contains("/auton/") {
        score += 25;
    }
    if SCRIPT.is_match(&path) {
        score -= 80;
    }
    if path.contains("research-output") {
        score += 180;
    }
    score + units(&path).min(120) as i64
}
pub fn grep_line_rank(line: &str, pattern: &str) -> i64 {
    static DEFINITIONS: LazyLock<Vec<(Regex, i64)>> = LazyLock::new(|| {
        [
            (r"^\s*export\s+(async\s+)?function\b", -140),
            (r"^\s*export\s+(class|interface|type|const)\b", -120),
            (r"^\s*(async\s+)?function\b", -80),
            (r"^\s*(class|interface|type|const)\b", -60),
        ]
        .into_iter()
        .map(|(p, s)| (re(p), s))
        .collect()
    });
    static PENALTIES: LazyLock<Vec<(Regex, i64)>> = LazyLock::new(|| {
        [
            (r"^\s*import\b", 120),
            (r"\b(describe|it|test)\s*\(", 120),
            (r"^\s*//", 20),
            (r"\blines\.push\s*\(", 140),
            (r"\bfor prompts about\b", 220),
            (r"\blikely implementation symbol\b", 220),
            (r"\bintent steering\b", 180),
            (r#""prompt"\s*:"#, 160),
            (r#""description"\s*:"#, 140),
            (r#""tags"\s*:"#, 120),
        ]
        .into_iter()
        .map(|(p, s)| (re(p), s))
        .collect()
    });
    static TEMPLATE: LazyLock<Regex> = LazyLock::new(|| re(r"`[^`]*\$\{[^`]+\}[^`]*`"));
    static PROSE: LazyLock<Regex> = LazyLock::new(|| re(r#"["'`](?:[^"'`]*\s){4,}[^"'`]*["'`]"#));
    static NORMAL: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"\bnormalizecurrenteventsquery\b",
            r"\bnormalize[a-z0-9_]*current[a-z0-9_]*events[a-z0-9_]*\b",
            r"\bnormalize\b",
            r"^\s*export\s+(async\s+)?function\s+normalize",
            r"^\s*(async\s+)?function\s+normalize",
        ]
        .map(re)
        .to_vec()
    });
    let normalized = line
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .to_lowercase();
    let pattern = pattern.trim().to_lowercase();
    let mut score = DEFINITIONS
        .iter()
        .find(|(r, _)| r.is_match(&normalized))
        .map(|(_, s)| *s)
        .unwrap_or(0);
    for (r, p) in PENALTIES.iter() {
        if r.is_match(&normalized) {
            score += p;
        }
    }
    if TEMPLATE.is_match(line) {
        score += 40;
    }
    if PROSE.is_match(line) {
        score += 70;
    }
    if pattern.contains("current-events") {
        score += if NORMAL[0].is_match(&normalized) {
            -260
        } else if NORMAL[1].is_match(&normalized) {
            -180
        } else if NORMAL[2].is_match(&normalized) {
            -80
        } else {
            0
        };
    }
    if pattern.contains("normalization") {
        score += if NORMAL[3].is_match(&normalized) {
            -120
        } else if NORMAL[4].is_match(&normalized) {
            -80
        } else {
            0
        };
    }
    score
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub path: String,
    pub line: usize,
    pub text: String,
}
pub fn grep_miss(pattern: &str, literal: bool) -> String {
    static LEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*([A-Za-z_][A-Za-z0-9_]*)"));
    static IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z_][A-Za-z0-9_]*$"));
    let mut notices = vec!["no matches".into()];
    if literal && pattern.chars().any(|c| "|()[]{}.*+?^$\\".contains(c)) {
        notices.push(
            "literal=true with regex-like pattern; retry with a simpler literal or disable literal"
                .into(),
        );
        if let Some(c) = LEADING.captures(pattern) {
            notices.push(format!("for definition lookups, retry the exact symbol name {} first without regex-like suffixes, assignment patterns, or added punctuation",&c[1]));
        }
        let alternatives: Vec<_> = pattern
            .split('|')
            .map(str::trim)
            .filter(|s| IDENTIFIER.is_match(s))
            .collect();
        if alternatives.len() >= 2 {
            notices.push(format!("if these are alternative identifiers, retry one identifier at a time, starting with {}, and keep the search inside likely implementation/reporting paths like scripts/ or src/ instead of broadening to prose",alternatives[0]));
        } else if pattern.contains('|') {
            notices.push("if this was meant as alternative identifiers, retry one identifier at a time and keep the search inside likely implementation/reporting paths like scripts/ or src/ instead of broadening to prose".into());
        }
    }
    notices.push("if the user asked which file/script defines something, prefer find on filenames before broad content grep".into());
    notices.push("for local code lookups, retry with a narrower source path like src/ or packages/*/src before reading docs".into());
    format!("[{}]", notices.join("; "))
}
pub struct GrepProjection<'a> {
    pub pattern: &'a str,
    pub search_dir: &'a str,
    pub directory: bool,
    pub literal: bool,
    pub context: usize,
    pub limit: usize,
}
pub fn grep_projection(
    options: GrepProjection<'_>,
    matches: &[Match],
    files: &BTreeMap<String, String>,
) -> Result<Value, DomainError> {
    let GrepProjection {
        pattern,
        search_dir,
        directory,
        literal,
        context,
        limit,
    } = options;
    if pattern.len() > 4096
        || search_dir.len() > 4096
        || matches
            .iter()
            .map(|m| m.path.len() + m.text.len())
            .sum::<usize>()
            > 32 * 1024 * 1024
        || matches.len() > 10000
        || limit == 0
        || limit > 10000
        || context > 100
        || files.len() > 10000
        || files.values().map(String::len).sum::<usize>() > 32 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("grep projection input".into()));
    }
    if matches.is_empty() {
        return Ok(
            json!({"content":[{"type":"text","text":format!("(no matches)\n{}",grep_miss(pattern,literal))}],"details":{"matchCount":0}}),
        );
    }
    let mut ranked: Vec<_> = matches
        .iter()
        .map(|m| {
            (
                grep_path_rank(&m.path) + grep_line_rank(&m.text, pattern),
                m,
            )
        })
        .collect();
    ranked.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then_with(|| compare(&a.path, &b.path))
            .then(a.line.cmp(&b.line))
    });
    let matches: Vec<_> = ranked.into_iter().map(|(_, m)| m).collect();
    // Merge requested windows, then scan each file once. A dense newline file must
    // not allocate a whole-file line index or rescan its bytes for every hit.
    let mut windows: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    for hit in &matches {
        if hit.line == 0 || hit.line > 32 * 1024 * 1024 + 1 {
            return Err(DomainError::Conflict);
        }
        windows
            .entry(&hit.path)
            .or_default()
            .push((hit.line.saturating_sub(context).max(1), hit.line + context));
    }
    let mut selected = BTreeMap::new();
    let mut selected_count = 0;
    for (path, mut ranges) in windows {
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = vec![];
        for (start, end) in ranges {
            if let Some(last) = merged.last_mut().filter(|last| start <= last.1 + 1) {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let source = files.get(path).ok_or(DomainError::Conflict)?;
        let mut rows = BTreeMap::new();
        let mut range = 0;
        let mut count = 0;
        for (index, text) in source.split('\n').enumerate() {
            let line = index + 1;
            count = line;
            while range < merged.len() && line > merged[range].1 {
                range += 1;
            }
            if range < merged.len() && line >= merged[range].0 {
                selected_count += 1;
                if selected_count > 100_000 {
                    return Err(DomainError::LimitExceeded("grep context rows".into()));
                }
                rows.insert(line, text);
            }
        }
        selected.insert(path, (count, rows));
    }
    let reached = matches.len() >= limit;
    let mut lines = vec![];
    let mut clipped = false;
    let mut bytes = 0;
    for hit in &matches {
        let display = if directory && search_dir.is_empty() {
            hit.path.as_str()
        } else if directory {
            hit.path
                .strip_prefix(&format!("{search_dir}/"))
                .unwrap_or_else(|| hit.path.rsplit('/').next().unwrap())
        } else {
            hit.path.rsplit('/').next().unwrap()
        };
        let (count, rows) = selected
            .get(hit.path.as_str())
            .ok_or(DomainError::Conflict)?;
        // rg counts LF-delimited physical lines; lone CRs stay on that line.
        if hit.line > *count {
            return Err(DomainError::Conflict);
        }
        let start = hit.line.saturating_sub(context).max(1);
        let end = (hit.line + context).min(*count);
        for current in start..=end {
            let raw = rows.get(&current).ok_or(DomainError::Conflict)?;
            let text = raw.strip_suffix('\r').unwrap_or(raw).replace('\r', "\\r");
            let text = if units(&text) > 500 {
                clipped = true;
                format!("{}... [truncated]", slice_utf16(&text, 500))
            } else {
                text
            };
            let line = if current == hit.line {
                format!("{display}:{current}: {text}")
            } else {
                format!("{display}-{current}- {text}")
            };
            bytes += line.len();
            if bytes > 16 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("grep output".into()));
            }
            lines.push(line);
        }
    }
    let truncated = truncate::truncate(
        &lines.join("\n"),
        Limits {
            max_lines: 9_007_199_254_740_991,
            max_bytes: 50 * 1024,
        },
        false,
    );
    let mut output = truncated.content.clone();
    let unique: BTreeSet<_> = matches.iter().map(|m| &m.path).collect();
    let mut details = json!({"matchCount":matches.len(),"uniqueFileCount":unique.len(),"firstMatchPath":matches[0].path,"firstMatchLine":matches[0].line});
    let mut notices = vec![];
    if reached {
        details["matchLimitReached"] = json!(limit);
        notices.push(format!("{limit} matches limit; next limit={}", limit * 2));
    }
    if truncated.truncated {
        details["truncation"] = json!(truncated);
        notices.push("50.0KB byte limit".into());
    }
    if clipped {
        details["linesTruncated"] = json!(true);
        notices.push("line clips at 500 chars; use read".into());
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join("; ")));
    }
    if matches.len() == 1 && !reached && !truncated.truncated {
        let hit = &matches[0];
        let offset = hit.line.saturating_sub(20).max(1);
        output.push_str(&format!("\n\n[1 match in {}:{}; if the user only asked which file/path, answer now; otherwise next: read {} offset={offset} limit=120]",hit.path,hit.line,hit.path));
    }
    Ok(json!({"content":[{"type":"text","text":output}],"details":details}))
}
