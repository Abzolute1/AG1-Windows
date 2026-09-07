use ag1_core::runtime::tree;
use serde_json::{Value, json};
#[test]
fn branch_summary_hygiene_and_budget_match_the_pinned_sdk() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/tree-oracle.json"
    ))
    .unwrap();
    for row in oracle["collections"].as_array().unwrap() {
        let journal = ag1_core::storage::StoredJournal {
            revision: 9,
            committed_sequence: 9,
            header: b"{}".to_vec(),
            leaf: row["old"].as_str().map(str::to_owned),
            entries: oracle["entries"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, e)| ag1_core::storage::StoredEntry {
                    id: e["id"].as_str().unwrap().into(),
                    parent: e["parentId"].as_str().map(str::to_owned),
                    physical_order: i as u64,
                    raw: serde_json::to_vec(e).unwrap(),
                    normalized: serde_json::to_vec(e).unwrap(),
                })
                .collect(),
        };
        let result = tree::collect(&journal, row["target"].as_str().unwrap()).unwrap();
        assert_eq!(
            json!({"entries":result.entries,"commonAncestorId":result.common_ancestor}),
            row["result"]
        );
    }
    for row in oracle["preparations"].as_array().unwrap() {
        let result = tree::summary(
            oracle["entries"].as_array().unwrap(),
            row["budget"].as_i64().unwrap(),
        )
        .unwrap();
        assert_eq!(
            json!({"messages":result.messages,"fileOps":{"read":result.read,"written":result.written,"edited":result.edited},"totalTokens":result.tokens}),
            row["result"],
            "budget {}",
            row["budget"]
        );
    }
}
