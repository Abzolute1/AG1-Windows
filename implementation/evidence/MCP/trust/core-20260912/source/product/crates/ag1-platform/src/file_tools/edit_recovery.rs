//! Recovery advice only: exact matching, uniqueness and publication rules stay unchanged.
use serde_json::{Value, json};
pub(super) fn explain(error: &str, path: &str, source: &str, args: &Value) -> String {
    if error.starts_with("No changes made") {
        return format!("{error}\nNo edit was applied. Inspect the intended diff or run the relevant check; do not resubmit a replacement that leaves the current content unchanged.");
    }
    if !error.contains("Could not find")
        && !error.contains("occurrences")
        && !error.contains("overlap")
    {
        return error.into();
    }
    let mut advice = format!(
        "{error}\nNo changes from this call were written. Do not repeat the same arguments. Read a small current range, copy its exact indentation, and use one unique, non-overlapping replacement."
    );
    let Some(edits) = args["edits"].as_array() else {
        return advice;
    };
    let lines: Vec<_> = source.lines().take(100_000).collect();
    let mut locations = std::collections::BTreeSet::new();
    let mut comparisons = 250_000usize;
    for edit in edits.iter().take(8) {
        let Some(old) = edit["oldText"].as_str() else {
            continue;
        };
        if !old.is_empty() && source.matches(old).take(2).count() == 1 {
            continue;
        }
        let wanted: Vec<_> = old.lines().collect();
        if wanted.is_empty() || wanted.len() > 128 || old.len() > 8192 {
            continue;
        }
        for (index, window) in lines.windows(wanted.len()).enumerate() {
            if comparisons < wanted.len() {
                return advice;
            }
            comparisons -= wanted.len();
            if !window
                .iter()
                .zip(&wanted)
                .all(|(a, b)| a.trim() == b.trim())
            {
                continue;
            }
            if !locations.insert(index) {
                continue;
            }
            let offset = index.saturating_sub(2) + 1;
            advice.push_str(&format!("\nCandidate at line {} (whitespace-insensitive location only; not permission to apply): read {}", index+1, json!({"path":path,"offset":offset,"limit":wanted.len()+4})));
            let actual = window.join("\n");
            if locations.len() == 1 && actual.len() <= 2048 {
                advice.push_str(&format!(
                    "\nLF-normalized candidate text as a JSON string: {}",
                    json!(actual)
                ));
            }
            if locations.len() >= 3 {
                return advice;
            }
        }
    }
    advice
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locates_wrong_indentation_without_changing_matching_semantics() {
        let source = "before\n    if (ready) {\n      run();\n    }\nafter\n";
        let result = explain(
            "Could not find the exact text",
            "src/game.ts",
            source,
            &json!({"edits":[{"oldText":"      if (ready) {\n        run();"}]}),
        );
        assert!(result.contains("Candidate at line 2"));
        assert!(result.contains(r#"\n      run();"#));
        assert!(result.contains(r#""offset":1"#));
        assert!(result.contains("No changes from this call were written"));
    }
    #[test]
    fn bounds_duplicate_locations_and_does_not_invent_a_match() {
        let args = json!({"edits":[{"oldText":"  run();"}]});
        let hint = explain("Found 40 occurrences", "a", &"run();\n".repeat(40), &args);
        assert_eq!(hint.matches("Candidate at line").count(), 3);
        let hint = explain("Could not find", "a", "different();", &args);
        assert!(!hint.contains("Candidate"));
        assert_eq!(
            explain("schema invalid", "a", "run();", &args),
            "schema invalid"
        );
    }
}
