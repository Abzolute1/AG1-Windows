//! Search planning, ranking and provider response parsing. No network or secrets.
pub mod duckduckgo_web;
mod selection;
use super::research::{self, compact, compile, has, replace, rx, trim};
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};
use url::Url;
static DATA: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../resources/websearch-data.json"))
        .expect("pinned websearch data")
});
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultRow {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub snippet: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub original_query: String,
    pub effective_query: String,
    pub strategy: String,
    pub follow_up_queries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avoided_domains: Option<Vec<String>>,
    pub freshness: String,
}
struct Fallback {
    pattern: regex::Regex,
    domains: Vec<String>,
    results: Vec<ResultRow>,
}
static FALLBACKS: LazyLock<Vec<Fallback>> = LazyLock::new(|| {
    DATA["data"]["OFFICIAL_DOC_FALLBACKS"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Fallback {
            pattern: compile(v["match"]["pattern"].as_str().unwrap(), true),
            domains: serde_json::from_value(v["preferredDomains"].clone()).unwrap(),
            results: serde_json::from_value(v["results"].clone()).unwrap(),
        })
        .collect()
});
fn group(query: &str) -> Option<&'static Fallback> {
    let preferred = if has(r"(?i)\blm studio\b", query) {
        Some("lmstudio.ai")
    } else if has(r"(?i)\bnvidia|nemotron\b", query) {
        Some("build.nvidia.com")
    } else {
        None
    };
    FALLBACKS.iter().find(|g| {
        if let Some(domain) = preferred {
            g.domains.iter().any(|d| d == domain)
        } else {
            g.pattern.is_match(query)
        }
    })
}
fn unique(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values.filter(|v| seen.insert(v.clone())).collect()
}
fn terms(query: &str, docs: bool) -> Vec<String> {
    let pattern = if docs {
        r"[a-z][a-z0-9]*"
    } else {
        r"[a-z0-9][a-z0-9.+#-]*"
    };
    let query = query.to_lowercase();
    let stop = DATA["data"][if docs {
        "DOCS_QUERY_STOPWORDS"
    } else {
        "SEARCH_QUERY_STOPWORDS"
    }]
    .as_array()
    .unwrap();
    unique(
        rx(pattern)
            .find_iter(&query)
            .map(|m| {
                let word = m.as_str();
                if docs && word.ends_with('s') && word.len() > 4 {
                    word[..word.len() - 1].to_string()
                } else {
                    word.into()
                }
            })
            .filter(|v| {
                v.len() >= if docs { 4 } else { 3 } && !stop.iter().any(|s| s.as_str() == Some(v))
            }),
    )
}
pub fn docs_terms(query: &str) -> Vec<String> {
    terms(query, true)
}
pub fn strongest(query: &str) -> bool {
    has(
        r"(?i)\b(strongest|best|authoritative|independent)\s+sources?\b",
        query,
    ) || has(r"(?i)\bcite\b.*\bsources?\b", query)
        || has(r"(?i)\bcorroborat(?:e|ion|ing)\b", query)
}
fn error_query(query: &str) -> Option<String> {
    if has(r#""[^"]{8,}"|'[^']{8,}'|`[^`]{8,}`"#, query) {
        return None;
    }
    let pattern = rx(
        r"(?i)\b(?:TypeError|ReferenceError|SyntaxError|RangeError|Error|Exception):\s*[^!?\n]+",
    );
    let fallback = rx(r"\b[A-Z][A-Za-z]+Error:\s*[^!?\n]+");
    let message = compact(
        pattern
            .find(query)
            .or_else(|| fallback.find(query))?
            .as_str(),
    );
    if crate::task::units(&message) < 12 {
        return None;
    }
    let context = terms(query, false)
        .into_iter()
        .filter(|t| !message.to_lowercase().contains(t))
        .take(5)
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!(
        "\"{message}\"{}",
        if context.is_empty() {
            String::new()
        } else {
            format!(" {context}")
        }
    ))
}
fn append(base: &str, terms: &[&str]) -> String {
    let lower = base.to_lowercase();
    let additions = terms
        .iter()
        .filter(|t| !lower.contains(&t.to_lowercase()))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    if additions.is_empty() {
        base.into()
    } else {
        format!("{base} {additions}")
    }
}
fn normalize_docs(query: &str) -> String {
    let query = trim(query);
    if has(r"(?i)\bopenai\b", query) && has(r"(?i)\bapi\b", query) && has(r"(?i)\bpricing\b", query)
    {
        "OpenAI API pricing official website".into()
    } else if (has(r"(?i)\bnvidia\b", query)
        && has(r"(?i)\b(nemotron|api|provider|docs?)\b", query))
        || (has(r"(?i)\blm studio\b", query)
            && has(r"(?i)\b(openai[- ]compatible|endpoint|api|server)\b", query))
        || has(r"(?i)\breact\b", query)
        || has(r"(?i)\btypescript\b|\btsconfig\b", query)
        || group(query).is_some()
    {
        append(query, &["official docs"])
    } else {
        query.into()
    }
}
pub fn coverage_kind(query: &str, intent: &str) -> Option<&'static str> {
    if intent == "current_events" {
        Some("current_events")
    } else if has(
        r"(?i)\b(?:pricing|price|cost|billing|input price|output price|per token|per 1m)\b",
        query,
    ) {
        Some("pricing")
    } else if has(
        r"(?i)\b(?:release|changelog|migration|breaking changes?|upgrade guide|release notes?)\b",
        query,
    ) {
        Some("release")
    } else if error_query(query).is_some()
        || has(
            r"(?i)\b(?:TypeError|ReferenceError|SyntaxError|RangeError|Exception|traceback|stack trace)\b",
            query,
        )
    {
        Some("error")
    } else if intent == "docs_reference" {
        Some("docs")
    } else {
        None
    }
}
pub fn plan(query: &str, year: u32) -> Result<Plan, DomainError> {
    if query.len() > 4096 || !(1970..=9999).contains(&year) {
        return Err(DomainError::LimitExceeded(
            "web search query or year".into(),
        ));
    }
    let intent = research::intent(query);
    let original = trim(query);
    let mut plan = Plan {
        original_query: original.into(),
        effective_query: original.into(),
        strategy: "general web query".into(),
        follow_up_queries: vec![],
        preferred_domains: None,
        avoided_domains: None,
        freshness: "any".into(),
    };
    // Explicit search syntax belongs to the caller. Rewriting it can discard
    // domain restrictions, quoted phrases, exclusions, or exact punctuation.
    if has(
        r#"(?i)(?:\b(?:site|filetype|intitle|inurl):|"[^"]+"|(?:^|\s)-\S+)"#,
        original,
    ) {
        plan.strategy = "explicit search syntax preserved".into();
        return Ok(plan);
    }
    if let Some(error) = error_query(original) {
        plan.effective_query = append(&error, &["fix"]);
        plan.strategy =
            "error-message query preserves the exact error string and adds only targeted context"
                .into();
        plan.follow_up_queries = vec![
            append(&error, &["cause"]),
            append(&error, &["fix", "version"]),
        ];
    } else if intent == "current_events"
        && has(r"(?i)\bclaude code\b", original)
        && has(r"(?i)\bleak\b", original)
    {
        plan.effective_query = "Claude Code source map npm leak March 2026 Anthropic".into();
        plan.strategy="current-events query rewritten to specific entities, leak vector, package channel, and date".into();
        plan.follow_up_queries = [
            "Anthropic Claude Code npm source map leak Chaofan Shou March 2026",
            "Claude Code source code leak npm package source maps analysis",
            "Claude Code leak npm packaging source maps Anthropic response",
        ]
        .map(str::to_string)
        .to_vec();
        plan.freshness = "current".into();
    } else if intent == "current_events" {
        // A date we guessed can turn a current question into a search for made-up
        // future pages. Keep the user's wording intact; explicit dates remain
        // intact because they are already part of the original query.
        plan.strategy =
            "current-events query preserved; verify claims with a primary source".into();
        plan.follow_up_queries = ["official source", "announcement", "release notes"]
            .map(|tail| format!("{original} {tail}"))
            .to_vec();
        plan.freshness = "current".into();
    } else if intent == "docs_reference" {
        plan.effective_query = normalize_docs(original);
        plan.strategy = "documentation query biased toward official reference pages".into();
        plan.follow_up_queries = ["official docs", "reference"]
            .map(|tail| format!("{} {tail}", plan.effective_query))
            .to_vec();
        plan.preferred_domains = group(&plan.effective_query).map(|g| g.domains.clone());
        plan.avoided_domains = Some(
            ["medium.com", "dev.to", "reddit.com", "stackoverflow.com"]
                .map(str::to_string)
                .to_vec(),
        );
        if has(
            r"(?i)\b(latest|current|pricing|release|changelog)\b",
            original,
        ) {
            plan.freshness = "current".into();
        }
    } else if intent == "code_lookup" {
        if !has(
            r"(?i)\b(github|gitlab|source|raw\.githubusercontent|site:github)\b",
            original,
        ) {
            plan.effective_query = append(original, &["github", "source"]);
        }
        plan.strategy =
            "code lookup query biased toward source repositories and raw implementation matches"
                .into();
        plan.follow_up_queries = ["github", "source"]
            .map(|tail| format!("{original} {tail}"))
            .to_vec();
        plan.preferred_domains = Some(
            [
                "github.com",
                "raw.githubusercontent.com",
                "gitlab.com",
                "bitbucket.org",
            ]
            .map(str::to_string)
            .to_vec(),
        );
        plan.avoided_domains = Some(
            [
                "learn.microsoft.com",
                "answers.microsoft.com",
                "stackoverflow.com",
            ]
            .map(str::to_string)
            .to_vec(),
        );
    }
    Ok(plan)
}
pub fn offline_fallback(query: &str, intent: &str) -> Vec<ResultRow> {
    if intent != "docs_reference" {
        return vec![];
    }
    if has(r"(?i)\bopenai\b", query) && has(r"(?i)\bapi\b", query) && has(r"(?i)\bpricing\b", query)
    {
        return vec![
            ResultRow {
                title: "Pricing | OpenAI API".into(),
                url: "https://developers.openai.com/api/docs/pricing".into(),
                snippet: "Official OpenAI API pricing documentation.".into(),
            },
            ResultRow {
                title: "OpenAI Platform Pricing".into(),
                url: "https://platform.openai.com/pricing".into(),
                snippet: "Official OpenAI platform pricing overview.".into(),
            },
        ];
    }
    group(query).map(|g| g.results.clone()).unwrap_or_default()
}
fn parsed(value: &str) -> Option<(String, String)> {
    let url = Url::parse(value).ok()?;
    Some((
        url.host_str().unwrap_or_default().to_lowercase(),
        url.path().to_lowercase(),
    ))
}
fn host_is(host: &str, domains: &str) -> bool {
    domains
        .split('|')
        .any(|d| host == d || host.ends_with(&format!(".{d}")))
}
const REPOS: &str = "github.com|raw.githubusercontent.com|gitlab.com|bitbucket.org";
const WEAK: &str = "tech-insider.org|cybersecuritynews.com|ccleaks.com|aitoolanalysis.com|beankinney.com|osintteam.blog|infosecwriteups.com";
const COMMUNITY: &str =
    "stackoverflow.com|reddit.com|medium.com|dev.to|hashnode.com|youtube.com|x.com|twitter.com";
// A page only gets the authoritative label when its host belongs to its
// publisher. A convincing URL or a high search rank is not enough evidence.
const OFFICIAL: &str = "anthropic.com|openai.com|googleblog.com|microsoft.com|nvidia.com|developer.nvidia.com|qwen.ai|qwenlm.github.io|alibabacloud.com";
const TECH: &str = "varonis.com|arstechnica.com|infoq.com|thehackernews.com|techradar.com|bleepingcomputer.com|wired.com|theregister.com";
fn docs_host(host: &str) -> bool {
    ["docs.", "developer.", "api.", "learn."]
        .iter()
        .any(|p| host.starts_with(p))
}
fn general_host_score(host: &str) -> i64 {
    if host_is(host, TECH) {
        4
    } else if host_is(host, "fortune.com|reuters.com|bloomberg.com|ft.com|wsj.com") {
        2
    } else if host_is(host, "securityweek.com|techspot.com") {
        -3
    } else if host_is(host, "linkedin.com|github.com|raw.githubusercontent.com") {
        -8
    } else if host_is(host, WEAK) {
        -6
    } else if host_is(host, "dev.to|medium.com|substack.com") {
        -4
    } else {
        0
    }
}
fn source_score(url: &str, intent: &str, outcome: Option<&str>) -> i64 {
    let Some((host, path)) = parsed(url) else {
        return -10;
    };
    let mut score = 0;
    // A vendor's own release page should win over commentary about the release.
    if host_is(&host, OFFICIAL) {
        score += 10
    }
    if docs_host(&host) {
        score += 5
    }
    if host_is(
        &host,
        "readthedocs.io|mozilla.org|react.dev|nodejs.org|python.org|golang.org|rust-lang.org",
    ) {
        score += 5
    }
    if has(
        r"(docs|doc|reference|manual|guide|api|spec|specification|changelog|release)",
        &path,
    ) {
        score += 3
    }
    if COMMUNITY.split('|').any(|d| host.contains(d)) {
        score -= 4
    }
    if ["duckduckgo.com", "google.com", "bing.com"]
        .iter()
        .any(|d| host.contains(d))
    {
        score -= 6
    }
    if intent == "code_lookup" {
        if host_is(&host, REPOS) {
            score += 8
        }
        if has(
            r"(/blob/|/raw/|/tree/|/src/|/packages/|/commits?/|/pull/)",
            &path,
        ) {
            score += 4
        }
        if ["docs.", "learn.", "developer."]
            .iter()
            .any(|p| host.starts_with(p))
        {
            score -= 3
        }
        if [
            "microsoft.com",
            "learn.microsoft.com",
            "answers.microsoft.com",
        ]
        .iter()
        .any(|d| host.contains(d))
        {
            score -= 4
        }
    }
    if intent == "current_events" {
        if has(
            r"(/news/|/blog/|/release|/releases/|/announcement|/announcements/|/press/|/security/|/incident|/advisory)",
            &path,
        ) {
            score += 4
        }
        if has(
            r"(leak|incident|breach|outage|security|advisory|analysis|report|202\d)",
            &path,
        ) {
            score += 4
        }
        score += general_host_score(&host);
        if host_is(&host, "dev.to|medium.com|substack.com") {
            score -= 4
        }
        if has(
            r"(\bnews\b|\bblog\b|\bpress\b|\bsecurity\b|\bincident\b|\badvisory\b)",
            &host,
        ) {
            score += 2
        }
        if ["docs.", "learn.", "developer."]
            .iter()
            .any(|p| host.starts_with(p))
            || has(r"(docs|reference|manual|guide|api|spec)", &path)
        {
            score -= 4
        }
        if host_is(&host, REPOS) {
            score -= 5
        }
    }
    if intent == "docs_reference" {
        if host_is(
            &host,
            "react.dev|nextjs.org|nodejs.org|developer.mozilla.org|docs.python.org|python.org|typescriptlang.org|www.typescriptlang.org|rust-lang.org|go.dev|angular.dev|vuejs.org|docs.github.com|docs.npmjs.com",
        ) {
            score += 10
        }
        if has(
            r"(/reference/|/docs/|/doc/|/api/|/guide/|/manual/|/learn/)",
            &path,
        ) {
            score += 4
        }
        if has(r"(/news/|/blog/|/announcement|/press/)", &path) {
            score -= 3
        }
        if has(r"(/blogs?/)", &path) {
            score -= 4
        }
        if host_is(&host, "medium.com|dev.to|telerik.com|substack.com") {
            score -= 5
        }
        if host_is(&host, REPOS) {
            score -= 2
        }
    }
    score
        + match outcome {
            Some("rich") => 6,
            Some("thin") => -1,
            Some("placeholder") => -8,
            _ => 0,
        }
}
fn source_kind(url: &str, intent: &str) -> &'static str {
    let Some((host, path)) = parsed(url) else {
        return "general";
    };
    if host_is(&host, OFFICIAL) {
        "official"
    } else if ["docs.", "developer.", "api."]
        .iter()
        .any(|p| host.starts_with(p))
        || has(r"(/docs?/|/reference/|/api/|/manual/)", &path)
    {
        "docs"
    } else if host_is(&host, REPOS) {
        "source_repo"
    } else if host_is(
        &host,
        "dev.to|medium.com|substack.com|reddit.com|x.com|twitter.com|youtube.com",
    ) {
        "community"
    } else if host_is(&host, WEAK) {
        "weak"
    } else if intent == "current_events"
        && has(
            r"(analysis|breakdown|deep-dive|postmortem|forensic|investigation|research|threat)",
            &path,
        )
    {
        "analysis"
    } else if intent == "current_events"
        && has(
            r"(/news/|/article/|/articles/|/press/|/security/|/incident|/advisory)",
            &path,
        )
    {
        "news"
    } else {
        "general"
    }
}
fn fetch_risk(url: &str, outcome: Option<&str>) -> &'static str {
    let Some((host, path)) = parsed(url) else {
        return "high";
    };
    if outcome == Some("placeholder") {
        "high"
    } else if host_is(
        &host,
        "fortune.com|bloomberg.com|wsj.com|ft.com|linkedin.com|x.com|twitter.com",
    ) {
        "medium"
    } else if ["docs.", "developer.", "api."]
        .iter()
        .any(|p| host.starts_with(p))
        || host_is(
            &host,
            "github.com|raw.githubusercontent.com|react.dev|nodejs.org|docs.python.org|www.typescriptlang.org|developer.mozilla.org|docs.npmjs.com",
        )
        || has(r"(/docs?/|/reference/|/api/|/manual/)", &path)
    {
        "low"
    } else {
        "medium"
    }
}
fn score_reasons(
    result: &ResultRow,
    intent: &str,
    query: &str,
    outcome: Option<&str>,
) -> Vec<String> {
    let Some((host, path)) = parsed(&result.url) else {
        return vec![];
    };
    let text = format!("{}\n{}\n{}", result.title, result.snippet, result.url).to_lowercase();
    let mut reasons = vec![];
    if host_is(
        &host,
        "anthropic.com|openai.com|googleblog.com|microsoft.com|nvidia.com|developer.nvidia.com|react.dev|nodejs.org|python.org|typescriptlang.org|www.typescriptlang.org|mozilla.org|npmjs.com|lmstudio.ai",
    ) {
        reasons.push("official_domain")
    }
    if host.starts_with("docs.")
        || host.starts_with("developer.")
        || has(r"(/docs?/|/reference/|/api/|/manual/|/learn/)", &path)
    {
        reasons.push("docs_path")
    }
    if host_is(&host, REPOS) {
        reasons.push("source_repo")
    }
    if has(
        r"\b(202\d|jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\b",
        &text,
    ) {
        reasons.push("recent_date_signal")
    }
    if has(
        r"(analysis|breakdown|deep-dive|postmortem|forensic|investigation|research|threat)",
        &path,
    ) || has(
        r"\b(analysis|breakdown|postmortem|investigation|threat labs?)\b",
        &text,
    ) {
        reasons.push("analysis_path")
    }
    if host_is(&host, TECH) || host_is(&host, "krebsonsecurity.com") {
        reasons.push("known_strong_tech_outlet")
    }
    if host_is(&host, COMMUNITY) {
        reasons.push("community_source_penalty")
    }
    if host_is(&host, WEAK) {
        reasons.push("known_weak_domain_penalty")
    }
    if outcome == Some("placeholder") {
        reasons.push("previous_placeholder_penalty")
    }
    if outcome == Some("rich") {
        reasons.push("previous_rich_fetch_bonus")
    }
    if terms(query, false).iter().any(|t| text.contains(t)) {
        reasons.push("query_term_match")
    }
    if intent == "docs_reference" && source_kind(&result.url, intent) == "official" {
        reasons.push("official_domain")
    }
    unique(reasons.into_iter().map(str::to_string))
}
fn strongest_host(host: &str) -> i64 {
    let mut score = 0;
    if host_is(host, TECH) || host_is(host, "krebsonsecurity.com") {
        score += 5
    }
    if host_is(
        host,
        "anthropic.com|openai.com|googleblog.com|microsoft.com|blog.github.com",
    ) {
        score += 4
    }
    if host_is(host, "fortune.com|bloomberg.com|reuters.com|ft.com|wsj.com") {
        score += 1
    }
    if host_is(
        host,
        "tech-insider.org|cybersecuritynews.com|analyticsindiamag.com|osintteam.blog|infosecwriteups.com",
    ) {
        score -= 8
    }
    if host_is(host, "dev.to|medium.com|substack.com") {
        score -= 6
    }
    score
}
fn docs_content(result: &ResultRow, query: &str) -> i64 {
    let terms = docs_terms(query);
    let title = result.title.to_lowercase();
    let snippet = result.snippet.to_lowercase();
    let url = result.url.to_lowercase();
    let mut score = 0;
    if let Some((host, path)) = parsed(&result.url)
        && has(r"(?i)\bopenai\b", query)
        && has(r"(?i)\bapi\b", query)
        && has(r"(?i)\bpricing\b", query)
    {
        if host == "developers.openai.com" && has(r"/api/docs/pricing/?$", &path) {
            score += 20
        } else if host == "platform.openai.com" && has(r"/pricing/?$", &path) {
            score += 16
        } else if host == "openai.com" && has(r"/api/pricing/?$", &path) {
            score += 6
        } else if host_is(
            &host,
            "aipricing.guru|apicents.com|pricepertoken.com|llmpricingtable.com|tokencalculator.com",
        ) {
            score -= 12
        }
    }
    if terms.is_empty() {
        return score;
    }
    for term in terms {
        score += if title.contains(&term) {
            5
        } else if url.contains(&term) {
            4
        } else if snippet.contains(&term) {
            3
        } else {
            0
        };
    }
    if has(r"\b(hook|hooks|api|reference|action|actions)\b", &title) {
        score += 1
    }
    score
}
fn content_score(result: &ResultRow, intent: &str, query: &str) -> i64 {
    if intent == "docs_reference" {
        return docs_content(result, query);
    }
    if intent != "current_events" {
        return 0;
    }
    let text = format!("{}\n{}", result.title, result.snippet).to_lowercase();
    let mut score = 0;
    if has(
        r"\b(leak|incident|breach|outage|analysis|security|advisory|exposed|published|coverage|report)\b",
        &text,
    ) {
        score += 4
    }
    if has(
        r"\b(202\d|jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\b",
        &text,
    ) {
        score += 2
    }
    if has(r"\b(docs?|reference|manual|guide|tutorial)\b", &text) {
        score -= 3
    }
    // The reference deliberately applies these query checks without /i.
    if has(r"\bclaude code\b", query) && has(r"\bleak\b", query) {
        if has(
            r"\b(npm|registry|source map|sourcemap|map file|512[, ]?000|1[, ]?900|typescript source|source code)\b",
            &text,
        ) {
            score += 6
        }
        if !has(
            r"\b(vulnerability|exploit|malware|attack|typosquatting)\b",
            query,
        ) && has(
            r"\b(vulnerability|exploit|malware|attack campaign|days after the leak|days after source leak)\b",
            &text,
        ) {
            score -= 8
        }
        if !has(r"\b(plans|features|roadmap|buddy|undercover)\b", query)
            && has(r"\b(plans|feature flags|roadmap|undercover|buddy)\b", &text)
        {
            score -= 3
        }
    }
    if strongest(query)
        && let Some((host, path)) = parsed(&result.url)
    {
        let analysis = has(
            r"\b(analysis|breakdown|deep dive|postmortem|forensic|threat labs?|investigation|full analysis|timeline|root cause)\b",
            &text,
        );
        if analysis {
            score += 6
        }
        if has(
            r"(analysis|breakdown|postmortem|forensic|investigation)",
            &path,
        ) {
            score += 3
        }
        if has(r"\b(threat labs?|research)\b", &text) {
            score += 2
        }
        score += strongest_host(&host);
        if has(r"/news/|/article/|/articles/|/press/|/coverage/", &path) && !analysis {
            score -= 3
        }
        if host_is(
            &host,
            "varonis.com|crowdstrike.com|sentinelone.com|unit42.paloaltonetworks.com|securelist.com|mandiant.com",
        ) {
            score += 2
        }
    }
    score
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    #[serde(flatten)]
    pub result: ResultRow,
    pub authoritative: bool,
    pub source_kind: String,
    pub score: i64,
    pub score_reasons: Vec<String>,
    pub fetch_risk: String,
}
fn canonical(url: &str) -> String {
    if let Ok(mut parsed) = Url::parse(url) {
        parsed.set_fragment(None);
        let pairs = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Vec<_>>();
        let dropped = pairs
            .iter()
            .any(|(k, _)| has(r"(?i)^(utm_|fbclid$|gclid$|mc_cid$|mc_eid$)", k));
        if dropped {
            let retained = pairs
                .into_iter()
                .filter(|(k, _)| !has(r"(?i)^(utm_|fbclid$|gclid$|mc_cid$|mc_eid$)", k))
                .collect::<Vec<_>>();
            parsed.set_query(None);
            if !retained.is_empty() {
                parsed.query_pairs_mut().extend_pairs(retained);
            }
        }
        parsed
            .to_string()
            .strip_suffix('/')
            .unwrap_or(parsed.as_str())
            .to_string()
    } else {
        trim(url)
            .strip_suffix('/')
            .unwrap_or(trim(url))
            .to_lowercase()
    }
}
fn dedupe_raw(results: Vec<ResultRow>) -> Vec<ResultRow> {
    let mut seen = BTreeSet::new();
    results
        .into_iter()
        .filter(|r| seen.insert(canonical(&r.url)))
        .collect()
}
pub fn rank(
    query: &str,
    results: &[ResultRow],
    outcomes: &BTreeMap<String, String>,
) -> Result<Vec<Ranked>, DomainError> {
    if query.len() > 4096
        || results.len() > 256
        || results
            .iter()
            .any(|r| r.url.len() > 8192 || r.title.len() > 16384 || r.snippet.len() > 65536)
    {
        return Err(DomainError::LimitExceeded("web search ranking".into()));
    }
    let intent = research::intent(query);
    let mut rows: Vec<_> = results
        .iter()
        .map(|r| {
            let outcome = outcomes.get(&r.url).map(String::as_str);
            let score = source_score(&r.url, intent, outcome);
            let source_kind = source_kind(&r.url, intent);
            Ranked {
                result: r.clone(),
                // Dated paths used to cross the generic score threshold and
                // made arbitrary news pages look like primary sources. For
                // current events, only a publisher-owned host earns this label.
                authoritative: if intent == "current_events" {
                    source_kind == "official"
                } else {
                    score >= 3
                },
                source_kind: source_kind.into(),
                score: score + content_score(r, intent, query),
                score_reasons: score_reasons(r, intent, query, outcome),
                fetch_risk: fetch_risk(&r.url, outcome).into(),
            }
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.score));
    let mut seen = BTreeSet::new();
    rows.retain(|r| seen.insert(canonical(&r.result.url)));
    Ok(rows)
}
pub fn diversify(rows: Vec<Ranked>, query: &str, intent: &str) -> Vec<Ranked> {
    if rows.len() <= 2
        || (intent == "docs_reference"
            && (group(query).is_some() || has(r"(?i)\bofficial\b", query)))
    {
        return rows;
    }
    let mut selected = vec![];
    let mut deferred = vec![];
    let mut seen = BTreeSet::new();
    for row in rows {
        let host = parsed(&row.result.url)
            .map(|(host, _)| host.strip_prefix("www.").unwrap_or(&host).to_string())
            .unwrap_or_default();
        if !host.is_empty() && seen.contains(&host) && selected.len() < 2 {
            deferred.push(row);
        } else {
            selected.push(row);
            if !host.is_empty() {
                seen.insert(host);
            }
        }
    }
    selected.extend(deferred);
    selected
}
pub fn decode_html(value: &str) -> String {
    let mut value = replace(value, r"<[^>]*>", " ");
    for (from, to) in [
        ("&nbsp;", " "),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&#39;", "'"),
        ("&quot;", "\""),
        ("&#x2F;", "/"),
        ("&#x27;", "'"),
    ] {
        value = value.replace(from, to)
    }
    trim(&replace(&compact(&value), r"\s+([.,;:!?])", "$1")).into()
}
fn usable(url: &str) -> bool {
    url.len() <= 8192
        && Url::parse(url).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
                && !host_is(u.host_str().unwrap_or_default(), "duckduckgo.com")
        })
}
fn normalize_result_url(value: &str) -> String {
    let Ok(base) = Url::parse("https://duckduckgo.com/") else {
        return value.into();
    };
    let Ok(url) = base.join(value) else {
        return value.into();
    };
    if host_is(url.host_str().unwrap_or_default(), "duckduckgo.com")
        && url.path().starts_with("/l/")
    {
        // query_pairs already percent-decodes once. A second decode corrupts
        // percent-escaped paths and literal percent signs in destination URLs.
        if let Some((_, target)) = url.query_pairs().find(|(k, _)| k == "uddg") {
            return target.into_owned();
        }
    }
    url.to_string()
}

/// Classify transport-success pages before interpreting an empty parse as no hits.
/// CAPTCHA pages are evidence of blocked access, never prompts to solve/bypass it.
pub fn duckduckgo_page_status(html: &str) -> &'static str {
    let doc = dom_query::Document::from(html);
    if !doc
        .select("form#challenge-form, .anomaly-modal, form[action*='anomaly.js']")
        .is_empty()
    {
        "blocked"
    } else if !doc.select("a.result__a, a.result-link").is_empty() {
        "results"
    } else if !doc.select(".no-results, .no-results__message").is_empty()
        || compact(&doc.select("body").text())
            .to_lowercase()
            .starts_with("no results")
        || doc.select("body").text().contains("No more results.")
        || doc
            .select("title")
            .text()
            .to_lowercase()
            .contains("no results")
    {
        "empty"
    } else {
        "parse_error"
    }
}

pub fn parse_duckduckgo(html: &str, max: usize) -> Result<Vec<ResultRow>, DomainError> {
    if html.len() > 1_000_000 || max > 256 {
        return Err(DomainError::LimitExceeded("web search HTML".into()));
    }
    if max == 0 {
        return Ok(vec![]);
    }
    let doc = dom_query::Document::from(html);
    doc.select("script, style, template").remove();
    let mut rows = vec![];
    let mut seen = BTreeSet::new();
    for anchor in doc.select("a.result__a, a.result-link").iter() {
        let Some(href) = anchor.attr("href") else {
            continue;
        };
        let url = normalize_result_url(&href);
        let title = decode_html(&anchor.text());
        if title.is_empty() || !usable(&url) || !seen.insert(canonical(&url)) {
            continue;
        }
        let mut snippet = String::new();
        // HTML results have a common .result ancestor. Lite puts the snippet
        // in the next table row. Bound sibling scans to this result only.
        for parent in anchor.ancestors(Some(6)).iter() {
            if parent.is(".result, .web-result") {
                snippet = decode_html(&parent.select(".result__snippet").text());
                break;
            }
            if parent.is("tr") {
                let mut next = parent.next_sibling();
                for _ in 0..3 {
                    if next.is_empty() || !next.select("a.result-link").is_empty() {
                        break;
                    }
                    snippet = decode_html(&next.select(".result-snippet").text());
                    if !snippet.is_empty() {
                        break;
                    }
                    next = next.next_sibling();
                }
                break;
            }
        }
        if snippet.is_empty() {
            let mut next = anchor.next_sibling();
            for _ in 0..4 {
                if next.is_empty() || next.is("a.result__a, a.result-link") {
                    break;
                }
                if next.is(".result__snippet, .result-snippet") {
                    snippet = decode_html(&next.text());
                    break;
                }
                next = next.next_sibling();
            }
        }
        rows.push(ResultRow {
            title: title.chars().take(4096).collect(),
            url,
            snippet: snippet.chars().take(4096).collect(),
        });
        if rows.len() >= max {
            break;
        }
    }
    Ok(rows)
}
pub fn parse_brave(data: &Value, max: usize) -> Result<(Vec<ResultRow>, usize), DomainError> {
    super::schema::bounded(data, 1_000_000, 32)?;
    if max > 256 {
        return Err(DomainError::LimitExceeded("web search results".into()));
    }
    let mut rows = vec![];
    if let Some(values) = data["web"]["results"].as_array() {
        for row in values {
            let (Some(title), Some(url)) = (
                row["title"].as_str().filter(|s| !s.is_empty()),
                row["url"].as_str().filter(|s| !s.is_empty()),
            ) else {
                continue;
            };
            if !usable(url) {
                continue;
            }
            let snippet = row["description"]
                .as_str()
                .or_else(|| {
                    row["extra_snippets"].as_array().and_then(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .find(|s| !trim(s).is_empty())
                    })
                })
                .unwrap_or_default();
            rows.push(ResultRow {
                title: decode_html(title),
                url: url.into(),
                snippet: decode_html(snippet),
            });
            if rows.len() >= max {
                break;
            }
        }
    }
    rows.truncate(max);
    let count = rows.len();
    Ok((dedupe_raw(rows), count))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub provider: String,
    pub url: String,
    pub status: String,
    pub result_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub results: Vec<ResultRow>,
    pub attempts: Vec<Attempt>,
    pub provider: String,
    pub provider_mode: String,
    pub provider_order: Vec<String>,
    pub cache_hit: bool,
}
pub struct SearchProjection {
    pub result: Value,
    pub hints: Vec<(String, research::UrlHint)>,
}
pub fn project(
    query: &str,
    year: u32,
    max: usize,
    response: ProviderResponse,
    profile: research::Profile,
    outcomes: &BTreeMap<String, String>,
) -> Result<SearchProjection, DomainError> {
    use serde_json::json;
    if max > 4 || response.attempts.len() > 16 {
        return Err(DomainError::LimitExceeded("web search projection".into()));
    }
    let intent = research::intent(query);
    let plan = plan(query, year)?;
    let mut fallback = if response.results.is_empty() {
        offline_fallback(&plan.effective_query, intent)
    } else {
        vec![]
    };
    fallback.truncate(max);
    let used_fallback = !fallback.is_empty();
    let blocked = response.attempts.iter().any(|a| a.status == "blocked");
    let failed = response.results.is_empty()
        && (blocked || !response.attempts.iter().any(|a| a.status == "ok"));
    let raw = if used_fallback {
        &fallback
    } else {
        &response.results
    };
    let candidate_count = raw.len();
    let ranked = selection::select(query, raw, outcomes, max)?;
    let mut details = json!({"query":query,"effectiveQuery":plan.effective_query,"intent":intent,"searchStrategy":plan.strategy,"followUpQueries":plan.follow_up_queries,"freshness":plan.freshness,"preferCorroboration":strongest(query),"resultCount":ranked.len(),"compressionMode":profile.mode,"cacheHit":response.cache_hit,"searchProvider":if used_fallback{"official-fallback"}else{&response.provider},"searchProviderMode":response.provider_mode,"searchProviderOrder":response.provider_order,"searchAttempts":response.attempts,"fallbackUsed":used_fallback,"sourceUrls":ranked.iter().map(|r|&r.result.url).collect::<Vec<_>>(),"preferredSourceUrls":ranked.iter().filter(|r|r.authoritative||r.score>=4).map(|r|&r.result.url).collect::<Vec<_>>(),"results":ranked});
    details["candidateCount"] = json!(candidate_count);
    details["selectionPolicy"] = json!("relevance-source-version-v1");
    if let Some(domains) = plan.preferred_domains {
        details["preferredDomains"] = json!(domains);
    }
    if let Some(domains) = plan.avoided_domains {
        details["avoidedDomains"] = json!(domains);
    }
    let mut transcript_profile = profile.clone();
    transcript_profile.search_transcript_results = match profile.mode.as_str() {
        "ultra" => 2,
        "tight" => 3,
        _ => 4,
    };
    transcript_profile.search_snippet_chars = match profile.mode.as_str() {
        "ultra" => 140,
        "tight" => 220,
        _ => 320,
    };
    let mut text = research::search_transcript(
        &plan.effective_query,
        &ranked
            .iter()
            .map(|r| research::SearchResult {
                title: r.result.title.clone(),
                url: r.result.url.clone(),
                snippet: Some(r.result.snippet.clone()),
                authoritative: r.authoritative,
            })
            .collect::<Vec<_>>(),
        &transcript_profile,
        &research::SearchOptions {
            intent: Some(intent.into()),
            prefer_corroboration: strongest(query),
            original_query: Some(plan.original_query),
            search_strategy: Some(plan.strategy),
            follow_up_queries: plan.follow_up_queries,
        },
    );
    if failed || used_fallback {
        let reason = if blocked {
            "DuckDuckGo blocked or rate-limited automated access (it may require human verification). Search results are unavailable. Do not repeat or rephrase searches while it is blocked."
        } else if failed {
            "Search providers failed; this is not evidence that no matching pages exist."
        } else {
            "No matching search results were returned."
        };
        text = format!(
            "{}{}",
            reason,
            if used_fallback {
                format!(
                    "\nKnown documentation suggestions (not search results; not fetched or verified):\n{}",
                    ranked
                        .iter()
                        .map(|r| format!("- {}: {}", r.result.title, r.result.url))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            } else {
                "\nUse webFetch for known URLs, or try another available search provider.".into()
            }
        );
        details["resultCount"] = json!(response.results.len());
        details["sourceUrls"] = json!([]);
        details["preferredSourceUrls"] = json!([]);
        details["suggestions"] = json!(ranked);
        details["results"] = json!([]);
        details["status"] = json!(if failed {
            if blocked { "blocked" } else { "error" }
        } else {
            "empty"
        });
        if failed {
            details["error"] = json!(reason);
        }
    }
    let hints = ranked
        .iter()
        .map(|r| {
            (
                r.result.url.clone(),
                research::UrlHint {
                    intent: intent.into(),
                    rich_fetch: research::rich_fetch(query, intent),
                    authoritative: Some(r.authoritative),
                    focus_terms: selection::focus_terms(query),
                    coverage_kind: coverage_kind(query, intent).map(str::to_string),
                },
            )
        })
        .collect();
    Ok(SearchProjection {
        result: if failed {
            json!({"isError":true,"details":details,"content":[{"type":"text","text":text}]})
        } else {
            json!({"details":details,"content":[{"type":"text","text":text}]})
        },
        hints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_events_keep_the_users_words_and_do_not_invent_a_year() {
        let plan = plan("latest AI news on Qwen models", 2026).unwrap();

        assert_eq!(plan.effective_query, "latest AI news on Qwen models");
        assert_eq!(plan.freshness, "current");
        assert!(
            plan.follow_up_queries
                .iter()
                .any(|query| query.ends_with("official source"))
        );
    }

    #[test]
    fn current_events_only_mark_publisher_pages_as_authoritative() {
        let results = vec![
            ResultRow {
                title: "Qwen 3.6 roadmap 2026".into(),
                url: "https://example-news.invalid/research/qwen-3-6-roadmap-2026".into(),
                snippet: "Unverified commentary about a model release.".into(),
            },
            ResultRow {
                title: "Qwen release announcement".into(),
                url: "https://qwen.ai/blog?id=qwen-release".into(),
                snippet: "A publisher-owned announcement.".into(),
            },
        ];

        let ranked = rank("latest Qwen model news", &results, &BTreeMap::new()).unwrap();
        let commentary = ranked
            .iter()
            .find(|row| row.result.url.contains("example-news"))
            .unwrap();
        let publisher = ranked
            .iter()
            .find(|row| row.result.url.contains("qwen.ai"))
            .unwrap();

        assert!(!commentary.authoritative);
        assert!(publisher.authoritative);
        assert!(publisher.score > commentary.score);

        let selected =
            selection::select("latest Qwen model news", &results, &BTreeMap::new(), 2).unwrap();
        assert!(selected[0].result.url.contains("qwen.ai"));
    }

    #[test]
    fn current_events_transcript_requires_a_fetch_before_an_answer() {
        let text = research::search_transcript(
            "latest Qwen model news",
            &[research::SearchResult {
                title: "Qwen release announcement".into(),
                url: "https://qwen.ai/blog?id=qwen-release".into(),
                snippet: None,
                authoritative: true,
            }],
            &research::profile(32_768., 0.),
            &research::SearchOptions {
                intent: Some("current_events".into()),
                ..Default::default()
            },
        );

        assert!(text.contains("Do not answer a current-events question from snippets"));
        assert!(text.contains("webFetch https://qwen.ai/blog?id=qwen-release"));
    }
}
