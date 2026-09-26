//! Intentional corrections to legacy file-oracle behavior: unsuccessful edits
//! are errors, aborted calls report no applied changes, and ambiguous apply
//! replacements cannot silently select the first location.
use ag1_core::tools::{file, preview};
use serde_json::{Value, json};

fn apply(original: Option<&[u8]>, edits: Value) -> file::Transformation {
    file::transform(
        "editApply",
        &json!({"path":"file.txt","edits":edits}),
        original,
    )
    .unwrap()
}

#[test]
fn later_missing_or_ambiguous_replacements_abort_the_entire_call_truthfully() {
    for edits in [
        json!([{"oldText":"one","newText":"ONE"},{"oldText":"missing","newText":"x"}]),
        json!([{"oldText":"one","newText":"two"},{"oldText":"two","newText":"x"}]),
        json!([{"oldText":"two","newText":"TWO"}]),
    ] {
        let original = b"one two two";
        let result = apply(Some(original), edits);
        assert!(
            result.bytes.is_none(),
            "No partially transformed bytes may be published"
        );
        assert_eq!(result.result["isError"], true);
        assert_eq!(result.result["details"]["applied"], 0);
        assert_eq!(result.result["details"]["verified"], false);
        assert!(
            result.result["details"]["error"]
                .as_str()
                .unwrap()
                .contains("No changes from this call were written")
        );
    }
}

#[test]
fn missing_file_and_failed_previews_have_explicit_error_flags() {
    let missing = apply(None, json!([{"oldText":"one","newText":"ONE"}]));
    assert!(missing.bytes.is_none());
    assert_eq!(missing.result["isError"], true);
    assert_eq!(missing.result["details"]["applied"], 0);
    for (source, old) in [
        (None, "one"),
        (Some("one"), "missing"),
        (Some("one one"), "one"),
    ] {
        let result =
            file::structured(&preview::preview("file.txt", source, old, "ONE").unwrap()).unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["details"]["error"].is_string());
    }
    let success =
        file::structured(&preview::preview("file.txt", Some("one"), "one", "ONE").unwrap())
            .unwrap();
    assert_ne!(success["isError"], true);
}

#[test]
fn overlapping_occurrences_are_also_ambiguous() {
    for (source, old) in [("aaa", "aa"), ("🦙🦙🦙", "🦙🦙")] {
        let result = apply(
            Some(source.as_bytes()),
            json!([{"oldText":old,"newText":"x"}]),
        );
        assert!(result.bytes.is_none());
        assert_eq!(result.result["isError"], true);
        assert_eq!(result.result["details"]["applied"], 0);
        let edited = file::transform(
            "edit",
            &json!({"path":"file.txt","edits":[{"oldText":old,"newText":"x"}]}),
            Some(source.as_bytes()),
        );
        assert!(
            matches!(edited, Err(ag1_core::DomainError::InvalidInput(reason)) if reason.contains("Found 2 occurrences"))
        );
        let previewed = preview::preview("file.txt", Some(source), old, "x").unwrap();
        assert_eq!(
            previewed.error.as_deref(),
            Some("oldText found 2 times, use more context")
        );
        assert!(previewed.hunks.is_empty());
    }
}

#[test]
fn existing_counts_and_fuzzy_matching_keep_their_semantics() {
    let duplicate = preview::preview("file.txt", Some("one one one"), "one", "x").unwrap();
    assert_eq!(
        duplicate.error.as_deref(),
        Some("oldText found 3 times, use more context")
    );
    let args = json!({"path":"file.txt","allowFuzzyMatch":true,"edits":[{"oldText":"\"value\"","newText":"\"next\""}]});
    let result = file::transform("edit", &args, Some("let x = “value”;\n".as_bytes())).unwrap();
    assert_eq!(
        result.bytes.as_deref(),
        Some(b"let x = \"next\";\n".as_slice())
    );
    assert_eq!(result.result["details"]["fuzzyMatchUsed"], true);
    let overlap = file::transform(
        "edit",
        &json!({"path":"file.txt","allowFuzzyMatch":true,"edits":[{"oldText":"\"\"","newText":"x"}]}),
        Some("“““".as_bytes()),
    );
    assert!(
        matches!(overlap, Err(ag1_core::DomainError::InvalidInput(reason)) if reason.contains("Found 2 occurrences"))
    );
}

#[test]
fn unique_sequential_replacements_and_explicit_insertion_keep_working() {
    let result = apply(
        Some(b"one two"),
        json!([
            {"oldText":"one","newText":"ONE"},
            {"oldText":"ONE two","newText":"done"}
        ]),
    );
    assert_eq!(result.bytes.as_deref(), Some(b"done".as_slice()));
    assert_eq!(result.result["details"]["applied"], 2);
    assert_ne!(result.result["isError"], true);
    let inserted = apply(Some(b"abc"), json!([{"oldText":"","newText":"start"}]));
    assert_eq!(inserted.bytes.as_deref(), Some(b"startabc".as_slice()));
}
