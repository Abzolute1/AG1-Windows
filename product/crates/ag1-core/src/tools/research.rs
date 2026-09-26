//! Deterministic research projections ported from the pinned AG1 reference.
//! Text is untrusted source material; classifications never create verified claims.
use crate::task::units;
mod document;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, LazyLock, Mutex},
};

pub(crate) fn compile(pattern: &str, insensitive: bool) -> Regex {
    RegexBuilder::new(
        &pattern
            .replace(r"\b", r"(?-u:\b)")
            .replace(r"\w", r"[A-Za-z0-9_]")
            .replace(r"\d", r"[0-9]"),
    )
    .case_insensitive(insensitive)
    .build()
    .expect("pinned research regex")
}
pub(crate) fn rx(pattern: &'static str) -> Arc<Regex> {
    static CACHE: LazyLock<Mutex<BTreeMap<&'static str, Arc<Regex>>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));
    CACHE
        .lock()
        .expect("research regex cache")
        .entry(pattern)
        .or_insert_with(|| Arc::new(compile(pattern, false)))
        .clone()
}
pub(crate) fn has(pattern: &'static str, text: &str) -> bool {
    rx(pattern).is_match(text)
}
pub(crate) fn replace(text: &str, pattern: &'static str, to: &str) -> String {
    rx(pattern).replace_all(text, to).into_owned()
}
fn whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}
pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(whitespace)
}
pub(crate) fn compact(text: &str) -> String {
    text.split(whitespace)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub(crate) fn slice(text: &str, count: usize) -> String {
    String::from_utf16_lossy(&text.encode_utf16().take(count).collect::<Vec<_>>())
}
fn clipped(text: &str, max: usize) -> String {
    if units(text) > max {
        format!(
            "{}...",
            slice(
                text,
                if max == 0 {
                    units(text).saturating_sub(1)
                } else {
                    max - 1
                }
            )
        )
    } else {
        text.into()
    }
}

struct Rule {
    pattern: Regex,
    label: String,
}
static RULES: LazyLock<BTreeMap<String, Vec<Rule>>> = LazyLock::new(|| {
    let value: Value = serde_json::from_str(include_str!("../../resources/research-rules.json"))
        .expect("pinned research rules");
    value["rules"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, rows)| {
            (
                name.clone(),
                rows.as_array()
                    .unwrap()
                    .iter()
                    .map(|row| Rule {
                        pattern: compile(
                            row["pattern"].as_str().unwrap(),
                            row["flags"].as_str().unwrap().contains('i'),
                        ),
                        label: row["slot"]
                            .as_str()
                            .or_else(|| row["reason"].as_str())
                            .unwrap_or_default()
                            .into(),
                    })
                    .collect(),
            )
        })
        .collect()
});
fn rules(name: &str) -> &'static [Rule] {
    &RULES[name]
}
fn any(name: &str, text: &str) -> bool {
    rules(name).iter().any(|r| r.pattern.is_match(text))
}

pub fn explicit_code(query: &str) -> bool {
    has(
        r"(?i)(?:[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+)|(?:^|[\s/])(src|packages|app|lib|test|tests)\b|(?:\.[cm]?[jt]sx?\b)|(?:site:github)|(?:github)|(?:gitlab)|(?:raw\.githubusercontent)",
        query,
    )
}
fn api(query: &str) -> bool {
    has(
        r"(?i)\b(parameter|parameters|option|options|argument|arguments|returns?|signature|hook|api|endpoint|pricing|model id|provider|what it does|when .* useful|when to use)\b",
        query,
    )
}
pub fn code_lookup(query: &str) -> bool {
    explicit_code(query)
        || has(
            r"(?i)\b(rust|cargo|rustc|python|typescript|javascript|compiler|stdlib)\b",
            query,
        )
        || (has(r"\b[a-z][A-Za-z0-9]+[A-Z][A-Za-z0-9]*\b", query) && !api(query))
}
pub fn intent(query: &str) -> &'static str {
    let docs = has(
        r"(?i)\b(api|docs?|documentation|reference|manual|guide|spec|tutorial)\b",
        query,
    );
    let api = api(query);
    if has(
        r"(?i)\b(news|incident|leak|announcement|security advisory|outage|breach)\b",
        query,
    ) || (has(
        r"(?i)\b(latest|recent|today|current|update|released?)\b",
        query,
    ) && !docs
        && !api
        && !code_lookup(query))
    {
        "current_events"
    } else if (docs || api) && !explicit_code(query) {
        "docs_reference"
    } else if code_lookup(query) {
        "code_lookup"
    } else if docs {
        "docs_reference"
    } else {
        "general"
    }
}
pub fn rich_fetch(query: &str, intent: &str) -> bool {
    intent == "current_events"
        || has(
            r"(?i)\b(research|summari[sz]e|everything|surrounding|comprehensive|deep dive|investigate)\b",
            query,
        )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub mode: String,
    pub search_transcript_results: usize,
    pub search_snippet_chars: usize,
    pub fetch_transcript_facts: usize,
    pub fetch_transcript_preview_lines: usize,
    pub fetch_transcript_line_chars: usize,
}
pub fn profile(window: f64, percent: f64) -> Profile {
    let (mode, results, snippet, facts, lines, chars) =
        if (window > 0. && window <= 32000.) || percent >= 80. {
            ("ultra", 1, 70, 2, 2, 120)
        } else if (window > 0. && window <= 80000.) || percent >= 35. {
            ("tight", 2, 100, 3, 3, 140)
        } else {
            ("normal", 2, 120, 4, 4, 160)
        };
    Profile {
        mode: mode.into(),
        search_transcript_results: results,
        search_snippet_chars: snippet,
        fetch_transcript_facts: facts,
        fetch_transcript_preview_lines: lines,
        fetch_transcript_line_chars: chars,
    }
}
pub fn effective_profile(mut profile: Profile, rich: bool) -> Profile {
    if rich {
        profile.fetch_transcript_facts = (profile.fetch_transcript_facts + 2).min(6);
        profile.fetch_transcript_preview_lines =
            (profile.fetch_transcript_preview_lines + 2).min(6);
        profile.fetch_transcript_line_chars = (profile.fetch_transcript_line_chars + 20).min(220);
    }
    profile
}
pub fn snippet(text: &str, max: usize) -> String {
    if max == 0 {
        String::new()
    } else {
        clipped(&compact(text), max)
    }
}
pub fn limit_preview(text: &str, lines: usize, chars: usize) -> String {
    text.split('\n')
        .map(trim)
        .filter(|v| !v.is_empty())
        .take(lines)
        .map(|v| clipped(v, chars))
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn primary_html(html: &str) -> String {
    for pattern in [
        r"(?is)<article\b[^>]*>([\s\S]*?)</article>",
        r"(?is)<main\b[^>]*>([\s\S]*?)</main>",
        r"(?is)<body\b[^>]*>([\s\S]*?)</body>",
    ] {
        if let Some(c) = rx(pattern).captures(html) {
            let fragment = trim(&c[1]);
            if !fragment.is_empty() {
                return fragment.into();
            }
        }
    }
    html.into()
}
fn extract(html: &str, patterns: &[&'static str]) -> Option<String> {
    for pattern in patterns {
        if let Some(c) = rx(pattern).captures(html) {
            let value = compact(&c[1]);
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}
pub fn title(html: &str) -> Option<String> {
    extract(
        html,
        &[
            r#"(?i)<meta[^>]+property=["']og:title["'][^>]+content=["']([^"']+)["']"#,
            r#"(?i)<meta[^>]+name=["']twitter:title["'][^>]+content=["']([^"']+)["']"#,
            r"(?i)<title[^>]*>([^<]+)</title>",
        ],
    )
}
pub fn meta_description(html: &str) -> Option<String> {
    extract(
        html,
        &[
            r#"(?i)<meta[^>]+name=["']description["'][^>]+content=["']([^"']+)["']"#,
            r#"(?i)<meta[^>]+property=["']og:description["'][^>]+content=["']([^"']+)["']"#,
            r#"(?i)<meta[^>]+name=["']twitter:description["'][^>]+content=["']([^"']+)["']"#,
        ],
    )
}
fn clean(text: &str) -> String {
    // A copy-button label is chrome; `copy()` and prose about copying are data.
    let text = compact(text);
    if matches!(
        text.to_ascii_lowercase().as_str(),
        "copy" | "copy page" | "pagecopy"
    ) {
        String::new()
    } else if text.contains("pageCopy") {
        compact(&replace(
            &replace(
                &replace(&text, r"(?i)\bpageCopy\b", "page Copy"),
                r"(?i)\bcopy page\b",
                " ",
            ),
            r"(?i)\bcopy\b",
            " ",
        ))
    } else {
        text
    }
}

fn noise(text: &str) -> bool {
    let text = compact(text);
    units(&text) < 8
        || any("RESEARCH_PLACEHOLDER_PATTERNS", &text)
        || any("RESEARCH_NOISE_LINE_PATTERNS", &text)
}
fn code(text: &str) -> bool {
    let text = compact(text);
    any("RESEARCH_CODE_LINE_PATTERNS", &text)
        || (has(r"[{};]", &text) && has(r"[=()]", &text))
        || text.contains("=>")
}
fn chrome(text: &str) -> bool {
    let text = clean(text);
    text.is_empty() || any("RESEARCH_DOCS_CHROME_LINE_PATTERNS", &text)
}
fn low_docs(text: &str) -> bool {
    let text = clean(text);
    text.is_empty() || any("RESEARCH_LOW_VALUE_DOCS_PREVIEW_PATTERNS", &text)
}
fn focus_terms(terms: &[String]) -> BTreeSet<String> {
    terms
        .iter()
        .map(|v| trim(v).to_lowercase())
        .filter(|v| units(v) >= 3)
        .collect()
}
fn focus_score(text: &str, focus: &BTreeSet<String>) -> usize {
    if focus.is_empty() {
        return 0;
    }
    let text = text.to_lowercase();
    focus.iter().filter(|t| text.contains(t.as_str())).count()
        + usize::from(has(
            r"\b(when|ensure|once|if|because|rather than|so that|lets you|used to|useful)\b",
            &text,
        ))
}
pub fn preview(markdown: &str, lines: usize, chars: usize, focus: &[String]) -> String {
    let focus = focus_terms(focus);
    let mut seen = BTreeSet::new();
    let mut ranked = vec![];
    for (i, line) in markdown.split('\n').enumerate() {
        let line = clean(line);
        if line.is_empty()
            || noise(&line)
            || chrome(&line)
            || low_docs(&line)
            || !seen.insert(line.to_lowercase())
        {
            continue;
        }
        ranked.push((std::cmp::Reverse(focus_score(&line, &focus)), i, line));
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    if ranked.is_empty() {
        limit_preview(markdown, lines, chars)
    } else {
        ranked
            .iter()
            .take(lines)
            .map(|(_, _, v)| clipped(v, chars))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
fn remove_marker(text: &str) -> String {
    replace(
        &replace(&replace(text, r"^#{1,6}\s+", ""), r"^[-*+]\s+", ""),
        r"^\d+\.\s+",
        "",
    )
}
pub fn facts(markdown: &str, max: usize, chars: usize, focus: &[String]) -> Vec<String> {
    if max == 0 {
        return vec![];
    }
    let focus = focus_terms(focus);
    let mut seen = BTreeSet::new();
    let mut ranked = vec![];
    for (i, line) in markdown
        .split('\n')
        .map(trim)
        .filter(|v| !v.is_empty())
        .enumerate()
    {
        let candidate = clean(&remove_marker(line));
        if units(&candidate) < 24
            || any("RESEARCH_PLACEHOLDER_PATTERNS", &candidate)
            || has(r"^[`*_#>\-\d.\s]+$", &candidate)
            || chrome(&candidate)
            || has(
                r"(?i)\b(?:function|window\.|document\.|localStorage|sessionStorage|dataLayer|gtag|addEventListener|querySelector)\b",
                &candidate,
            )
            || (has(r"[{};]", &candidate) && has(r"[=()]", &candidate))
        {
            continue;
        }
        let candidate = snippet(&candidate, chars);
        if seen.insert(candidate.to_lowercase()) {
            ranked.push((
                std::cmp::Reverse(focus_score(&candidate, &focus)),
                i,
                candidate,
            ));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().take(max).map(|(_, _, v)| v).collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Coverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub present: Vec<String>,
    pub missing: Vec<String>,
    pub score: usize,
}
pub fn coverage(text: &str, intent: Option<&str>, kind: Option<&str>) -> Option<Coverage> {
    let normalized = compact(text);
    let kind=kind.or_else(||if intent==Some("current_events"){Some("current_events")}else if has(r"(?i)\b(?:pricing|price|cost|input price|output price|per\s+(?:1m|million|token)|billing)\b",&normalized){Some("pricing")}else if has(r"(?i)\b(?:changelog|release notes?|released|migration|breaking changes?|upgrade guide)\b",&normalized){Some("release")}else if has(r"(?i)\b(?:TypeError|ReferenceError|SyntaxError|RangeError|Error|Exception):\s*[^.!?]+|\btraceback\b|\bworkaround\b",&normalized){Some("error")}else if intent==Some("docs_reference"){Some("docs")}else{None})?;
    let group = match kind {
        "current_events" => "CURRENT_EVENTS_COVERAGE_SLOTS",
        "docs" => "DOCS_COVERAGE_SLOTS",
        "pricing" => "PRICING_COVERAGE_SLOTS",
        "release" => "RELEASE_COVERAGE_SLOTS",
        "error" => "ERROR_COVERAGE_SLOTS",
        _ => return None,
    };
    let mut present = vec![];
    let mut missing = vec![];
    for r in rules(group) {
        if r.pattern.is_match(&normalized) {
            present.push(r.label.clone())
        } else {
            missing.push(r.label.clone())
        }
    }
    Some(Coverage {
        kind: Some(kind.into()),
        score: present.len(),
        present,
        missing,
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessInput {
    pub markdown: String,
    #[serde(default)]
    pub facts: Vec<String>,
    pub preview: Option<String>,
    pub title: Option<String>,
    pub meta_description: Option<String>,
    pub intent: Option<String>,
    pub coverage_kind: Option<String>,
}
pub fn assess(input: &AssessInput) -> Assessment {
    let mut combined = vec![];
    for value in [&input.title, &input.meta_description, &input.preview]
        .into_iter()
        .flatten()
    {
        if !trim(value).is_empty() {
            combined.push(value.as_str());
        }
    }
    combined.extend(
        input
            .facts
            .iter()
            .filter(|v| !trim(v).is_empty())
            .map(String::as_str),
    );
    if !trim(&input.markdown).is_empty() {
        combined.push(&input.markdown);
    }
    let combined = combined.join("\n");
    let coverage = coverage(
        &combined,
        input.intent.as_deref(),
        input.coverage_kind.as_deref(),
    );
    for r in rules("RESEARCH_PLACEHOLDER_PATTERNS") {
        if r.pattern.is_match(&combined) {
            return Assessment {
                status: "placeholder".into(),
                reason: Some(r.label.clone()),
                coverage,
            };
        }
    }
    let preview = input.preview.as_deref().unwrap_or_default();
    let preview_lines: Vec<_> = preview
        .split('\n')
        .map(trim)
        .filter(|v| !v.is_empty())
        .collect();
    let informative = preview_lines
        .iter()
        .filter(|v| units(v) >= 40 && !noise(v))
        .count();
    let code_preview = preview_lines.iter().filter(|v| code(v)).count();
    let content: Vec<_> = input
        .markdown
        .split('\n')
        .map(trim)
        .filter(|v| !v.is_empty() && !noise(v))
        .collect();
    let ratio = if content.is_empty() {
        0.
    } else {
        content.iter().filter(|v| code(v)).count() as f64 / content.len() as f64
    };
    let prose = input.facts.iter().filter(|v| !code(v)).count();
    let (status, reason) = if ((code_preview >= 2.max(preview_lines.len().saturating_sub(1))
        && !preview_lines.is_empty())
        || ratio >= 0.6)
        && prose < 2
    {
        (
            "thin",
            Some("code-heavy extract without enough supporting explanation"),
        )
    } else if input.facts.len() >= 2
        || informative >= 3
        || (units(&compact(&combined)) >= 700 && prose >= 1)
    {
        ("rich", None)
    } else {
        ("thin", Some("limited article text extracted"))
    };
    Assessment {
        status: status.into(),
        reason: reason.map(str::to_string),
        coverage,
    }
}
fn canonical(text: &str) -> String {
    trim(&replace(
        &remove_marker(&clean(text)),
        r"\s+[–-]\s+[A-Za-z0-9][A-Za-z0-9 ._-]*$",
        "",
    ))
    .to_lowercase()
}
fn excerpt(preview: &str, title: Option<&str>, facts: &[String], profile: &Profile) -> String {
    if preview.is_empty() {
        return String::new();
    }
    let dedup = has(
        r"(?i)\b(api|reference|hook|hooks|component|components)\b",
        title.unwrap_or_default(),
    ) || preview.split('\n').any(|v| has(r"`[^`]+`", v))
        || facts.iter().any(|v| has(r"`[^`]+`", v));
    if !dedup {
        return limit_preview(
            preview,
            profile.fetch_transcript_preview_lines,
            profile.fetch_transcript_line_chars,
        );
    }
    let mut omit = BTreeSet::new();
    if let Some(title) = title {
        omit.insert(canonical(title));
        if let Some(head) = title.split(['–', '-']).next() {
            omit.insert(canonical(head));
        }
    }
    for fact in facts {
        omit.insert(canonical(fact));
    }
    let mut seen = BTreeSet::new();
    let mut lines = vec![];
    for line in preview.split('\n').map(trim).filter(|v| !v.is_empty()) {
        let norm = canonical(line);
        if norm.is_empty() || !seen.insert(norm.clone()) || omit.contains(&norm) {
            continue;
        }
        lines.push(line);
        if lines.len() >= profile.fetch_transcript_preview_lines {
            break;
        }
    }
    let text = if lines.is_empty() {
        preview.into()
    } else {
        lines.join("\n")
    };
    limit_preview(
        &text,
        profile.fetch_transcript_preview_lines,
        profile.fetch_transcript_line_chars,
    )
}
fn coverage_guidance(coverage: &Coverage) -> Vec<String> {
    let mut lines = vec![format!(
        "Coverage: {}",
        if coverage.present.is_empty() {
            "none".into()
        } else {
            coverage.present.join(", ")
        }
    )];
    if !coverage.missing.is_empty() {
        lines.push(format!("Missing: {}", coverage.missing.join(", ")));
    }
    lines.push("Coverage describes this extract only; it does not establish completion of the active task or independently verify the source's claims.".into());
    lines
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchTranscript {
    pub url: String,
    pub title: Option<String>,
    #[serde(default)]
    pub facts: Vec<String>,
    pub preview: Option<String>,
    pub profile: Profile,
    #[serde(default)]
    pub truncated: bool,
    pub meta_description: Option<String>,
    pub status: Option<String>,
    pub status_reason: Option<String>,
    pub intent: Option<String>,
    pub authoritative: Option<bool>,
    pub coverage: Option<Coverage>,
}
pub fn fetch_transcript(input: &FetchTranscript) -> String {
    let mut lines = vec![format!("Fetched source: {}", input.url)];
    if let Some(title) = input.title.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("Title: {title}"));
    }
    if input.status.as_deref() == Some("placeholder") {
        lines.push("Fetch status: placeholder/interstitial content".into());
        if let Some(reason) = &input.status_reason {
            lines.push(format!("Reason: {reason}"))
        }
        if let Some(meta) = &input.meta_description {
            lines.push(format!("Meta description: {meta}"))
        }
        lines.push("Do not rely on this source as evidence. Choose another URL or source.".into());
        return lines.join("\n");
    }
    if let Some(meta) = &input.meta_description
        && input.facts.is_empty()
    {
        lines.push(format!(
            "Summary: {}",
            snippet(meta, input.profile.fetch_transcript_line_chars)
        ));
    }
    if !input.facts.is_empty() {
        lines.push("Key facts:".into());
        for fact in input
            .facts
            .iter()
            .take(input.profile.fetch_transcript_facts)
        {
            lines.push(format!("- {fact}"))
        }
    }
    let preview = excerpt(
        input.preview.as_deref().unwrap_or_default(),
        input.title.as_deref(),
        &input.facts,
        &input.profile,
    );
    if !preview.is_empty() {
        lines.push("Key excerpt:".into());
        lines.push(preview);
    }
    match input.status.as_deref(){
        Some("thin")=>{lines.push(format!("Fetch status: thin extract{}",input.status_reason.as_ref().filter(|v|!v.is_empty()).map(|v|format!(" ({v})")).unwrap_or_default()));lines.push("This source yielded limited article text. Use another source if key facts are still missing.".into());},
        Some("rich")=>match (input.intent.as_deref(),input.authoritative){
            (Some("current_events"),Some(true))=>{lines.push("This extract is from an authoritative source; verify that its facts address the active task.".into());lines.push("Source classification and coverage describe this extract, not completion of the active task.".into());},
            (Some("current_events"),_)=>lines.push("This source summary is usable as a fallback, but corroborate with an authoritative or independent source before treating it as final.".into()),
            (Some("docs_reference"),Some(true))=>lines.push("This extract is from an authoritative documentation page; its applicability depends on the active task and version.".into()),
            _=>lines.push("This source extract is available as evidence for the active task.".into()),
        },
        _ if input.truncated||input.profile.mode!="normal"=>lines.push("This fetch was compacted to preserve context.".into()),_=>{},
    }
    if let Some(coverage) = &input.coverage {
        lines.extend(coverage_guidance(coverage));
    }
    lines.join("\n")
}
#[derive(Clone, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    #[serde(default)]
    pub authoritative: bool,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub intent: Option<String>,
    #[serde(default)]
    pub prefer_corroboration: bool,
    pub original_query: Option<String>,
    pub search_strategy: Option<String>,
    #[serde(default)]
    pub follow_up_queries: Vec<String>,
}
pub fn search_transcript(
    query: &str,
    results: &[SearchResult],
    profile: &Profile,
    options: &SearchOptions,
) -> String {
    let single = options.intent.as_deref() == Some("docs_reference")
        && results.first().is_some_and(|v| v.authoritative)
        && has(
            r"(?i)\b(parameter|parameters|option|options|argument|arguments|returns?|signature|hook|api|what it does|when .* useful)\b",
            query,
        );
    let displayed = &results[..results.len().min(if single {
        1
    } else {
        profile.search_transcript_results
    })];
    let mut lines = vec![format!("Web search: {query}")];
    if let Some(original) = &options.original_query
        && !original.is_empty()
        && trim(original) != trim(query)
    {
        lines.push(format!("Rewritten from: {}", trim(original)));
    }
    if let Some(strategy) = &options.search_strategy
        && !strategy.is_empty()
    {
        lines.push(format!("Search strategy: {strategy}"));
    }
    if displayed.is_empty() {
        lines.push("No results found.".into());
        if let Some(next) = options.follow_up_queries.first() {
            lines.push(format!("Try a narrower query next: {next}"));
        }
        return lines.join("\n");
    }
    lines.push("Top sources:".into());
    for (i, result) in displayed.iter().enumerate() {
        lines.push(format!(
            "{}. {}{} — {}",
            i + 1,
            result.title,
            if result.authoritative {
                " [authoritative]"
            } else {
                ""
            },
            result.url
        ));
        if let Some(text) = &result.snippet {
            let text = snippet(text, profile.search_snippet_chars);
            if !text.is_empty() {
                lines.push(format!("   {text}"));
            }
        }
    }
    if results.len() > displayed.len() {
        lines.push(format!(
            "[{} additional result(s) omitted]",
            results.len() - displayed.len()
        ));
    }
    if let Some(top) = displayed.first()
        && !top.url.is_empty()
    {
        if options.intent.as_deref() == Some("docs_reference") && top.authoritative {
            lines.push(if single {
                "Only if a key concept is still missing, fetch the top authoritative URL above."
                    .into()
            } else {
                format!(
                    "Only if a key concept is still missing, next step: webFetch {}",
                    top.url
                )
            });
        } else {
            lines.push(format!("Next step: webFetch {}", top.url));
        }
        if options.intent.as_deref() == Some("current_events") {
            lines.push(if options.prefer_corroboration{"Fetch the strongest source first. Only fetch one independent corroborating source if the first source is blocked, thin, or still leaves a key fact missing."}else{"Fetch one strongest source first. Do not fetch multiple sources in parallel unless the first source is blocked, thin, or clearly missing a key fact."}.into());
            lines.push("For a brief current-events summary, if the first fetched source is rich and specific, answer from it directly. Treat the other top search results as corroborating candidates unless a key fact still needs confirmation.".into());
            if let Some(next) = options.follow_up_queries.first() {
                lines.push(format!("If the first source is blocked or thin, do not repeat this broad search. Use a narrower follow-up query such as: {next}"));
            }
        }
    }
    lines.join("\n")
}

/// The reference's lightweight HTML conversion, including its entity ordering.
/// This is text extraction, not a sanitizer for rendering HTML in a browser.
pub fn html_markdown(html: &str) -> Result<String, crate::DomainError> {
    if html.len() > 1_000_000 {
        return Err(crate::DomainError::LimitExceeded(
            "research document".into(),
        ));
    }
    let mut md = html.to_string();
    for pattern in [
        r"(?is)<script\b[^>]*>[\s\S]*?</script>",
        r"(?is)<style\b[^>]*>[\s\S]*?</style>",
        r"(?is)<noscript\b[^>]*>[\s\S]*?</noscript>",
        r"(?is)<template\b[^>]*>[\s\S]*?</template>",
        r"(?is)<svg\b[^>]*>[\s\S]*?</svg>",
        r"(?s)<!--[\s\S]*?-->",
    ] {
        md = replace(&md, pattern, " ");
    }
    // Regex's linear-time engine has no backreferences. Match complete h1..h6
    // pairs explicitly while preserving the source's first-match/global order.
    static HEADING: LazyLock<Regex> = LazyLock::new(|| {
        compile(
            r"(?is)<h1[^>]*>(.*?)</h1>|<h2[^>]*>(.*?)</h2>|<h3[^>]*>(.*?)</h3>|<h4[^>]*>(.*?)</h4>|<h5[^>]*>(.*?)</h5>|<h6[^>]*>(.*?)</h6>",
            false,
        )
    });
    md = HEADING
        .replace_all(&md, |captures: &regex::Captures<'_>| {
            let level = (1..=6).find(|i| captures.get(*i).is_some()).unwrap();
            format!("\n\n{} {}\n\n", "#".repeat(level), trim(&captures[level]))
        })
        .into_owned();
    for (pattern, to) in [
        (r"(?is)<p[^>]*>(.*?)</p>", "\n\n$1\n\n"),
        (r"(?is)<a[^>]*>(.*?)</a>", "$1"),
        (r"(?i)<(ul|ol)[^>]*>", "\n"),
        (r"(?i)</(ul|ol)[^>]*>", "\n"),
        (r"(?is)<li[^>]*>(.*?)</li>", "- $1\n"),
        (r"(?i)<code[^>]*>(.*?)</code>", "`$1`"),
        (
            r"(?is)<pre[^>]*><code[^>]*>(.*?)</code></pre>",
            "\n```\n$1\n```\n",
        ),
        (r"<[^>]+>", ""),
    ] {
        md = replace(&md, pattern, to);
    }
    for (from, to) in [
        ("&nbsp;", " "),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&#39;", "'"),
        ("&quot;", "\""),
    ] {
        md = md.replace(from, to);
    }
    Ok(trim(&replace(&md, r"\n{3,}", "\n\n")).into())
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlHint {
    pub intent: String,
    #[serde(default)]
    pub rich_fetch: bool,
    pub authoritative: Option<bool>,
    #[serde(default)]
    pub focus_terms: Vec<String>,
    pub coverage_kind: Option<String>,
}
/// Host-populated fetch facts; transport implementations must validate provenance.
pub struct FetchedDocument<'a> {
    pub url: &'a str,
    pub final_url: &'a str,
    pub status_code: u16,
    pub content_type: Option<&'a str>,
    pub cache_hit: bool,
    pub bytes: &'a [u8],
}
pub struct FetchProjection {
    pub result: Value,
    pub status: Option<String>,
}
pub fn fetch_projection(
    document: FetchedDocument<'_>,
    base: Profile,
    hint: Option<&UrlHint>,
) -> Result<FetchProjection, crate::DomainError> {
    use serde_json::json;
    if document.bytes.len() > 1_000_000
        || document.url.len() > 8192
        || document.final_url.len() > 8192
        || document.content_type.is_some_and(|v| v.len() > 4096)
        || hint.is_some_and(|h| {
            h.focus_terms.len() > 64 || h.focus_terms.iter().any(|v| v.len() > 256)
        })
    {
        return Err(crate::DomainError::LimitExceeded(
            "research document projection".into(),
        ));
    }
    let mut details = json!({"url":document.url,"finalUrl":document.final_url,"statusCode":document.status_code,"cacheHit":document.cache_hit});
    if let Some(content_type) = document.content_type {
        details["contentType"] = json!(content_type);
        if [
            "image/",
            "application/pdf",
            "application/octet-stream",
            "video/",
            "audio/",
        ]
        .iter()
        .any(|prefix| content_type.starts_with(prefix))
        {
            return Ok(FetchProjection {
                result: json!({"isError":true,"details":details,"content":[{"type":"text","text":format!("Binary content detected ({content_type}). WebFetch only supports text/HTML content.")}]}),
                status: None,
            });
        }
    }
    let decoded = String::from_utf8_lossy(document.bytes);
    let content = decoded.strip_prefix('\u{feff}').unwrap_or(&decoded);
    let is_html = document.content_type.is_some_and(|t| {
        let t = t.to_ascii_lowercase();
        t.starts_with("text/html") || t.starts_with("application/xhtml+xml")
    }) || document.content_type.is_none()
        && has(
            r"(?i)<(?:!doctype|html|head|body|main|article|p)[ >]",
            content,
        );
    let extracted = document::extract(content, is_html, document.final_url);
    let meta = extracted.description;
    let markdown = extracted.markdown;
    let focus = hint.map(|h| h.focus_terms.as_slice()).unwrap_or_default();
    let preview = preview(&markdown, 12, 180, focus);
    let compression = effective_profile(base, hint.is_some_and(|h| h.rich_fetch));
    let mut facts = facts(
        &markdown,
        compression.fetch_transcript_facts + 1,
        compression.fetch_transcript_line_chars,
        focus,
    );
    let title = extracted.title.or_else(|| {
        rx(r"(?m)^#\s+(.+)$")
            .captures(&markdown)
            .map(|c| trim(&c[1]).to_string())
    });
    if let Some(meta) = &meta
        && units(meta) >= 40
        && !hint.is_some_and(|h| h.intent == "docs_reference")
        && !facts
            .iter()
            .any(|f| f.to_lowercase() == meta.to_lowercase())
    {
        facts.insert(0, meta.clone());
    }
    facts.truncate(compression.fetch_transcript_facts);
    let assessment = assess(&AssessInput {
        markdown: markdown.clone(),
        facts: facts.clone(),
        preview: Some(preview.clone()),
        title: title.clone(),
        meta_description: meta.clone(),
        intent: hint.map(|h| h.intent.clone()),
        coverage_kind: hint.and_then(|h| h.coverage_kind.clone()),
    });
    if let Some(title) = &title {
        details["title"] = json!(title);
    }
    details["facts"] = json!(facts);
    details["preview"] = json!(preview);
    let anchor_focus = url::Url::parse(document.url)
        .ok()
        .and_then(|u| u.fragment().map(str::to_string))
        .map(|s| {
            s.split(['-', '.', '_'])
                .filter(|t| t.len() >= 3)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut excerpt_focus = focus.to_vec();
    excerpt_focus.extend(anchor_focus);
    let excerpt = document::excerpt(
        &markdown,
        &excerpt_focus,
        match compression.mode.as_str() {
            "ultra" => 3500,
            "tight" => 7000,
            _ => 12000,
        },
    );
    details["truncated"] = json!(excerpt.truncated);
    details["extractionMethod"] = json!(if is_html {
        "dom-markdown-v1"
    } else {
        "plain-text-v1"
    });
    details["extractedChars"] = json!(markdown.chars().count());
    details["excerptChars"] = json!(excerpt.text.chars().count());
    details["compressionMode"] = json!(compression.mode);
    if let Some(hint) = hint {
        details["intent"] = json!(hint.intent);
        if let Some(kind) = &hint.coverage_kind {
            details["coverageKind"] = json!(kind);
        }
    }
    let authoritative = hint.is_some_and(|h| h.authoritative == Some(true));
    details["authoritative"] = json!(authoritative);
    if let Some(meta) = &meta {
        details["metaDescription"] = json!(meta);
    }
    details["status"] = json!(assessment.status);
    if let Some(reason) = &assessment.reason {
        details["statusReason"] = json!(reason);
    }
    if let Some(coverage) = &assessment.coverage {
        details["coverage"] = json!(coverage);
    }
    let mut text = fetch_transcript(&FetchTranscript {
        url: document.url.into(),
        title,
        facts,
        preview: Some(preview),
        profile: compression,
        truncated: excerpt.truncated,
        meta_description: meta,
        status: Some(assessment.status.clone()),
        status_reason: assessment.reason,
        intent: hint.map(|h| h.intent.clone()),
        authoritative: Some(authoritative),
        coverage: assessment.coverage,
    });
    if !excerpt.text.trim().is_empty() {
        text.push_str("\n\nSource excerpt (untrusted page content):\n");
        text.push_str(&excerpt.text);
    }
    let unavailable = assessment.status == "placeholder";
    if unavailable {
        details["error"] = json!("The page did not yield usable source content.");
    }
    let mut result = json!({"details":details,"content":[{"type":"text","text":text}]});
    if unavailable {
        result["isError"] = json!(true);
    }
    Ok(FetchProjection {
        result,
        status: Some(assessment.status),
    })
}
