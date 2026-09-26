//! Fixed, independently paginated detail queries. Variables never become query
//! text. Read results are checked for target and page identity in the shared
//! service, not only by one renderer. These are display reads, not admissions.
use super::{
    operations::Operation,
    service::{ApiError, Result},
};
use serde_json::{Value, json};

pub fn repository_target(id: &str) -> bool {
    matches!(
        id,
        "discussions.list" | "discussions.categories" | "discussions.get"
    )
}
pub fn query(id: &str) -> Option<&'static str> {
    Some(match id {
        "projects.list" => "query($login:String!,$after:String){organization(login:$login){login projectsV2(first:30,after:$after){nodes{id number title url} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}",
        "projects.user_list" => "query($login:String!,$after:String){user(login:$login){login projectsV2(first:30,after:$after){nodes{id number title url} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}",
        "discussions.list" => "query($owner:String!,$name:String!,$after:String){repository(owner:$owner,name:$name){id nameWithOwner discussions(first:30,after:$after){nodes{id number title body url} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}",
        "projects.field_users" => "query($id:ID!,$fieldId:ID!,$fieldName:String!,$after:String){field:node(id:$fieldId){id} node(id:$id){... on ProjectV2Item{id fieldValueByName(name:$fieldName){__typename ... on ProjectV2ItemFieldUserValue{field{... on ProjectV2FieldCommon{id name}} values:users(first:30,after:$after){nodes{id login name url} pageInfo{hasNextPage endCursor}}}}}} rateLimit{cost remaining resetAt}}",
        "projects.field_labels" => "query($id:ID!,$fieldId:ID!,$fieldName:String!,$after:String){field:node(id:$fieldId){id} node(id:$id){... on ProjectV2Item{id fieldValueByName(name:$fieldName){__typename ... on ProjectV2ItemFieldLabelValue{field{... on ProjectV2FieldCommon{id name}} values:labels(first:30,after:$after){nodes{id name color description url} pageInfo{hasNextPage endCursor}}}}}} rateLimit{cost remaining resetAt}}",
        "projects.field_pull_requests" => "query($id:ID!,$fieldId:ID!,$fieldName:String!,$after:String){field:node(id:$fieldId){id} node(id:$id){... on ProjectV2Item{id fieldValueByName(name:$fieldName){__typename ... on ProjectV2ItemFieldPullRequestValue{field{... on ProjectV2FieldCommon{id name}} values:pullRequests(first:30,after:$after){nodes{id number title url state} pageInfo{hasNextPage endCursor}}}}}} rateLimit{cost remaining resetAt}}",
        "projects.field_reviewers" => "query($id:ID!,$fieldId:ID!,$fieldName:String!,$after:String){field:node(id:$fieldId){id} node(id:$id){... on ProjectV2Item{id fieldValueByName(name:$fieldName){__typename ... on ProjectV2ItemFieldReviewerValue{field{... on ProjectV2FieldCommon{id name}} values:reviewers(first:30,after:$after){nodes{__typename ... on User{id login url} ... on Team{id name slug url} ... on Bot{id login url} ... on Mannequin{id login url} ... on EnterpriseTeam{id name slug}} pageInfo{hasNextPage endCursor}}}}}} rateLimit{cost remaining resetAt}}",
        "projects.items" => {
            "query($id:ID!,$after:String){node(id:$id){... on ProjectV2{id title items(first:30,after:$after){nodes{id content{... on Issue{title url} ... on PullRequest{title url} ... on DraftIssue{title body}}} pageInfo{hasNextPage endCursor}}}} rateLimit{cost remaining resetAt}}"
        }
        "discussions.categories" => {
            "query($owner:String!,$name:String!,$after:String){repository(owner:$owner,name:$name){id nameWithOwner discussionCategories(first:30,after:$after){nodes{id name description isAnswerable} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}"
        }
        "discussions.get" => {
            "query($owner:String!,$name:String!,$number:Int!,$after:String){repository(owner:$owner,name:$name){nameWithOwner discussion(number:$number){id number title body url closed category{id name} comments(first:30,after:$after){nodes{id body url isAnswer author{login}} pageInfo{hasNextPage endCursor}}}} rateLimit{cost remaining resetAt}}"
        }
        "discussions.replies" => {
            "query($id:ID!,$after:String){node(id:$id){... on DiscussionComment{id body discussion{id number title url repository{nameWithOwner}} replies(first:30,after:$after){nodes{id body url isAnswer author{login}} pageInfo{hasNextPage endCursor}}}} rateLimit{cost remaining resetAt}}"
        }
        "projects.get" => {
            "query($id:ID!){node(id:$id){... on ProjectV2{id number title url shortDescription readme closed owner{... on User{login} ... on Organization{login}}}} rateLimit{cost remaining resetAt}}"
        }
        "projects.fields" => {
            "query($id:ID!,$after:String){node(id:$id){... on ProjectV2{id title fields(first:30,after:$after){nodes{__typename ... on ProjectV2FieldCommon{id name dataType} ... on ProjectV2SingleSelectField{options{id name}} ... on ProjectV2MultiSelectField{options:multiSelectOptions{id name}} ... on ProjectV2IterationField{configuration{iterations{id title startDate duration} completedIterations{id title startDate duration}}}} pageInfo{hasNextPage endCursor}}}} rateLimit{cost remaining resetAt}}"
        }
        "projects.item_values" => {
            "query($id:ID!,$after:String){node(id:$id){... on ProjectV2Item{id isArchived project{id title url} content{__typename ... on Issue{title url} ... on PullRequest{title url} ... on DraftIssue{title body}} fieldValues(first:30,after:$after){nodes{__typename ... on ProjectV2ItemFieldValueCommon{id field{... on ProjectV2FieldCommon{id name dataType}}} ... on ProjectV2ItemFieldTextValue{text} ... on ProjectV2ItemFieldNumberValue{number} ... on ProjectV2ItemFieldDateValue{date} ... on ProjectV2ItemFieldSingleSelectValue{name optionId} ... on ProjectV2ItemFieldMultiSelectValue{value options{id name}} ... on ProjectV2ItemFieldIterationValue{title iterationId startDate duration} ... on ProjectV2ItemFieldUserValue{field{... on ProjectV2FieldCommon{id name dataType}}} ... on ProjectV2ItemFieldLabelValue{field{... on ProjectV2FieldCommon{id name dataType}}} ... on ProjectV2ItemFieldPullRequestValue{field{... on ProjectV2FieldCommon{id name dataType}}} ... on ProjectV2ItemFieldReviewerValue{field{... on ProjectV2FieldCommon{id name dataType}}} ... on ProjectV2ItemFieldMilestoneValue{field{... on ProjectV2FieldCommon{id name}} milestone{title number url}} ... on ProjectV2ItemFieldRepositoryValue{field{... on ProjectV2FieldCommon{id name}} repository{nameWithOwner url}}} pageInfo{hasNextPage endCursor}}}} rateLimit{cost remaining resetAt}}"
        }
        _ => return None,
    })
}
pub fn schema(id: &str) -> Option<Value> {
    query(id)?;
    let node =
        json!({"type":"string","minLength":1,"maxLength":1024,"pattern":"^[A-Za-z0-9_+=/-]+$"});
    let name = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.-]+$"});
    let (mut required, mut properties) = if matches!(id, "projects.list" | "projects.user_list") {
        (json!(["login"]), json!({"login":name}))
    } else if repository_target(id) {
        (json!(["owner", "name"]), json!({"owner":name,"name":name}))
    } else {
        (json!(["id"]), json!({"id":node}))
    };
    if field_type(id).is_some() {
        required.as_array_mut().unwrap().extend([json!("fieldId"), json!("fieldName")]);
        properties["fieldId"] = node;
        properties["fieldName"] = json!({"type":"string","minLength":1,"maxLength":1024});
    }
    if id == "discussions.get" {
        required.as_array_mut().unwrap().push(json!("number"));
        properties["number"] = json!({"type":"integer","minimum":1,"maximum":2147483647});
    }
    if id != "projects.get" {
        properties["after"] =
            json!({"type":"string","nullable":true,"minLength":1,"maxLength":4096});
    }
    Some(
        json!({"id":id,"reviewed":"2026-09-10","documentation":if id.starts_with("projects."){"https://docs.github.com/en/graphql/reference/projects"}else{"https://docs.github.com/en/graphql/reference/discussions"},"note":"Each connection has its own cursor and explicit 30-row page. Node-ID requests are account-wide; a selected repository does not constrain them. Detail reads never grant write permission. Non-connection option/iteration arrays are bounded by native response limits. Nested people, labels, pull requests and reviewers use separate field pages bound to the item, field ID and field name. Renamed or mismatched fields require refresh.","parameters":{"type":"object","required":required,"properties":properties,"additionalProperties":false}}),
    )
}
pub fn field_type(id: &str) -> Option<&'static str> {
    match id {
        "projects.field_users" => Some("ProjectV2ItemFieldUserValue"),
        "projects.field_labels" => Some("ProjectV2ItemFieldLabelValue"),
        "projects.field_pull_requests" => Some("ProjectV2ItemFieldPullRequestValue"),
        "projects.field_reviewers" => Some("ProjectV2ItemFieldReviewerValue"),
        _ => None,
    }
}
fn malformed() -> ApiError {
    ApiError::new(
        "graphql_response",
        "GitHub returned a malformed, oversized or mismatched detail page. Refresh the target; no change was authorized.",
    )
}

pub fn validate(operation: &Operation, data: &Value) -> Result<()> {
    if query(&operation.id).is_none() {
        return Ok(());
    }
    let variables = &operation.body.as_ref().ok_or_else(malformed)?["variables"];
    if !data["data"].is_object()
        || data
            .get("errors")
            .is_some_and(|e| !e.as_array().is_some_and(Vec::is_empty))
    {
        return Err(malformed());
    }
    let root = if repository_target(&operation.id) {
        let repo = &data["data"]["repository"];
        let expected = format!(
            "{}/{}",
            variables["owner"].as_str().ok_or_else(malformed)?,
            variables["name"].as_str().ok_or_else(malformed)?
        );
        if !repo["nameWithOwner"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(&expected))
        {
            return Err(malformed());
        }
        repo
    } else if matches!(operation.id.as_str(), "projects.list" | "projects.user_list") {
        let owner = &data["data"][if operation.id == "projects.list" { "organization" } else { "user" }];
        if !owner["login"].as_str().zip(variables["login"].as_str()).is_some_and(|(actual, expected)|actual.eq_ignore_ascii_case(expected)) { return Err(malformed()); }
        owner
    } else {
        let node = &data["data"]["node"];
        if node["id"] != variables["id"] || !node["id"].is_string() {
            return Err(malformed());
        }
        node
    };
    let connection = match operation.id.as_str() {
        "discussions.list" => &root["discussions"],
        "projects.list" | "projects.user_list" => &root["projectsV2"],
        "discussions.categories" => &root["discussionCategories"],
        "discussions.get" => {
            if root["discussion"]["number"] != variables["number"]
                || !root["discussion"]["id"].is_string()
            {
                return Err(malformed());
            }
            &root["discussion"]["comments"]
        }
        "discussions.replies" => &root["replies"],
        "projects.fields" => &root["fields"],
        "projects.items" => &root["items"],
        "projects.item_values" => &root["fieldValues"],
        "projects.get" => return bounded(root, 0),
        id if field_type(id).is_some() => {
            let value = &root["fieldValueByName"];
            if value["__typename"].as_str() != field_type(id)
                || value["field"]["id"] != variables["fieldId"]
                || data["data"]["field"]["id"] != variables["fieldId"]
                || value["field"]["name"] != variables["fieldName"] {
                return Err(malformed());
            }
            &value["values"]
        }
        _ => return Err(malformed()),
    };
    let rows = connection["nodes"].as_array().ok_or_else(malformed)?;
    let info = &connection["pageInfo"];
    if rows.len() > 30
        || rows.iter().any(|row| !row.is_object() && !row.is_null())
        || !info["hasNextPage"].is_boolean()
    {
        return Err(malformed());
    }
    let cursor = &info["endCursor"];
    if !(cursor.is_null()
        || cursor
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 4096))
        || info["hasNextPage"] == true && (cursor.is_null() || cursor == &variables["after"])
    {
        return Err(malformed());
    }
    bounded(root, 0)
}
pub(super) fn bounded(value: &Value, depth: usize) -> Result<()> {
    if depth > 24 {
        return Err(malformed());
    }
    match value {
        Value::Array(rows) => {
            if rows.len() > 1000 {
                return Err(malformed());
            }
            for row in rows {
                bounded(row, depth + 1)?;
            }
        }
        Value::Object(map) => {
            if map.len() > 128 {
                return Err(malformed());
            }
            for (key, value) in map {
                if !value.is_null() && (["id", "title", "body", "readme", "shortDescription", "name", "login", "nameWithOwner", "url", "__typename", "dataType", "optionId", "iterationId", "startDate", "date", "text"].contains(&key.as_str()) && !value.is_string()
                    || ["closed", "isArchived", "isAnswer"].contains(&key.as_str()) && !value.is_boolean()) { return Err(malformed()); }
                bounded(value, depth + 1)?;
            }
        }
        Value::String(s) if s.len() > 1024 * 1024 => return Err(malformed()),
        _ => {}
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
        sync::{Arc, Mutex, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    fn request(id: &str, after: Value) -> Request {
        let mut parameters = if matches!(id, "projects.list" | "projects.user_list") {
            json!({"login":"owned"})
        } else if repository_target(id) {
            json!({"owner":"owned","name":"repo"})
        } else {
            json!({"id":"P_owned"})
        };
        if field_type(id).is_some() { parameters["fieldId"]=json!("F_owned"); parameters["fieldName"]=json!("Literal field"); }
        if id == "discussions.get" {
            parameters["number"] = json!(7);
        }
        if id != "projects.get" {
            parameters["after"] = after;
        }
        serde_json::from_value(json!({"operation":id,"parameters":parameters})).unwrap()
    }
    fn data(id: &str) -> Value {
        let page = json!({"nodes":[{"id":"row","name":"Literal <script>value</script>"},null],"pageInfo":{"hasNextPage":true,"endCursor":"cursor-two"}});
        match id {
            "discussions.list" => json!({"data":{"repository":{"nameWithOwner":"owned/repo","discussions":page}}}),
            "projects.list" => json!({"data":{"organization":{"login":"owned","projectsV2":page}}}),
            "projects.user_list" => json!({"data":{"user":{"login":"owned","projectsV2":page}}}),
            id if field_type(id).is_some() => json!({"data":{"field":{"id":"F_owned"},"node":{"id":"P_owned","fieldValueByName":{"__typename":field_type(id),"field":{"id":"F_owned","name":"Literal field"},"values":page}}}}),
            "discussions.categories" => {
                json!({"data":{"repository":{"nameWithOwner":"owned/repo","discussionCategories":page}}})
            }
            "discussions.get" => {
                json!({"data":{"repository":{"nameWithOwner":"owned/repo","discussion":{"id":"D_owned","number":7,"comments":page}}}})
            }
            "discussions.replies" => json!({"data":{"node":{"id":"P_owned","replies":page}}}),
            "projects.fields" => json!({"data":{"node":{"id":"P_owned","fields":page}}}),
            "projects.items" => json!({"data":{"node":{"id":"P_owned","items":page}}}),
            "projects.item_values" => json!({"data":{"node":{"id":"P_owned","fieldValues":page}}}),
            "projects.get" => json!({"data":{"node":{"id":"P_owned","title":"Owned project"}}}),
            _ => panic!("fixture"),
        }
    }
    struct Fixture {
        value: Mutex<Value>,
    }
    impl Transport for Fixture {
        fn account(&self, c: &ScanControl<'_>) -> Result<Account> {
            check(c)?;
            Account::bound(1, "owned".into(), "owned-credential".into())
        }
        fn execute(&self, expected: &Account, _: &Operation, c: &ScanControl<'_>) -> Result<Reply> {
            assert!(expected == &self.account(c)?);
            Ok(Reply {
                data: self.value.lock().unwrap().clone(),
                status: 200,
                next: None,
                rate_remaining: None,
                rate_reset: None, rate_resource: None, rate_limit: None,
                outcome: Outcome::Read,
            })
        }
    }
    #[test]
    fn typed_detail_reads_bind_target_and_bounded_cursor_pages_for_ui_and_model() {
        for id in [
            "projects.list", "projects.user_list", "discussions.list",
            "projects.field_users", "projects.field_labels", "projects.field_pull_requests", "projects.field_reviewers",
            "discussions.categories",
            "discussions.get",
            "discussions.replies",
            "projects.get",
            "projects.fields",
            "projects.items",
            "projects.item_values",
        ] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(temp.path().join("github-connected"), b"owned").unwrap();
            let fixture = Arc::new(Fixture {
                value: Mutex::new(data(id)),
            });
            let service = Service::new(temp.path().into(), fixture.clone());
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            let compiled = request(id, json!("cursor-one")).compile().unwrap();
            assert_eq!(compiled.body.as_ref().unwrap()["query"], query(id).unwrap());
            assert!(validate(&compiled, &data(id)).is_ok(), "{id}");
            assert!(
                service
                    .read(vec![request(id, json!("cursor-one"))], "scope", &c)
                    .is_ok()
            );
            let prepared = service
                .prepare(request(id, json!("cursor-one")), "scope", &c)
                .unwrap();
            assert!(service.execute(prepared, "scope", &c).is_ok());
            for bad in [
                json!({"data":{"node":{"id":"wrong"},"repository":{"nameWithOwner":"other/repo"}}}),
                json!({"data":null}),
                json!({"errors":[{"message":"permission denied"}],"data":data(id)["data"]}),
            ] {
                *fixture.value.lock().unwrap() = bad;
                assert!(
                    service
                        .read(vec![request(id, Value::Null)], "scope", &c)
                        .is_err(),
                    "{id}"
                );
                let prepared = service
                    .prepare(request(id, Value::Null), "scope", &c)
                    .unwrap();
                assert!(service.execute(prepared, "scope", &c).is_err(), "{id}");
            }
            let mut r = request(id, Value::Null);
            r.parameters.insert("unexpected".into(), json!(true));
            assert!(r.compile().is_err());
            if id != "projects.get" {
                let r = request(id, json!(false));
                assert!(r.compile().is_err());
                assert!(request(id, json!("x".repeat(4097))).compile().is_err());
                assert!(
                    validate(
                        &request(id, json!("cursor-two")).compile().unwrap(),
                        &data(id)
                    )
                    .is_err()
                );
            }
            if repository_target(id) {
                let mut r = request(id, Value::Null);
                r.repo = Some("other/repo".into());
                assert!(r.compile().is_err());
            } else {
                let mut r = request(id, Value::Null);
                r.repo = Some("owned/repo".into());
                assert!(r.compile().is_err());
            }
        }
    }
    #[test]
    fn nested_fields_reject_changed_type_identity_name_and_cursor_without_authority() {
        for id in ["projects.field_users", "projects.field_labels", "projects.field_pull_requests", "projects.field_reviewers"] {
            let op = request(id, Value::Null).compile().unwrap();
            for path in ["/data/node/id", "/data/field/id", "/data/node/fieldValueByName/field/id", "/data/node/fieldValueByName/field/name", "/data/node/fieldValueByName/__typename"] {
                let mut value = data(id);
                *value.pointer_mut(path).unwrap() = json!("wrong");
                assert!(validate(&op, &value).is_err(), "{id} {path}");
            }
            for key in ["fieldId", "fieldName"] {
                let mut r = request(id, Value::Null);
                r.parameters.remove(key);
                assert!(r.compile().is_err());
            }
            let mut r = request(id, Value::Null);
            r.parameters.insert("fieldName".into(), json!("name \" } mutation { literal"));
            let compiled = r.compile().unwrap();
            assert_eq!(compiled.body.unwrap()["query"], query(id).unwrap());
        }
    }
    #[test]
    fn malformed_visible_scalars_are_rejected_before_rendering() {
        let op = request("projects.get", Value::Null).compile().unwrap();
        for (key, invalid) in [("title", json!({"untrusted":"object"})), ("readme",json!(["array"])), ("closed",json!("yes"))] {
            let mut response = data("projects.get");response["data"]["node"][key]=invalid;
            assert!(validate(&op,&response).is_err());
        }
    }
    #[test]
    fn malformed_pages_options_and_incomplete_targets_are_not_silently_truncated() {
        let op = request("projects.fields", Value::Null).compile().unwrap();
        for fields in [
            json!({}),
            json!({"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":null}}),
            json!({"nodes":[],"pageInfo":{"hasNextPage":"true","endCursor":"next"}}),
            json!({"nodes":[false],"pageInfo":{"hasNextPage":false,"endCursor":null}}),
            json!({"nodes":vec![json!({});31],"pageInfo":{"hasNextPage":false,"endCursor":null}}),
            json!({"nodes":[{"options":vec![json!({});1001]}],"pageInfo":{"hasNextPage":false,"endCursor":null}}),
        ] {
            assert!(
                validate(
                    &op,
                    &json!({"data":{"node":{"id":"P_owned","fields":fields}}})
                )
                .is_err()
            );
        }
        let op = request("discussions.get", Value::Null).compile().unwrap();
        let mut value = data("discussions.get");
        value["data"]["repository"]["discussion"]["number"] = json!(8);
        assert!(validate(&op, &value).is_err());
    }
}
