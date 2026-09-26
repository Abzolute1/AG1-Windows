use ag1_core::tools::{
    research, web_presentation,
    websearch::{self as w, Attempt, ProviderResponse},
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn captured_human_challenge_is_not_an_empty_search() {
    let body = include_str!("fixtures/duckduckgo-challenge.html");
    assert_eq!(w::duckduckgo_page_status(body), "blocked");
    assert!(w::parse_duckduckgo(body, 4).unwrap().is_empty());
    assert_eq!(
        w::duckduckgo_page_status("<html><body>Service unavailable</body></html>"),
        "parse_error"
    );
    assert_eq!(
        w::duckduckgo_page_status("<div class='no-results'>No results found for a query</div>"),
        "empty"
    );
}

#[test]
fn html_attributes_entities_snippets_and_redirects_survive_parsing() {
    let html = r#"<div class="result"><h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org%2Fa%252Fb%3Fx%3D1%26y%3D2&amp;rut=ignored" data-other="x" class="other result__a">A &amp; B &#x1f980;</a></h2><div class="result__snippet">Use <b>Retry</b> &lt;limit&gt;.</div></div>"#;
    assert_eq!(w::duckduckgo_page_status(html), "results");
    let rows = w::parse_duckduckgo(html, 4).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "A & B 🦀");
    assert_eq!(rows[0].url, "https://example.org/a%2Fb?x=1&y=2");
    assert_eq!(rows[0].snippet, "Use Retry <limit>.");
}

#[test]
fn real_lite_shape_keeps_snippets_with_their_results() {
    let html = r#"<table>
    <tr><td>1.</td><td><a rel="nofollow" href="https://example.org/first" class='result-link'>First</a></td></tr>
    <tr><td></td><td class='result-snippet'>First <b>snippet</b>.</td></tr>
    <tr><td></td><td class='link-text'>example.org</td></tr>
    <tr><td>2.</td><td><a href="https://example.net/second" class='result-link'>Second</a></td></tr>
    <tr><td></td><td class='result-snippet'>Second snippet.</td></tr></table>"#;
    let rows = w::parse_duckduckgo(html, 4).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].snippet, "First snippet.");
    assert_eq!(rows[1].snippet, "Second snippet.");
}

#[test]
fn invalid_links_and_duplicates_do_not_consume_result_budget() {
    let html = r#"<a class=result__a href="javascript:alert(1)">Bad</a><a class=result__a href="https://u:p@example.org">Credentials</a><a class=result__a href="https://example.org/a">First</a><a class=result__a href="https://example.org/a">Duplicate</a><a class=result__a href="https://example.org/b">Second</a>"#;
    let rows = w::parse_duckduckgo(html, 2).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].title, "Second");
    assert!(w::parse_duckduckgo(html, 0).unwrap().is_empty());
}

fn response(status: &str) -> ProviderResponse {
    ProviderResponse {
        results: vec![],
        attempts: vec![Attempt {
            provider: "duckduckgo-html".into(),
            url: "https://html.duckduckgo.com/html/".into(),
            status: status.into(),
            result_count: 0,
            error: None,
        }],
        provider: "duckduckgo-html".into(),
        provider_mode: "auto".into(),
        provider_order: vec!["duckduckgo".into()],
        cache_hit: false,
    }
}
#[test]
fn blocked_search_is_an_error_and_static_suggestions_are_never_hits() {
    let r = w::project(
        "Python documentation",
        2026,
        4,
        response("blocked"),
        research::profile(50000., 4.),
        &BTreeMap::new(),
    )
    .unwrap()
    .result;
    assert_eq!(r["isError"], true);
    assert_eq!(r["details"]["status"], "blocked");
    assert_eq!(r["details"]["resultCount"], 0);
    assert_eq!(r["details"]["sourceUrls"], json!([]));
    assert_eq!(r["details"]["results"], json!([]));
    assert!(!r["details"]["suggestions"].as_array().unwrap().is_empty());
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not search results"));
    assert!(!text.contains("No results found."));
    let p = web_presentation::render(
        "webSearch",
        &json!({"query":"Python documentation"}),
        &r,
        false,
    )
    .unwrap();
    assert_eq!(p.summary, "Search blocked");
}
#[test]
fn genuine_empty_search_is_distinct_from_failed_search() {
    let r = w::project(
        "a garden",
        2026,
        4,
        response("ok"),
        research::profile(50000., 4.),
        &BTreeMap::new(),
    )
    .unwrap()
    .result;
    assert_ne!(r["isError"], true);
    assert_eq!(r["details"]["resultCount"], 0);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No results found.")
    );
    let r = w::project(
        "a garden",
        2026,
        4,
        response("parse_error"),
        research::profile(50000., 4.),
        &BTreeMap::new(),
    )
    .unwrap()
    .result;
    assert_eq!(r["isError"], true);
}

#[test]
fn explicit_search_syntax_is_never_rewritten() {
    for query in [
        "site:docs.python.org urllib3 retry policy timeout",
        "\"TypeError: exact text\" -site:example.com",
        "site:github.com current release 2026",
        "filetype:pdf research -advertisement",
    ] {
        assert_eq!(w::plan(query, 2026).unwrap().effective_query, query);
    }
}

#[test]
fn empty_primary_does_not_hide_a_blocked_fallback() {
    let mut provider = response("ok");
    provider.attempts.extend(response("blocked").attempts);
    let result = w::project(
        "a garden",
        2026,
        4,
        provider,
        research::profile(50000., 4.),
        &BTreeMap::new(),
    )
    .unwrap()
    .result;
    assert_eq!(result["isError"], true);
    assert_eq!(result["details"]["status"], "blocked");
}
