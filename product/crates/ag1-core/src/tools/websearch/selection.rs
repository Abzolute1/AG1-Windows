//! Bounded source selection after retrieval. Scores are heuristics, not verification.
use super::*;

pub(super) fn focus_terms(query: &str) -> Vec<String> {
    let query = replace(query, r"(?i)-?\b(?:site|filetype|inurl|intitle):\S+", " ");
    unique(
        query
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
            .map(str::to_lowercase)
            .filter(|t| {
                t.len() >= 3
                    && !matches!(
                        t.as_str(),
                        "the"
                            | "and"
                            | "for"
                            | "with"
                            | "from"
                            | "what"
                            | "how"
                            | "does"
                            | "are"
                            | "can"
                            | "that"
                            | "this"
                            | "official"
                            | "documentation"
                            | "docs"
                            | "reference"
                            | "api"
                            | "latest"
                            | "current"
                            | "using"
                            | "use"
                            | "about"
                    )
            }),
    )
    .into_iter()
    .take(12)
    .collect()
}
fn version(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    url.path_segments()?
        .find(|s| has(r"(?i)^v?\d+(?:\.\d+){1,3}(?:\.x)?$", s))
        .map(|s| s.trim_start_matches('v').to_string())
}
fn coverage(row: &ResultRow, terms: &[String]) -> usize {
    let text = format!("{} {} {}", row.title, row.url, row.snippet).to_lowercase();
    terms.iter().filter(|t| text.contains(t.as_str())).count()
}
pub(super) fn select(
    query: &str,
    raw: &[ResultRow],
    outcomes: &BTreeMap<String, String>,
    max: usize,
) -> Result<Vec<Ranked>, DomainError> {
    let terms = focus_terms(query);
    let requested_versions: Vec<_> = rx(r"\b\d+(?:\.\d+){1,3}\b")
        .find_iter(query)
        .map(|m| m.as_str())
        .collect();
    let intent = research::intent(query);
    let mut ranked = rank(query, raw, outcomes)?;
    let primary_domains: Vec<&str> = [
        ("sqlite", "sqlite.org"),
        ("postgresql", "postgresql.org"),
        ("postgres", "postgresql.org"),
        ("urllib3", "urllib3.readthedocs.io"),
        ("python", "docs.python.org"),
        ("rust", "rust-lang.org"),
        ("node", "nodejs.org"),
        ("javascript", "developer.mozilla.org"),
        ("react", "react.dev"),
        ("typescript", "typescriptlang.org"),
        // For Qwen release questions, prefer the model publisher instead of an
        // article that merely has the right words in its URL or title.
        ("qwen", "qwen.ai"),
        ("git", "git-scm.com"),
        ("go", "go.dev"),
        ("php", "php.net"),
    ]
    .into_iter()
    .filter(|(topic, _)| {
        query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token == *topic)
    })
    .map(|(_, host)| host)
    .collect();
    for row in &mut ranked {
        let matched = coverage(&row.result, &terms);
        // Existing domain/path bonuses must not bury topical results beneath a
        // famous but unrelated documentation page in the larger candidate pool.
        let position = raw
            .iter()
            .position(|r| r.url == row.result.url)
            .unwrap_or(raw.len());
        let title = row.result.title.to_lowercase();
        let title_matches = terms.iter().filter(|t| title.contains(t.as_str())).count();
        row.score = row.score.clamp(-20, 18)
            + (matched.min(12) as i64 * 4)
            + title_matches.min(12) as i64 * 3
            + 10i64.saturating_sub(position as i64 * 2).max(0);
        row.score_reasons
            .push(format!("provider_position:{}", position + 1));
        if let Some((host, path)) = parsed(&row.result.url) {
            let primary = primary_domains.iter().any(|domain| host_is(&host, domain))
                || group(query).is_some_and(|g| g.domains.iter().any(|d| host_is(&host, d)));
            if primary && matched > 0 {
                row.score += 18;
                row.authoritative = true;
                row.source_kind = "official".into();
                row.score_reasons.push(
                    if intent == "current_events" {
                        "publisher_primary_source"
                    } else {
                        "project_primary_documentation"
                    }
                    .into(),
                );
            }
            if matches!(intent, "docs_reference" | "code_lookup") {
                if host_is(&host, "typeerror.org|w3cub.com|gitbooks.io|deepwiki.com") {
                    row.score -= 10;
                    row.score_reasons.push("documentation_mirror".into());
                }
                if !has(r"(?i)\b(beta|nightly|dev|preview)\b", query)
                    && (host.starts_with("beta.")
                        || host.starts_with("dev-doc.")
                        || has(r"/(beta|nightly|preview)/", &path))
                {
                    row.score -= 12;
                    row.score_reasons.push("prerelease_not_requested".into());
                }
            }
        }
        if matched == 0 && !terms.is_empty() {
            row.score -= 40;
            row.score_reasons.push("no_topical_terms_in_result".into());
        } else if matched > 0 {
            row.score_reasons
                .push(format!("topical_terms:{matched}/{}", terms.len()));
        }
        if matches!(intent, "docs_reference" | "code_lookup")
            && let Some(v) = version(&row.result.url)
        {
            if requested_versions
                .iter()
                .any(|wanted| v == *wanted || v.starts_with(&format!("{wanted}.")))
            {
                row.score += 18;
                row.score_reasons.push("requested_version_match".into());
            } else if !requested_versions.is_empty() {
                row.score -= 18;
                row.score_reasons.push("different_explicit_version".into());
            } else if raw.iter().any(|other| {
                version(&other.url).is_none()
                    && parsed(&other.url).map(|v| v.0) == parsed(&row.result.url).map(|v| v.0)
                    && coverage(other, &terms) >= matched
                    && matched > 0
            }) {
                row.score -= 12;
                row.score_reasons
                    .push("prefer_unversioned_peer_for_unspecified_version".into());
            }
        }
    }
    ranked.sort_by_key(|r| std::cmp::Reverse(r.score));
    // Documentation benefits from related pages on the same official host.
    // General/news searches still use the existing light domain diversification.
    if !matches!(intent, "docs_reference" | "code_lookup") {
        ranked = diversify(ranked, query, intent);
    }
    ranked.truncate(max);
    Ok(ranked)
}
