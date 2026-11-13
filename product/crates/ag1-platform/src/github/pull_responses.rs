//! A successful HTTP exchange alone does not confirm a review or merge.
use super::{
    operations::Operation,
    service::{ApiError, Reply, Result},
};
fn sha(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(super) fn validate(op: &Operation, reply: &Reply) -> Result<()> {
    let data = &reply.data;
    let okay = match op.id.as_str() {
        "pulls.merge" => reply.status == 200 && data["merged"] == true && sha(&data["sha"]),
        "pulls.review" => {
            let body = op.body.as_ref().unwrap();
            let state = match body["event"].as_str() {
                Some("APPROVE") => "APPROVED",
                Some("REQUEST_CHANGES") => "CHANGES_REQUESTED",
                Some("COMMENT") => "COMMENTED",
                None => "PENDING",
                _ => return Err(error(op)),
            };
            reply.status == 200
                && data["id"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= 9_007_199_254_740_991)
                && sha(&data["commit_id"])
                && data["commit_id"] == body["commit_id"]
                && data["state"] == state
        }
        _ => return Ok(()),
    };
    if okay { Ok(()) } else { Err(error(op)) }
}
fn error(op: &Operation) -> ApiError {
    ApiError::new("pull_response","GitHub returned an unexpected review or merge receipt. Refresh the pull request before another change.").uncertain(op)
}
#[cfg(test)]
mod tests {
    use super::super::{operations::Request, service::Outcome};
    use super::*;
    use serde_json::{Value, json};
    #[test]
    fn review_and_merge_receipts_require_exact_commit_and_confirmed_semantics() {
        for event in [
            None,
            Some("APPROVE"),
            Some("REQUEST_CHANGES"),
            Some("COMMENT"),
        ] {
            let mut body = json!({"commit_id":"a".repeat(40),"body":"Owned review"});
            if let Some(event) = event {
                body["event"] = json!(event);
            }
            let request:Request=serde_json::from_value(json!({"operation":"pulls.review","repo":"owned/repo","parameters":{"number":7},"body":body})).unwrap();
            let op = request.compile().unwrap();
            super::super::schemas::validate(&op).unwrap();
            let state = match event {
                None => "PENDING",
                Some("APPROVE") => "APPROVED",
                Some("REQUEST_CHANGES") => "CHANGES_REQUESTED",
                _ => "COMMENTED",
            };
            let good = Reply {
                data: json!({"id":17,"commit_id":"a".repeat(40),"state":state}),
                status: 200,
                next: None,
                rate_remaining: None,
                rate_reset: None,
                rate_resource: None,
                rate_limit: None,
                outcome: Outcome::Applied,
            };
            validate(&op, &good).unwrap();
            for changed in 0..4 {
                let mut bad = good.clone();
                match changed {
                    0 => bad.data["commit_id"] = json!("b".repeat(40)),
                    1 => bad.data["state"] = json!("DISMISSED"),
                    2 => bad.data["id"] = Value::Null,
                    _ => bad.status = 201,
                };
                assert_eq!(validate(&op, &bad).unwrap_err().outcome, Outcome::Unknown);
            }
        }
        let request:Request=serde_json::from_value(json!({"operation":"pulls.merge","repo":"owned/repo","parameters":{"number":7},"body":{"sha":"a".repeat(40)}})).unwrap();
        let op = request.compile().unwrap();
        for (data, status, okay) in [
            (json!({"merged":true,"sha":"b".repeat(40)}), 200, true),
            (json!({"merged":false,"sha":"b".repeat(40)}), 200, false),
            (json!({"merged":true}), 200, false),
            (json!({"merged":true,"sha":"b".repeat(40)}), 202, false),
        ] {
            let reply = Reply {
                data,
                status,
                next: None,
                rate_remaining: None,
                rate_reset: None,
                rate_resource: None,
                rate_limit: None,
                outcome: Outcome::Applied,
            };
            assert_eq!(validate(&op, &reply).is_ok(), okay);
        }
    }
}
