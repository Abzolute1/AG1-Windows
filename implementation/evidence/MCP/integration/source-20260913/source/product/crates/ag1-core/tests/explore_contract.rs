use ag1_core::runtime::subagent::explore;
use serde_json::json;

#[test]
fn explore_board_retains_order_evidence_and_reference_presentation() {
    assert_eq!(
        explore::questions(&json!({"questions":["  One "," ","Two"]})).unwrap(),
        vec!["One", "Two"]
    );
    assert!(explore::questions(&json!({"questions":[" "]})).is_err());
    let questions = vec!["Where?".into(), "Why?".into()];
    let results = vec![
        json!({"type":"explore","state":"completed","summary":" Answer α ","evidence":[{"kind":"file","action":"read","path":"src/a.rs"}]}),
        json!({"type":"explore","state":"timeout","summary":"","evidence":[]}),
    ];
    let findings = explore::merge(&questions, &results, 17).unwrap();
    assert_eq!(
        explore::report(&findings).unwrap(),
        "Explore report (2 workers):\n\n1. Where? [completed]\nAnswer α\nEvidence:\n- read: src/a.rs\n\n2. Why? [timeout]\n(no output)"
    );
    let mut board = explore::empty();
    board["unknown"] = json!({"false":false,"zero":0});
    for at in 0..8 {
        board = explore::append(&board, &findings, at).unwrap();
    }
    assert_eq!(board["findings"].as_array().unwrap().len(), 12);
    assert_eq!(board["unknown"], json!({"false":false,"zero":0}));
    assert_eq!(explore::prior(&board).unwrap().lines().count(), 8);
    let mut rollup = json!({"at":17,"findings":findings,"results":results});
    explore::validate_rollup(&rollup, &json!({"questions":questions})).unwrap();
    rollup["findings"][0]["summary"] = json!("fabricated");
    assert!(explore::validate_rollup(&rollup, &json!({"questions":questions})).is_err());
    for corrupt in [
        json!({"version":2,"findings":[]}),
        json!({"version":1,"findings":[{}]}),
    ] {
        assert!(explore::append(&corrupt, &[], 0).is_err());
    }
}
