//! Typed graph changes use the same one-use native plans as REST changes.
//! A 200 response with absent/mismatched result nodes is an uncertain effect.
use super::{
    operations::Operation,
    service::{ApiError, Result},
};
use serde_json::{Value, json};

pub fn query(id: &str) -> Option<&'static str> {
    Some(match id {
        "discussions.create" => {
            "mutation($input:CreateDiscussionInput!){result:createDiscussion(input:$input){node:discussion{id title body repository{id} category{id}}}}"
        }
        "discussions.comment" => {
            "mutation($input:AddDiscussionCommentInput!){result:addDiscussionComment(input:$input){node:comment{id body discussion{id} replyTo{id}}}}"
        }
        "projects.add_item" => {
            "mutation($input:AddProjectV2ItemByIdInput!){result:addProjectV2ItemById(input:$input){node:item{id project{id} content{... on Issue{id} ... on PullRequest{id}}}}}"
        }
        "projects.update_field" => {
            "mutation($input:UpdateProjectV2ItemFieldValueInput!){result:updateProjectV2ItemFieldValue(input:$input){node:projectV2Item{id project{id}}}}"
        }
        "discussions.update" => {
            "mutation($input:UpdateDiscussionInput!){result:updateDiscussion(input:$input){node:discussion{id title body category{id}}}}"
        }
        "discussions.close" => {
            "mutation($input:CloseDiscussionInput!){result:closeDiscussion(input:$input){node:discussion{id closed}}}"
        }
        "discussions.reopen" => {
            "mutation($input:ReopenDiscussionInput!){result:reopenDiscussion(input:$input){node:discussion{id closed}}}"
        }
        "discussions.edit_comment" => {
            "mutation($input:UpdateDiscussionCommentInput!){result:updateDiscussionComment(input:$input){node:comment{id body discussion{id}}}}"
        }
        "discussions.answer" => {
            "mutation($input:MarkDiscussionCommentAsAnswerInput!){result:markDiscussionCommentAsAnswer(input:$input){node:discussion{id answer{id}}}}"
        }
        "discussions.unanswer" => {
            "mutation($input:UnmarkDiscussionCommentAsAnswerInput!){result:unmarkDiscussionCommentAsAnswer(input:$input){node:discussion{id answer{id}}}}"
        }
        "projects.update" => {
            "mutation($input:UpdateProjectV2Input!){result:updateProjectV2(input:$input){node:projectV2{id title readme shortDescription closed public}}}"
        }
        "projects.clear_field" => {
            "mutation($input:ClearProjectV2ItemFieldValueInput!){result:clearProjectV2ItemFieldValue(input:$input){node:projectV2Item{id project{id}}}}"
        }
        "projects.add_draft" => {
            "mutation($input:AddProjectV2DraftIssueInput!){result:addProjectV2DraftIssue(input:$input){node:projectItem{id project{id} content{... on DraftIssue{id title body}}}}}"
        }
        "projects.archive" => {
            "mutation($input:ArchiveProjectV2ItemInput!){result:archiveProjectV2Item(input:$input){node:item{id isArchived project{id}}}}"
        }
        "projects.unarchive" => {
            "mutation($input:UnarchiveProjectV2ItemInput!){result:unarchiveProjectV2Item(input:$input){node:item{id isArchived project{id}}}}"
        }
        _ => return None,
    })
}

pub fn schema(id: &str) -> Option<Value> {
    query(id)?;
    let node =
        json!({"type":"string","minLength":1,"maxLength":1024,"pattern":"^[A-Za-z0-9_+=/-]+$"});
    let text = json!({"type":"string","maxLength":60000});
    let title = json!({"type":"string","minLength":1,"maxLength":256});
    let (required, mut fields) = match id {
        "discussions.update" => (
            json!(["discussionId"]),
            json!({"discussionId":node,"title":title,"body":text,"categoryId":node}),
        ),
        "discussions.close" => (
            json!(["discussionId"]),
            json!({"discussionId":node,"reason":{"type":"string","enum":["DUPLICATE","OUTDATED","RESOLVED"]}}),
        ),
        "discussions.reopen" => (json!(["discussionId"]), json!({"discussionId":node})),
        "discussions.edit_comment" => (
            json!(["commentId", "body"]),
            json!({"commentId":node,"body":text}),
        ),
        "discussions.answer" | "discussions.unanswer" => (json!(["id"]), json!({"id":node})),
        "projects.update" => (
            json!(["projectId"]),
            json!({"projectId":node,"title":title,"readme":text,"shortDescription":{"type":"string","maxLength":256},"closed":{"type":"boolean"},"public":{"type":"boolean"}}),
        ),
        "projects.add_draft" => (
            json!(["projectId", "title"]),
            json!({"projectId":node,"title":title,"body":text,"assigneeIds":{"type":"array","maxItems":100,"items":node}}),
        ),
        "projects.clear_field" => (
            json!(["projectId", "itemId", "fieldId"]),
            json!({"projectId":node,"itemId":node,"fieldId":node}),
        ),
        "projects.archive" | "projects.unarchive" => (
            json!(["projectId", "itemId"]),
            json!({"projectId":node,"itemId":node}),
        ),
        _ => return None,
    };
    fields["clientMutationId"] = json!({"type":"string","nullable":true,"maxLength":1024});
    Some(
        json!({"id":id,"reviewed":"2026-09-10","documentation":"https://docs.github.com/en/graphql/reference","note":"Explicit node IDs are account-wide. Review exact fields and permission requirements. Replacement updates may replace concurrent edits; accepted responses are not an atomic server snapshot. Refresh after every change.","parameters":{"type":"object","required":["input"],"additionalProperties":false,"properties":{"input":{"type":"object","required":required,"additionalProperties":false,"properties":fields}}}}),
    )
}

pub fn validate(operation: &Operation, response: &Value) -> Result<()> {
    if query(&operation.id).is_none() {
        return Ok(());
    }
    let error = || {
        ApiError::new("graphql_effect", "GitHub returned an incomplete or mismatched change result. The change may have applied; refresh before making a new request.").uncertain(operation)
    };
    let input = &operation.body.as_ref().ok_or_else(error)?["variables"]["input"];
    let node = &response["data"]["result"]["node"];
    if !response["data"].is_object()
        || response
            .get("errors")
            .is_some_and(|v| !v.as_array().is_some_and(Vec::is_empty))
        || !node["id"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 1024)
    {
        return Err(error());
    }
    let matches = match operation.id.as_str() {
        "discussions.create" => {
            node["repository"]["id"] == input["repositoryId"]
                && node["category"]["id"] == input["categoryId"]
        }
        "discussions.comment" => {
            node["discussion"]["id"] == input["discussionId"]
                && (input["replyToId"].is_null() || node["replyTo"]["id"] == input["replyToId"])
        }
        "projects.add_item" => {
            node["project"]["id"] == input["projectId"]
                && node["content"]["id"] == input["contentId"]
        }
        "projects.update_field" => {
            node["project"]["id"] == input["projectId"] && node["id"] == input["itemId"]
        }
        "discussions.update" => node["id"] == input["discussionId"],
        "discussions.close" => node["id"] == input["discussionId"] && node["closed"] == true,
        "discussions.reopen" => node["id"] == input["discussionId"] && node["closed"] == false,
        "discussions.edit_comment" => node["id"] == input["commentId"],
        "discussions.answer" => node["answer"]["id"] == input["id"],
        "discussions.unanswer" => node.get("answer").is_some_and(Value::is_null),
        "projects.update" => node["id"] == input["projectId"],
        "projects.add_draft" => {
            node["project"]["id"] == input["projectId"] && node["content"]["id"].is_string()
        }
        "projects.clear_field" => {
            node["id"] == input["itemId"] && node["project"]["id"] == input["projectId"]
        }
        "projects.archive" | "projects.unarchive" => {
            node["id"] == input["itemId"]
                && node["project"]["id"] == input["projectId"]
                && node["isArchived"] == (operation.id == "projects.archive")
        }
        _ => false,
    };
    if !matches {
        return Err(error());
    }
    super::graph_reads::bounded(node, 0).map_err(|_| error())?;
    // Every edited scalar selected by the fixed query must be present and match.
    // This is a response check, not a compare-and-swap guarantee against later edits.
    let values = if operation.id == "projects.add_draft" {
        &node["content"]
    } else {
        node
    };
    for field in [
        "title",
        "body",
        "readme",
        "shortDescription",
        "closed",
        "public",
    ] {
        if input.get(field).is_some() && values.get(field) != input.get(field) {
            return Err(error());
        }
    }
    if input.get("categoryId").is_some() && node["category"]["id"] != input["categoryId"] {
        return Err(error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{
        operations::Request,
        service::{Account, Outcome, Reply, Service, Transport, check},
    };
    use super::*;
    use crate::resource_fs::ScanControl;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };
    struct Fixture {
        value: Mutex<Value>,
        sends: AtomicUsize,
        deny: AtomicBool,
    }
    impl Transport for Fixture {
        fn account(&self, c: &ScanControl<'_>) -> Result<Account> {
            check(c)?;
            Account::bound(1, "owned".into(), "fixture".into())
        }
        fn execute(&self, a: &Account, _: &Operation, c: &ScanControl<'_>) -> Result<Reply> {
            assert!(a == &self.account(c)?);
            self.sends.fetch_add(1, Ordering::SeqCst);
            if self.deny.load(Ordering::SeqCst) {
                return Err(ApiError::new("permission", "Denied by owned fixture."));
            }
            Ok(Reply {
                data: self.value.lock().unwrap().clone(),
                status: 200,
                next: None,
                rate_remaining: None,
                rate_reset: None,
                rate_resource: None,
                rate_limit: None,
                outcome: Outcome::Applied,
            })
        }
    }
    fn cases() -> Vec<(&'static str, Value, Value)> {
        vec![
            (
                "discussions.create",
                json!({"repositoryId":"R","categoryId":"CAT","title":"New","body":"Body"}),
                json!({"id":"D","repository":{"id":"R"},"category":{"id":"CAT"},"title":"New","body":"Body"}),
            ),
            (
                "discussions.comment",
                json!({"discussionId":"D","replyToId":"C","body":"Reply"}),
                json!({"id":"REPLY","discussion":{"id":"D"},"replyTo":{"id":"C"},"body":"Reply"}),
            ),
            (
                "projects.add_item",
                json!({"projectId":"P","contentId":"ISSUE"}),
                json!({"id":"I","project":{"id":"P"},"content":{"id":"ISSUE"}}),
            ),
            (
                "projects.update_field",
                json!({"projectId":"P","itemId":"I","fieldId":"F","value":{"text":"Value"}}),
                json!({"id":"I","project":{"id":"P"}}),
            ),
            (
                "discussions.update",
                json!({"discussionId":"D","title":"New title","body":"Literal <script>text</script>","categoryId":"CAT"}),
                json!({"id":"D","title":"New title","body":"Literal <script>text</script>","category":{"id":"CAT"}}),
            ),
            (
                "discussions.close",
                json!({"discussionId":"D","reason":"RESOLVED"}),
                json!({"id":"D","closed":true}),
            ),
            (
                "discussions.reopen",
                json!({"discussionId":"D"}),
                json!({"id":"D","closed":false}),
            ),
            (
                "discussions.edit_comment",
                json!({"commentId":"C","body":"Changed"}),
                json!({"id":"C","body":"Changed","discussion":{"id":"D"}}),
            ),
            (
                "discussions.answer",
                json!({"id":"C"}),
                json!({"id":"D","answer":{"id":"C"}}),
            ),
            (
                "discussions.unanswer",
                json!({"id":"C"}),
                json!({"id":"D","answer":null}),
            ),
            (
                "projects.update",
                json!({"projectId":"P","title":"New project","closed":false,"public":false}),
                json!({"id":"P","title":"New project","closed":false,"public":false}),
            ),
            (
                "projects.clear_field",
                json!({"projectId":"P","itemId":"I","fieldId":"F"}),
                json!({"id":"I","project":{"id":"P"}}),
            ),
            (
                "projects.add_draft",
                json!({"projectId":"P","title":"Draft","body":"Body"}),
                json!({"id":"I","project":{"id":"P"},"content":{"id":"DI","title":"Draft","body":"Body"}}),
            ),
            (
                "projects.archive",
                json!({"projectId":"P","itemId":"I"}),
                json!({"id":"I","project":{"id":"P"},"isArchived":true}),
            ),
            (
                "projects.unarchive",
                json!({"projectId":"P","itemId":"I"}),
                json!({"id":"I","project":{"id":"P"},"isArchived":false}),
            ),
        ]
    }
    fn request(id: &str, input: Value) -> Request {
        serde_json::from_value(json!({"operation":id,"parameters":{"input":input}})).unwrap()
    }
    #[test]
    fn graph_changes_require_fresh_confirmation_and_validate_semantic_responses_without_replay() {
        for (id, input, node) in cases() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("github-connected"), b"owned").unwrap();
            let response = json!({"data":{"result":{"node":node}}});
            let f = Arc::new(Fixture {
                value: Mutex::new(response.clone()),
                sends: AtomicUsize::new(0),
                deny: AtomicBool::new(false),
            });
            let s = Service::new(tmp.path().into(), f.clone());
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            assert!(
                s.read(vec![request(id, input.clone())], "scope", &c)
                    .is_err()
            );
            assert_eq!(f.sends.load(Ordering::SeqCst), 0);
            let preview = s.preview(request(id, input.clone()), "scope", &c).unwrap();
            assert_eq!(f.sends.load(Ordering::SeqCst), 0);
            let token = preview["confirmation"].as_str().unwrap();
            assert!(s.confirm(token, "scope", &c).is_ok(), "{id}");
            assert!(s.confirm(token, "scope", &c).is_err());
            assert_eq!(f.sends.load(Ordering::SeqCst), 1);
            for bad in [
                json!({"data":null}),
                json!({"data":{"result":{"node":null}}}),
                json!({"data":response["data"],"errors":[{"message":"partial"}]}),
            ] {
                *f.value.lock().unwrap() = bad;
                let p = s.prepare(request(id, input.clone()), "scope", &c).unwrap();
                assert_eq!(
                    s.execute(p, "scope", &c).unwrap_err().outcome,
                    Outcome::Unknown,
                    "{id}"
                );
            }
            f.deny.store(true, Ordering::SeqCst);
            let p = s.prepare(request(id, input.clone()), "scope", &c).unwrap();
            assert_eq!(s.execute(p, "scope", &c).unwrap_err().code, "permission");
            let p = s.prepare(request(id, input.clone()), "scope", &c).unwrap();
            let before = f.sends.load(Ordering::SeqCst);
            cancel.store(true, Ordering::SeqCst);
            assert!(s.execute(p, "scope", &c).is_err());
            assert_eq!(before, f.sends.load(Ordering::SeqCst));
            let mut invalid = input.clone();
            invalid["unexpected"] = json!(true);
            assert!(request(id, invalid).compile().is_err());
            let mut r = request(id, input.clone());
            r.repo = Some("owned/repo".into());
            assert!(r.compile().is_err());
        }
    }
    #[test]
    fn scalar_and_state_mismatches_cannot_be_reported_as_success() {
        for (id, input, mut node) in cases() {
            let op = request(id, input).compile().unwrap();
            let response = json!({"data":{"result":{"node":node}}});
            assert!(validate(&op, &response).is_ok(), "{id}");
            match id {
                "discussions.create" => node["category"]["id"] = json!("wrong"),
                "discussions.comment" => node["replyTo"]["id"] = json!("wrong"),
                "projects.add_item" => node["content"]["id"] = json!("wrong"),
                "discussions.answer" => node["answer"]["id"] = json!("wrong"),
                "discussions.unanswer" => node["answer"] = json!({"id":"still-answered"}),
                "projects.add_draft" => node["content"]["title"] = json!("wrong"),
                "discussions.close" | "discussions.reopen" => node["closed"] = json!("false"),
                "projects.archive" | "projects.unarchive" => node["isArchived"] = Value::Null,
                _ => node["id"] = json!("wrong"),
            }
            assert_eq!(
                validate(&op, &json!({"data":{"result":{"node":node}}}))
                    .unwrap_err()
                    .outcome,
                Outcome::Unknown,
                "{id}"
            );
        }
    }
}
