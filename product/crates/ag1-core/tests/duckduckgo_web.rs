use ag1_core::tools::websearch::duckduckgo_web::*;
const SCRIPT: &str = include_str!("fixtures/duckduckgo-web/results.js");
const CHECK: &str = include_str!("fixtures/duckduckgo-web/arithmetic.js");
#[test]
fn retained_live_results_decode_without_running_javascript() {
    let rows = results(SCRIPT, 4).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0].url,
        "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html"
    );
    assert!(rows[0].snippet.contains("memory safety"));
    assert!(!rows[0].snippet.contains("<b>"));
}
#[test]
fn live_arithmetic_and_changed_seed_match_browser_calculation() {
    assert!(
        arithmetic_continuation(CHECK)
            .unwrap()
            .ends_with("&jsa=10715")
    );
    // Seed is multiplied by 3 and 5; this proves calculation rather than replay.
    assert!(
        arithmetic_continuation(&CHECK.replace("let jsa = 559", "let jsa = 560"))
            .unwrap()
            .ends_with("&jsa=10730")
    );
}
#[test]
fn unknown_code_fragments_operations_and_overflow_are_rejected() {
    for changed in [
        CHECK.replace("return num * 3;", "return eval(num);"),
        CHECK.replace("<br><div></br><br></div", "<img src=x onerror=alert(1)>"),
        CHECK.replace("jsa = dgZHlvHI(jsa);", "jsa = unknown(jsa);"),
        CHECK
            .replace("return num * 3;", "return num * 999;")
            .replace("jsa = dgZHlvHI(jsa);", &"jsa = FPBtGrSz(jsa);".repeat(60)),
        format!("{CHECK}fetch('https://evil.test')"),
    ] {
        assert!(arithmetic_continuation(&changed).is_err());
    }
}
#[test]
fn preload_requires_one_named_script_and_preserves_escaped_query() {
    assert_eq!(preload("<script id=deep_preload_script src='https://links.duckduckgo.com/d.js?q=a%2Bb&amp;dp=x'></script>").unwrap(), "https://links.duckduckgo.com/d.js?q=a%2Bb&dp=x");
    assert!(preload("<script src='https://evil.test/'></script>").is_err());
    assert!(preload("<script id=deep_preload_script src=a></script><script id=deep_preload_script src=b></script>").is_err());
}
#[test]
fn result_envelope_limits_deduplicates_and_rejects_unusable_payloads() {
    let body = r#"DDG.pageLayout.load('d',[{"t":"A &amp; B","u":"https://example.org/a%2Fb","a":"Use <b>safe</b> text"},{"t":"duplicate","u":"https://example.org/a%2Fb"},{"t":"bad","u":"javascript:alert(1)"},{"t":"second","u":"https://example.net/"}]);"#;
    let rows = results(body, 3).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "A & B");
    assert_eq!(rows[0].url, "https://example.org/a%2Fb");
    assert!(
        results("DDG.pageLayout.load('d',[]);", 4)
            .unwrap()
            .is_empty()
    );
    assert!(
        results(
            "DDG.pageLayout.load('d',[{\"t\":\"bad\",\"u\":\"file:///tmp/x\"}]);",
            4
        )
        .is_err()
    );
    assert!(results("DDG.pageLayout.load('d',[oops]);", 4).is_err());
    assert!(results(SCRIPT, 257).is_err());
    assert!(results(SCRIPT, 0).unwrap().is_empty());
}

#[test]
fn explicit_no_result_sentinel_is_empty_but_unknown_rows_are_not() {
    assert!(
        results(
            r#"DDG.pageLayout.load('d',[{"t":"DEEP_SIMPLE_NO_RESULTS","u":"","a":""}]);"#,
            4
        )
        .unwrap()
        .is_empty()
    );
    assert!(results(r#"DDG.pageLayout.load('d',[{"unrecognized":true}]);"#, 4).is_err());
}
