use ag1_core::tools::{research as r, websearch as w};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn row(title: &str, url: &str, snippet: &str) -> w::ResultRow {
    w::ResultRow {
        title: title.into(),
        url: url.into(),
        snippet: snippet.into(),
    }
}
fn search(query: &str, rows: Vec<w::ResultRow>, max: usize) -> Value {
    w::project(
        query,
        2026,
        max,
        w::ProviderResponse {
            attempts: vec![w::Attempt {
                provider: "duckduckgo-web".into(),
                url: "https://noai.duckduckgo.com/".into(),
                status: "ok".into(),
                result_count: rows.len(),
                error: None,
            }],
            results: rows,
            provider: "duckduckgo-web".into(),
            provider_mode: "auto".into(),
            provider_order: vec!["duckduckgo".into()],
            cache_hit: false,
        },
        r::profile(90000., 0.),
        &BTreeMap::new(),
    )
    .unwrap()
    .result
}
fn fetch(body: &str, content_type: &str, focus: &[&str], window: f64) -> Value {
    r::fetch_projection(
        r::FetchedDocument {
            url: "https://example.org/docs",
            final_url: "https://example.org/docs",
            status_code: 200,
            content_type: Some(content_type),
            cache_hit: false,
            bytes: body.as_bytes(),
        },
        r::profile(window, 0.),
        Some(&r::UrlHint {
            intent: "docs_reference".into(),
            focus_terms: focus.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }),
    )
    .unwrap()
    .result
}
fn text(v: &Value) -> &str {
    v["content"][0]["text"].as_str().unwrap()
}
#[test]
fn strong_relevant_source_beyond_original_cutoff_is_selected() {
    let mut rows: Vec<_> = (0..8)
        .map(|i| {
            row(
                "Python tutorials",
                &format!("https://example.org/tutorial/{i}"),
                "General programming examples",
            )
        })
        .collect();
    rows.push(row(
        "asyncio TaskGroup cancellation",
        "https://docs.python.org/3/library/asyncio-task.html",
        "TaskGroup uses cancellation internally and raises ExceptionGroup.",
    ));
    let v = search("Python TaskGroup cancellation documentation", rows, 3);
    assert_eq!(v["details"]["candidateCount"], 9);
    assert_eq!(v["details"]["resultCount"], 3);
    assert_eq!(
        v["details"]["results"][0]["url"],
        "https://docs.python.org/3/library/asyncio-task.html"
    );
    assert!(text(&v).contains("ExceptionGroup"));
}
#[test]
fn unrelated_official_docs_do_not_outrank_a_relevant_source() {
    let v = search(
        "urllib3 Retry backoff documentation",
        vec![
            row(
                "Python language reference",
                "https://docs.python.org/3/reference/",
                "Complete language reference",
            ),
            row(
                "Retry backoff",
                "https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html",
                "urllib3 Retry backoff factor and status codes",
            ),
        ],
        2,
    );
    assert!(
        v["details"]["results"][0]["url"]
            .as_str()
            .unwrap()
            .contains("urllib3")
    );
}
#[test]
fn version_preference_preserves_explicit_old_version_requests() {
    let rows = vec![
        row(
            "Rust Iterator docs",
            "https://doc.rust-lang.org/1.26.1/std/iter/trait.Iterator.html",
            "Rust Iterator collect examples",
        ),
        row(
            "Rust Iterator docs",
            "https://doc.rust-lang.org/stable/std/iter/trait.Iterator.html",
            "Rust Iterator collect examples",
        ),
    ];
    let current = search("Rust Iterator collect documentation", rows.clone(), 1);
    assert!(
        current["details"]["results"][0]["url"]
            .as_str()
            .unwrap()
            .contains("/stable/")
    );
    let old = search("Rust 1.26.1 Iterator collect documentation", rows, 1);
    assert!(
        old["details"]["results"][0]["url"]
            .as_str()
            .unwrap()
            .contains("/1.26.1/")
    );
}
#[test]
fn duplicates_drop_tracking_but_preserve_case_sensitive_paths_and_query_values() {
    let v = search(
        "case sensitive API documentation",
        vec![
            row(
                "API",
                "https://example.org/API?key=Abc&utm_source=test",
                "case sensitive",
            ),
            row(
                "Duplicate",
                "https://example.org/API?key=Abc",
                "case sensitive",
            ),
            row(
                "Other path",
                "https://example.org/api?key=Abc",
                "case sensitive",
            ),
            row(
                "Other key",
                "https://example.org/API?key=abc",
                "case sensitive",
            ),
        ],
        4,
    );
    assert_eq!(v["details"]["resultCount"], 3);
}
#[test]
fn dom_extraction_preserves_code_tables_entities_and_removes_navigation() {
    let v = fetch(
        r#"<html><head><meta content="A &amp; B" property="og:title"></head><body><nav>Navigation junk</nav><main><h1>Retry examples</h1><p>Use <code>copy()</code> to preserve an object.</p><pre><code class="language-python">if retries &lt; 3:
    client.copy()

    return "&#x1F980;"</code></pre><table><tr><th>Option</th><th>Meaning</th></tr><tr><td>backoff_factor</td><td>Seconds to wait</td></tr></table><script>hostile()</script><div hidden>Hidden junk</div></main></body></html>"#,
        "text/html",
        &["retry"],
        90000.,
    );
    let t = text(&v);
    assert_eq!(v["details"]["title"], "A & B");
    assert!(
        t.contains("if retries < 3:\n    client.copy()\n\n    return \"🦀\""),
        "{t}"
    );
    assert!(t.contains("```python"));
    assert!(t.contains(r"| backoff\_factor | Seconds to wait |"), "{t}");
    assert!(t.contains("`copy()`"));
    assert!(
        !t.contains("Navigation junk") && !t.contains("hostile()") && !t.contains("Hidden junk")
    );
    assert_eq!(v["details"]["truncated"], false);
}
#[test]
fn plain_text_keeps_angle_brackets_and_code_whitespace() {
    let source = "Vec<T>\nif x < 3:\n    print('<main>literal</main>')\n";
    let v = fetch(source, "text/plain", &[], 90000.);
    assert!(text(&v).contains(source));
    assert_eq!(v["details"]["extractionMethod"], "plain-text-v1");
}
#[test]
fn focused_excerpt_reaches_late_passages_and_keeps_code_blocks_whole() {
    let mut source=(0..200).map(|i|format!("## Background {i}\n\nUnrelated discussion of installation and everyday usage, paragraph number {i}.\n\n")).collect::<String>();
    source.push_str("## Cancellation semantics\n\nTaskGroup propagates cancellation after children finish.\n\n```python\nasync with TaskGroup() as group:\n    group.create_task(work())\n\n    await checkpoint()\n```\n\nExceptionGroup collects the remaining failures.\n");
    for (window, budget) in [(90000., 12000), (50000., 7000), (16000., 3500)] {
        let v = fetch(
            &source,
            "text/markdown",
            &["TaskGroup", "cancellation"],
            window,
        );
        let t = text(&v);
        assert!(t.contains("TaskGroup propagates cancellation"));
        assert!(t.contains("group.create_task(work())\n\n    await checkpoint()"));
        assert!(t.contains("Source passage omitted"));
        assert_eq!(v["details"]["truncated"], true);
        assert!(
            v["details"]["excerptChars"].as_u64().unwrap() <= budget,
            "{v}"
        );
    }
}
#[test]
fn binary_and_placeholder_are_errors_and_source_material_stays_untrusted() {
    let binary = fetch("%PDF-1.7", "application/pdf", &[], 90000.);
    assert_eq!(binary["isError"], true);
    let blocked = fetch(
        "<html><body>Please enable JavaScript to continue</body></html>",
        "text/html",
        &[],
        90000.,
    );
    assert_eq!(blocked["isError"], true, "{blocked}");
    assert!(text(&blocked).contains("untrusted page content"));
    assert_eq!(blocked["details"]["authoritative"], false);
}
#[test]
fn html_nested_main_keeps_all_articles_and_decodes_entities_once() {
    let v = fetch(
        "<main><article><h2>First article</h2><p>&amp;lt;tag&amp;gt; is escaped source.</p></article><article><h2>Second article</h2><p>Another useful source paragraph.</p></article></main>",
        "text/html",
        &[],
        90000.,
    );
    assert!(text(&v).contains("First article") && text(&v).contains("Second article"));
    assert!(text(&v).contains("&lt;tag&gt;"));
    assert_ne!(v["isError"], json!(true));
}

#[test]
fn primary_documentation_beats_mirrors_with_extra_query_keywords() {
    let v = search(
        "SQLite WAL checkpoint SQLITE_BUSY official documentation",
        vec![
            row(
                "Checkpoint a database - SQLite",
                "https://sqlite.org/c3ref/wal_checkpoint_v2.html",
                "sqlite3_wal_checkpoint_v2 runs a checkpoint operation",
            ),
            row(
                "SQLite Documentation",
                "https://typeerror.org/docs/sqlite/checkpoint",
                "SQLite WAL checkpoint SQLITE_BUSY official documentation",
            ),
            row(
                "SQLite Database Locked",
                "https://example.org/sqlite",
                "SQLite WAL checkpoint SQLITE_BUSY explained",
            ),
        ],
        3,
    );
    assert_eq!(
        v["details"]["results"][0]["url"],
        "https://sqlite.org/c3ref/wal_checkpoint_v2.html"
    );
    assert_eq!(v["details"]["results"][0]["sourceKind"], "official");
}
#[test]
fn links_resolve_against_page_but_cannot_embed_credentials_or_scripts() {
    let v = fetch(
        "<main><p>Read the <a href='../reference'>reference</a> and <a href='javascript:alert(1)'>bad target</a> or <a href='https://u:p@example.org'>credentials</a>.</p></main>",
        "text/html",
        &[],
        90000.,
    );
    assert!(text(&v).contains("https://example.org/reference"));
    assert!(!text(&v).contains("javascript:") && !text(&v).contains("https://u:p"));
}

#[test]
#[ignore = "Offline replay of retained live bodies supplied through AG1_SEARCH_QUALITY_EVIDENCE"]
fn retained_live_pages_keep_the_requested_api_details() {
    use base64::Engine;
    let directory = std::path::PathBuf::from(std::env::var("AG1_SEARCH_QUALITY_EVIDENCE").unwrap());
    let responses: Vec<Value> = serde_json::from_slice(
        &std::fs::read(directory.join("live-final-responses.json")).unwrap(),
    )
    .unwrap();
    let results: Vec<Value> =
        serde_json::from_slice(&std::fs::read(directory.join("live-final-results.json")).unwrap())
            .unwrap();
    let expected = [
        vec![
            "allowed_methods",
            "backoff_factor",
            "DEFAULT_ALLOWED_METHODS",
        ],
        vec!["SQLITE_BUSY", "SQLITE_CHECKPOINT_FULL"],
        vec!["collect", "FromIterator"],
        vec!["AbortSignal.timeout", "delay"],
    ];
    let mut replayed = vec![];
    for (index, result) in results.iter().take(4).enumerate() {
        let url = result["fetch"]["details"]["url"].as_str().unwrap();
        let response = responses.iter().find(|r| r["requestedUrl"] == url).unwrap();
        let body = base64::engine::general_purpose::STANDARD
            .decode(response["bodyBase64"].as_str().unwrap())
            .unwrap();
        let query = result["search"]["details"]["query"].as_str().unwrap();
        let raw = vec![
            serde_json::from_value::<w::ResultRow>(
                result["search"]["details"]["results"][0].clone(),
            )
            .unwrap(),
        ];
        let hints = w::project(
            query,
            2026,
            1,
            w::ProviderResponse {
                results: raw,
                attempts: vec![],
                provider: "replay".into(),
                provider_mode: "duckduckgo".into(),
                provider_order: vec![],
                cache_hit: false,
            },
            r::profile(90000., 0.),
            &BTreeMap::new(),
        )
        .unwrap()
        .hints;
        let projected = r::fetch_projection(
            r::FetchedDocument {
                url,
                final_url: response["finalUrl"].as_str().unwrap(),
                status_code: 200,
                content_type: response["contentType"].as_str(),
                cache_hit: false,
                bytes: &body,
            },
            r::profile(90000., 0.),
            Some(&hints[0].1),
        )
        .unwrap()
        .result;
        let content = text(&projected).replace('\\', "");
        for term in &expected[index] {
            assert!(content.contains(term), "{url}: missing {term}");
        }
        assert!(projected["details"]["excerptChars"].as_u64().unwrap() <= 12000);
        replayed.push(projected);
    }
    println!(
        "REPLAYED_FETCHES={}",
        serde_json::to_string(&replayed).unwrap()
    );
}
