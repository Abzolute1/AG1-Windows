//! One bounded operation catalog for UI discovery and model admissions.
//! No endpoint, method, risk, or credential header is accepted from a server.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Risk { Read, Write, Consequential }

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Spec {
    pub id: &'static str,
    pub area: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub risk: Risk,
}
macro_rules! specs {
    ($($area:literal => [$($id:literal, $method:literal, $path:literal, $risk:ident;)*];)*) => {
        pub const SPECS: &[Spec] = &[$($(Spec { id: $id, area: $area, method: $method, path: $path, risk: Risk::$risk },)*)*];
    };
}
specs! {
    "secrets" => [
        "secrets.set", "PUT", "native-secret-target", Consequential;
        "secrets.list", "GET", "native-secret-target", Read;
        "secrets.get", "GET", "native-secret-target", Read;
        "secrets.delete", "DELETE", "native-secret-target", Consequential;
        "secrets.repositories", "GET", "native-secret-target", Read;
        "secrets.set_repositories", "PUT", "native-secret-target", Consequential;
    ];
    "repositories" => [
        "repos.list", "GET", "user/repos", Read;
        "repos.search", "GET", "search/repositories", Read;
        "repos.get", "GET", "repos/{repo}", Read;
        "repos.contents", "GET", "repos/{repo}/contents/{path}", Read;
        "repos.commits", "GET", "repos/{repo}/commits", Read;
        "repos.create", "POST", "user/repos", Consequential;
        "repos.fork", "POST", "repos/{repo}/forks", Consequential;
        "repos.update", "PATCH", "repos/{repo}", Consequential;
        "repos.branches", "GET", "repos/{repo}/branches", Read;
        "repos.tags", "GET", "repos/{repo}/tags", Read;
    ];
    "issues" => [
        "issues.list", "GET", "repos/{repo}/issues", Read;
        "issues.search", "GET", "search/issues", Read;
        "issues.get", "GET", "repos/{repo}/issues/{number}", Read;
        "issues.create", "POST", "repos/{repo}/issues", Write;
        "issues.update", "PATCH", "repos/{repo}/issues/{number}", Write;
        "issues.comments", "GET", "repos/{repo}/issues/{number}/comments", Read;
        "issues.comment", "POST", "repos/{repo}/issues/{number}/comments", Write;
        "issues.labels", "GET", "repos/{repo}/labels", Read;
        "issues.issue_labels", "GET", "repos/{repo}/issues/{number}/labels", Read;
        "issues.set_labels", "PUT", "repos/{repo}/issues/{number}/labels", Write;
        "issues.assignees", "POST", "repos/{repo}/issues/{number}/assignees", Write;
        "issues.milestones", "GET", "repos/{repo}/milestones", Read;
        "issues.create_milestone", "POST", "repos/{repo}/milestones", Write;
    ];
    "pull_requests" => [
        "pulls.list", "GET", "repos/{repo}/pulls", Read;
        "pulls.get", "GET", "repos/{repo}/pulls/{number}", Read;
        "pulls.create", "POST", "repos/{repo}/pulls", Write;
        "pulls.update", "PATCH", "repos/{repo}/pulls/{number}", Write;
        "pulls.files", "GET", "repos/{repo}/pulls/{number}/files", Read;
        "pulls.reviews", "GET", "repos/{repo}/pulls/{number}/reviews", Read;
        "pulls.review", "POST", "repos/{repo}/pulls/{number}/reviews", Write;
        "pulls.reviewers", "POST", "repos/{repo}/pulls/{number}/requested_reviewers", Write;
        "pulls.checks", "GET", "repos/{repo}/commits/{ref}/check-runs", Read;
        "pulls.merge", "PUT", "repos/{repo}/pulls/{number}/merge", Consequential;
    ];
    "actions" => [
        "actions.workflow", "GET", "repos/{repo}/actions/workflows/{workflow_id}", Read;
        "actions.workflow_runs", "GET", "repos/{repo}/actions/workflows/{workflow_id}/runs", Read;
        "actions.job", "GET", "repos/{repo}/actions/jobs/{job_id}", Read;
        "actions.workflows", "GET", "repos/{repo}/actions/workflows", Read;
        "actions.runs", "GET", "repos/{repo}/actions/runs", Read;
        "actions.run", "GET", "repos/{repo}/actions/runs/{run_id}", Read;
        "actions.jobs", "GET", "repos/{repo}/actions/runs/{run_id}/jobs", Read;
        "actions.artifacts", "GET", "repos/{repo}/actions/runs/{run_id}/artifacts", Read;
        "actions.download_artifact", "GET", "repos/{repo}/actions/artifacts/{artifact_id}/zip", Read;
        "actions.download_logs", "GET", "repos/{repo}/actions/runs/{run_id}/logs", Read;
        "actions.download_job_log", "GET", "repos/{repo}/actions/jobs/{job_id}/logs", Read;
        "actions.dispatch", "POST", "repos/{repo}/actions/workflows/{workflow_id}/dispatches", Consequential;
        "actions.rerun", "POST", "repos/{repo}/actions/runs/{run_id}/rerun", Consequential;
        "actions.cancel", "POST", "repos/{repo}/actions/runs/{run_id}/cancel", Consequential;
    ];
    "releases" => [
        "releases.list", "GET", "repos/{repo}/releases", Read;
        "releases.get", "GET", "repos/{repo}/releases/{release_id}", Read;
        "releases.create", "POST", "repos/{repo}/releases", Consequential;
        "releases.update", "PATCH", "repos/{repo}/releases/{release_id}", Consequential;
        "releases.delete", "DELETE", "repos/{repo}/releases/{release_id}", Consequential;
        "releases.assets", "GET", "repos/{repo}/releases/{release_id}/assets", Read;
        "releases.upload_asset", "POST", "repos/{repo}/releases/{release_id}/assets", Consequential;
        "releases.download_asset", "GET", "repos/{repo}/releases/assets/{asset_id}", Read;
        "downloads.read", "LOCAL", "native/download", Read;
        "tags.create", "POST", "repos/{repo}/git/refs", Write;
    ];
    "notifications" => [
        "notifications.repository_list", "GET", "repos/{repo}/notifications", Read;
        "notifications.mark_repository", "PUT", "repos/{repo}/notifications", Consequential;
        "notifications.list", "GET", "notifications", Read;
        "notifications.read", "PATCH", "notifications/threads/{thread_id}", Write;
        "notifications.mark_all", "PUT", "notifications", Consequential;
    ];
    "discussions_projects" => [
        "discussions.list", "POST", "graphql", Read;
        "discussions.categories", "POST", "graphql", Read;
        "discussions.get", "POST", "graphql", Read;
        "discussions.replies", "POST", "graphql", Read;
        "discussions.create", "POST", "graphql", Write;
        "discussions.comment", "POST", "graphql", Write;
        "projects.list", "POST", "graphql", Read;
        "projects.user_list", "POST", "graphql", Read;
        "projects.items", "POST", "graphql", Read;
        "projects.get", "POST", "graphql", Read;
        "projects.fields", "POST", "graphql", Read;
        "projects.item_values", "POST", "graphql", Read;
        "projects.field_users", "POST", "graphql", Read;
        "projects.field_labels", "POST", "graphql", Read;
        "projects.field_pull_requests", "POST", "graphql", Read;
        "projects.field_reviewers", "POST", "graphql", Read;

        "projects.add_item", "POST", "graphql", Write;
        "projects.update_field", "POST", "graphql", Write;
        "discussions.update", "POST", "graphql", Write;
        "discussions.close", "POST", "graphql", Write;
        "discussions.reopen", "POST", "graphql", Write;
        "discussions.edit_comment", "POST", "graphql", Write;
        "discussions.answer", "POST", "graphql", Write;
        "discussions.unanswer", "POST", "graphql", Write;
        "projects.update", "POST", "graphql", Consequential;
        "projects.clear_field", "POST", "graphql", Write;
        "projects.add_draft", "POST", "graphql", Write;
        "projects.archive", "POST", "graphql", Write;
        "projects.unarchive", "POST", "graphql", Write;

    ];
    "advanced" => [
        "rate_limit.get", "GET", "rate_limit", Read;
        "api.rest", "CUSTOM", "", Consequential;
        "api.graphql", "POST", "graphql", Consequential;
    ];
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation: String,
    #[serde(default)] pub repo: Option<String>,
    #[serde(default)] pub parameters: BTreeMap<String, Value>,
    #[serde(default)] pub body: Value,
}
#[derive(Clone, Debug, Serialize)]
pub struct Operation {
    pub(crate) id: String,
    pub(crate) method: String,
    pub(crate) endpoint: String,
    pub(crate) body: Option<Value>,
    pub(crate) risk: Risk,
    pub(crate) repo: Option<String>,
}
type Result<T> = std::result::Result<T, String>;
fn invalid() -> String { "Invalid GitHub operation arguments. Review the operation schema.".into() }
fn component(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && v != "." && v != ".." && !v.starts_with('-')
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn encoded(v: &str) -> String {
    v.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
fn string(v: &Value) -> Result<String> {
    let s = match v { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), Value::Bool(b) => b.to_string(), _ => return Err(invalid()) };
    if s.len() > 4096 || s.chars().any(char::is_control) { return Err(invalid()); }
    Ok(s)
}
/// Arbitrary URLs, CLI interpolation placeholders, path normalization and
/// credential-management endpoints are not admitted through the workbench.
pub fn endpoint(value: &str) -> Result<String> {
    if value.is_empty() || value.len() > 4096 || value.starts_with('/') || value.contains([':', '\\', '#', '%', '{', '}'])
        || value.chars().any(|c| c.is_control() || c.is_whitespace()) { return Err(invalid()); }
    let path = value.split('?').next().unwrap_or("");
    if path.split('/').any(|s| !component(s)) { return Err(invalid()); }
    if !["issues", "repos", "repositories", "orgs", "user", "users", "search", "notifications", "teams", "enterprises", "rate_limit", "meta", "versions", "emojis", "licenses", "gists"].contains(&path.split('/').next().unwrap_or("")) { return Err(invalid()); }
    if super::schemas::private_endpoint(path) {
        return Err("Credential/secret APIs require a private channel and are not enabled in this workbench.".into());
    }
    Ok(value.into())
}

impl Request {
    pub fn compile(&self) -> Result<Operation> {
        let mut budget = 128 * 1024;
        bounded(&self.body, 0, &mut budget)?;
        for (key, value) in &self.parameters { budget = budget.checked_sub(key.len()+1).ok_or_else(invalid)?; bounded(value,0,&mut budget)?; }
        if self.operation.len() > 128 || self.repo.as_ref().is_some_and(|r|r.len()>256) || serde_json::to_vec(self).map_err(|_|invalid())?.len()>128*1024 { return Err(invalid()); }
        if self.parameters.len() > 32 { return Err(invalid()); }
        if let Some(operation)=super::secrets::compile(self)? {return Ok(operation);}
        if let Some(operation)=super::uploads::compile(self)? {return Ok(operation);}
        if let Some(operation)=super::downloads::local(self)? {return Ok(operation);}
        let spec = SPECS.iter().find(|s|s.id == self.operation).ok_or_else(invalid)?;
        if self.operation.starts_with("notifications.")&&!spec.path.starts_with("repos/")&&self.repo.is_some(){return Err("Account notification targets require omitting repo and reviewing their exact thread or account scope.".into());}
        if super::downloads::remote(&self.operation) && (!self.body.is_null() || self.parameters.len()!=1 || !self.parameters.values().all(|v|v.as_u64().is_some_and(|n|n>0&&n<=9_007_199_254_740_991))) {return Err(invalid());}
        if spec.id=="issues.labels" && (self.parameters.contains_key("issue_number")||self.parameters.contains_key("number")){return Err("issues.labels lists all repository labels and takes no issue number. Use issues.issue_labels with parameters.number for labels on one issue.".into());}
        let mut args = self.parameters.clone();
        let mut repo = self.repo.as_ref().map(|r| {
            let parts: Vec<_> = r.split('/').collect();
            if parts.len() != 2 || parts.iter().any(|s|!component(s)) { Err(String::from("Invalid repo: expected owner/name with exactly two valid GitHub name components.")) } else { Ok(r.clone()) }
        }).transpose()?;
        let mut path = spec.path.to_string();
        if path.contains("{repo}") { path = path.replace("{repo}", repo.as_deref().ok_or("Missing top-level repo. Use repo=\"owner/name\"; put other path/query fields in parameters.")?); }
        while let Some(start) = path.find('{') {
            let end = path[start..].find('}').ok_or_else(invalid)? + start;
            let key = &path[start+1..end];
            let raw = string(&args.remove(key).ok_or_else(||format!("Missing parameters.{key}. Inspect operation=schema with this catalog ID."))?)?;
            // Slashes are only allowed in contents paths and refs, and are
            // encoded per segment; traversal is never normalized implicitly.
            if key == "path" || key == "ref" {
                if !(key=="path" && raw.is_empty()) && raw.split('/').any(|s|s.is_empty() || s == "." || s == "..") { return Err(invalid()); }
            } else if !component(&raw) { return Err(invalid()); }
            let value = if key == "path" { raw.split('/').map(encoded).collect::<Vec<_>>().join("/") } else { encoded(&raw) };
            path.replace_range(start..=end, &value);
        }
        let mut method = spec.method.to_string();
        let mut body = (!self.body.is_null()).then(||self.body.clone());
        if let Some(v) = &body { if !v.is_object() { return Err(invalid()); } deny_credentials(v)?; }
        if spec.id == "api.rest" {
            method = string(&args.remove("method").ok_or_else(invalid)?)?;
            if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) { return Err(invalid()); }
            path = endpoint(&string(&args.remove("endpoint").ok_or_else(invalid)?)?)?;
            if path.contains('?') { return Err("Pass query values as parameters, not in the endpoint.".into()); }
            if let Some(repo) = &repo { if path != format!("repos/{repo}") && !path.starts_with(&format!("repos/{repo}/")) { return Err("The advanced endpoint does not match the selected repository.".into()); } }
        }
        if path == "graphql" {
            if super::graph_reads::repository_target(spec.id) {
                let owner = args.get("owner").and_then(Value::as_str).filter(|s|component(s)).ok_or_else(invalid)?;
                let name = args.get("name").and_then(Value::as_str).filter(|s|component(s)).ok_or_else(invalid)?;
                let target = format!("{owner}/{name}");
                if repo.as_ref().is_some_and(|r|!r.eq_ignore_ascii_case(&target)) {
                    return Err("Discussion variables do not match the selected repository.".into());
                }
                repo = Some(target);
            } else if repo.is_some() {
                return Err("This GraphQL operation uses account-wide or explicit node-ID targets. Omit repo and review its exact variables; a selected repository does not constrain the graph.".into());
            }
            body = Some(graphql(spec.id, &args, body)?);
            args.clear();
        } else {
            if method == "GET" && body.is_some() { return Err("GET operations do not accept a request body.".into()); }
            if !args.is_empty() {
                let mut pairs = Vec::new();
                for (key, value) in args {
                    if !component(&key) || super::credential_fields::sensitive(&key) { return Err(invalid()); }
                    if spec.id != "api.rest" && !super::schemas::rest_row(spec).is_some_and(|r|r["parameters"].as_array().unwrap().iter().any(|p|p["in"]=="query"&&p["name"]==key)) { return Err("Unsupported query parameter. Inspect operation=schema with this catalog ID for accepted parameters.".into()); }
                    if key == "page" && !value.as_u64().is_some_and(|n| (1..=1000).contains(&n)) { return Err("parameters.page must be an integer from 1 to 1000.".into()); }
                    if key == "per_page" && !value.as_u64().is_some_and(|n| (1..=100).contains(&n)) { return Err("parameters.per_page must be an integer from 1 to 100.".into()); }
                    let raw=if let Some(values)=value.as_array() {
                        if values.len()>100 {return Err("Query arrays accept at most 100 scalar values.".into());}
                        let values=values.iter().map(string).collect::<Result<Vec<_>>>()?;
                        if values.iter().any(|v|v.contains(',')){return Err("Query array elements cannot contain commas; use the endpoint scalar form if supported.".into());}
                        values.join(",")
                    }else{string(&value)?};
                    pairs.push(format!("{}={}", encoded(&key), encoded(&raw)));
                }
                path.push('?'); path.push_str(&pairs.join("&"));
            }
        }
        if spec.id=="actions.dispatch" {
            let fields=body.as_ref().ok_or_else(invalid)?;
            if !fields["ref"].as_str().is_some_and(|r|!r.trim().is_empty()&&r.len()<=255&&!r.chars().any(char::is_control)) {return Err("Workflow dispatch requires a branch or tag ref.".into());}
            if let Some(inputs)=fields.get("inputs") {if !inputs.as_object().is_some_and(|m|m.len()<=25&&m.values().all(|v|v.is_string()||v.is_boolean()||v.is_number())){return Err("Workflow dispatch accepts at most 25 scalar inputs.".into());}}
        }
        if spec.id == "pulls.merge" && !body.as_ref().and_then(|b|b["sha"].as_str()).is_some_and(|s|s.len()==40 && s.bytes().all(|b|b.is_ascii_hexdigit())) { return Err("Merge requires the exact reviewed 40-character head SHA.".into()); }
        if spec.id == "pulls.review" && !body.as_ref().and_then(|b|b["commit_id"].as_str()).is_some_and(|s|s.len()==40 && s.bytes().all(|b|b.is_ascii_hexdigit())) { return Err("Review requires the exact reviewed commit_id.".into()); }
        Ok(Operation { id: spec.id.into(), method, endpoint: path, body, risk: spec.risk, repo })
    }
}
fn deny_credentials(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => for (key, value) in map {
            if super::credential_fields::sensitive(key) { return Err("Do not put credentials in GitHub operation arguments.".into()); }
            deny_credentials(value)?;
        },
        Value::Array(items) => for item in items { deny_credentials(item)?; },
        _ => {},
    }
    Ok(())
}
fn bounded(value: &Value, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 16 { return Err(invalid()); }
    *budget = budget.checked_sub(1).ok_or_else(invalid)?;
    match value {
        Value::String(s) => *budget = budget.checked_sub(s.len()).ok_or_else(invalid)?,
        Value::Object(map) => for (key, value) in map { *budget = budget.checked_sub(key.len()).ok_or_else(invalid)?; bounded(value,depth+1,budget)?; },
        Value::Array(items) => for value in items { bounded(value,depth+1,budget)?; },
        _ => {},
    }
    Ok(())
}
fn graphql(id: &str, args: &BTreeMap<String, Value>, body: Option<Value>) -> Result<Value> {
    if id == "api.graphql" {
        if !args.is_empty() { return Err("api.graphql accepts query and variables inside body, not parameters.".into()); }
        let value = body.ok_or("api.graphql requires body.query (non-empty string); optional body.variables and body.operationName. Omit repo and parameters.")?;
        if value.as_object().unwrap().keys().any(|k| !["query", "variables", "operationName"].contains(&k.as_str())) { return Err(invalid()); }
        let query = value["query"].as_str().filter(|s| !s.trim().is_empty() && s.len() <= 32 * 1024).ok_or_else(invalid)?;
        // All advanced GraphQL is consequential: no regex-based mutation
        // detector can turn server-supplied query text into read authority.
        if query.contains('\0') { return Err(invalid()); }
        super::credential_fields::graph(query)?;
        if !value["variables"].is_null() && !value["variables"].is_object() { return Err("body.variables must be an object.".into()); }
        if !value["operationName"].is_null() && !value["operationName"].is_string() { return Err("body.operationName must be a string.".into()); }
        return Ok(value);
    }
    if body.is_some() { return Err(invalid()); }
    super::schemas::validate_value(&graphql_schema(id).ok_or_else(invalid)?["parameters"], &json!(args), 0).map_err(|_|invalid())?;
    if let Some(query)=super::graph_mutations::query(id){return Ok(json!({"query":query,"variables":args}));}
    if let Some(query)=super::graph_reads::query(id){return Ok(json!({"query":query,"variables":args}));}
    let (query, required, optional): (&str, &[&str], &[&str]) = match id {
        "discussions.list" => ("query($owner:String!,$name:String!,$after:String){repository(owner:$owner,name:$name){id discussions(first:30,after:$after){nodes{id number title body url} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}", &["owner","name"], &["after"]),
        "discussions.create" => ("mutation($input:CreateDiscussionInput!){createDiscussion(input:$input){discussion{id number title url}}}", &["input"], &[]),
        "discussions.comment" => ("mutation($input:AddDiscussionCommentInput!){addDiscussionComment(input:$input){comment{id body url}}}", &["input"], &[]),
        "projects.list" => ("query($login:String!,$after:String){organization(login:$login){projectsV2(first:30,after:$after){nodes{id number title url} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}", &["login"], &["after"]),
        "projects.user_list" => ("query($login:String!,$after:String){user(login:$login){projectsV2(first:30,after:$after){nodes{id number title url} pageInfo{hasNextPage endCursor}}} rateLimit{cost remaining resetAt}}", &["login"], &["after"]),
        "projects.items" => ("query($id:ID!,$after:String){node(id:$id){... on ProjectV2{id title items(first:30,after:$after){nodes{id content{... on Issue{title url} ... on PullRequest{title url} ... on DraftIssue{title body}}} pageInfo{hasNextPage endCursor}}}} rateLimit{cost remaining resetAt}}", &["id"], &["after"]),
        "projects.add_item" => ("mutation($input:AddProjectV2ItemByIdInput!){addProjectV2ItemById(input:$input){item{id}}}", &["input"], &[]),
        "projects.update_field" => ("mutation($input:UpdateProjectV2ItemFieldValueInput!){updateProjectV2ItemFieldValue(input:$input){projectV2Item{id}}}", &["input"], &[]),
        _ => return Err(invalid()),
    };
    if args.keys().any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str())) || required.iter().any(|k|!args.contains_key(*k)) { return Err(invalid()); }
    for (key, value) in args { if key == "input" { deny_credentials(value)?; } }
    Ok(json!({"query":query,"variables":args}))
}

/// Small, version-reviewed schemas for the typed templates, not an assertion
/// that arbitrary GraphQL has been fully validated. Node membership and account
/// permissions remain GitHub's responsibility; the preview shows exact IDs.
pub fn graphql_schema(id: &str) -> Option<Value> {
    if let Some(schema)=super::graph_mutations::schema(id){return Some(schema);}
    if let Some(schema)=super::graph_reads::schema(id){return Some(schema);}
    let text = json!({"type":"string","maxLength":64000});
    let node = json!({"type":"string","minLength":1,"maxLength":1024,"pattern":"^[A-Za-z0-9_+=/-]+$"});
    let name = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.-]+$"});
    let after = json!({"type":"string","nullable":true,"maxLength":4096});
    let (required, properties) = match id {
        "discussions.list" => (json!(["owner","name"]),json!({"owner":name,"name":name,"after":after})),
        "projects.list" | "projects.user_list" => (json!(["login"]),json!({"login":name,"after":after})),
        "projects.items" => (json!(["id"]),json!({"id":node,"after":after})),
        _ => {
            let (required, mut fields) = match id {
                "discussions.create" => (json!(["repositoryId","categoryId","title","body"]),json!({"repositoryId":node,"categoryId":node,"title":text,"body":text})),
                "discussions.comment" => (json!(["discussionId","body"]),json!({"discussionId":node,"body":text,"replyToId":{"anyOf":[node,{"type":"null"}]}})),
                "projects.add_item" => (json!(["projectId","contentId"]),json!({"projectId":node,"contentId":node})),
                "projects.update_field" => (json!(["projectId","itemId","fieldId","value"]),json!({"projectId":node,"itemId":node,"fieldId":node,"value":{"type":"object","minProperties":1,"maxProperties":1,"additionalProperties":false,"properties":{"text":text,"number":{"type":"number"},"date":{"type":"string","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"},"singleSelectOptionId":node,"iterationId":node,"multiSelectOptionIds":{"type":"array","maxItems":100,"items":node}}}})),
                _ => return None,
            };
            fields["clientMutationId"] = json!({"type":"string","nullable":true,"maxLength":1024});
            (json!(["input"]),json!({"input":{"type":"object","required":required,"additionalProperties":false,"properties":fields}}))
        }
    };
    Some(json!({"id":id,"reviewed":"2026-09-10","documentation":if id.starts_with("discussions."){"https://docs.github.com/en/graphql/reference/discussions"}else{"https://docs.github.com/en/graphql/reference/projects"},
        "note":"Typed variables are validated locally. Explicit node IDs are account-wide targets, not proof of membership in the selected repository. GitHub validates node membership, field semantics and permissions.",
        "parameters":{"type":"object","required":required,"additionalProperties":false,"properties":properties}}))
}

pub fn catalog() -> Value {
    json!({"host":"github.com", "rest_version":super::REST_VERSION, "operations":SPECS,
        "schema_discovery":{"operation":"schema","parameters":{"id":"issues.create"},"search":{"operation":"schema","parameters":{"query":"issues create","page":0}},"note":"Omit id and pass query/page to search pinned official API operations. Schema discovery is local and does not access your account."},
        "limits":{"request_bytes":131072,"response_bytes":4194304,"page_size":100,"graphql_page_size":30,"download_bytes":super::downloads::MAX_BYTES,"download_page_bytes":super::downloads::PAGE_BYTES,"retained_download_bytes":134217728,"retained_download_count":32},
        "unsupported":["enterprise_hosts","credential_token_management","arbitrary_redirects","automatic_write_retry"],
        "calling_convention":{"operation":"Use a catalog ID; slash catalog aliases are accepted with the SAME meaning (issues/list = issues.list, repository issues). Upstream-only IDs are discovered as rest:<id> and executed via api.rest.","repo":"Top-level owner/name string for repository REST operations. Schema parameters use AG1 field names, such as number. Paired parameters.owner/repo and upstream issue_number/pull_number aliases are also accepted if consistent.","parameters":"Object containing path/query fields. For contents root omit path or use empty string.","body":"JSON object required by create/update operations; inspect schema.body and bodyRequired. GraphQL uses body.query, optional body.variables and body.operationName; omit repo."},
        "examples":[{"operation":"repos.get","repo":"octocat/Hello-World"},{"operation":"repos.contents","repo":"octocat/Hello-World","parameters":{"path":""}},{"operation":"issues.list","repo":"octocat/Hello-World","parameters":{"per_page":10,"page":1}},{"operation":"issues.create","repo":"owner/name","body":{"title":"Issue title"}},{"operation":"api.graphql","body":{"query":"query { viewer { login } }"}}],
        "pagination":"Inspect each schema for supported page/per_page or after fields. REST pages start at 1; schema-search pages at 0. Keep per_page small for exploration; preserve filters when advancing using reply.next. GraphQL connections return pageInfo/endCursor.",
        "rate_limits":"rate_resource identifies the independent GitHub bucket (core, search, graphql); remaining/reset values from different buckets are not comparable. When remaining is zero, wait until rate_reset; honor error retry times. Schema/catalog discovery and artifact reads are local and consume no GitHub quota. Advanced GraphQL can select multiple fields in one bounded query. No automatic write replay.",
        "large_results":"Full oversized replies are retained as local artifacts. Use the returned artifact readCall/cursor with the artifact tool to recover exact content; this makes no GitHub request. Narrow filters and per_page to reduce context size.",
        "git_guide":{"local":"git.status/diff/branches inspect granted working files; stage/unstage/commit/branch_create change that local repository. Use GitHub REST for hosted files, issues, PRs and Actions.","network":"git.fetch/push/clone/fetch_pr/pull transfer Git data with a separately reviewed GitHub account. Fetch leaves working files unchanged; checkout/pull change working files and require exact reviewed commits.","details":"See localGit for exact operation arguments and limits; use the dedicated Git flow for working-copy changes."},
        "notes":"Operations expose API access, not proof of account permission. Native confirmation is required unless the user enabled Bypass approvals for this chat; advanced operations remain consequential. Binary assets and local Git use separate native capabilities."})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn catalog_has_unique_operations_and_each_remote_matrix_area_compiles() {
        let mut names=std::collections::BTreeSet::new();
        for spec in SPECS {
            assert!(names.insert(spec.id));
            let mut parameters=BTreeMap::new();
            let mut path=spec.path;
            while let Some(start)=path.find('{') { let end=start+path[start..].find('}').unwrap();let key=&path[start+1..end];if key!="repo" { parameters.insert(key.into(),json!(if key=="path" {"dir/file.txt"} else {"123"})); }path=&path[end+1..]; }
            let mut body=if spec.method=="GET" { Value::Null } else { json!({}) };
            match spec.id {
                id if id.starts_with("secrets.")=>{parameters.insert("target".into(),json!("actions_organization"));parameters.insert("organization".into(),json!("owned"));if id!="secrets.list"{parameters.insert("name".into(),json!("OWNED_SECRET"));}if id=="secrets.set"{parameters.insert("visibility".into(),json!("private"));}if id=="secrets.set_repositories"{parameters.insert("selected_repository_ids".into(),json!([1]));}body=Value::Null;},
                id if super::super::downloads::remote(id)=>{for v in parameters.values_mut(){*v=json!(123);}body=Value::Null;},
                "releases.upload_asset"=>{parameters.insert("release_id".into(),json!(123));parameters.insert("path".into(),json!("owned.bin"));parameters.insert("name".into(),json!("owned.bin"));body=Value::Null;},
                "downloads.read"=>{parameters.insert("id".into(),json!("download-owned"));parameters.insert("offset".into(),json!(0));body=Value::Null;},
                "actions.dispatch"=>body=json!({"ref":"main"}),
                "pulls.merge"=>body=json!({"sha":"a".repeat(40)}), "pulls.review"=>body=json!({"commit_id":"a".repeat(40)}),
                "api.rest"=>{parameters.insert("method".into(),json!("GET"));parameters.insert("endpoint".into(),json!("repos/owned/fixture"));body=Value::Null;},
                "api.graphql"=>body=json!({"query":"query { viewer { login } }"}),
                "discussions.list"|"discussions.categories"|"discussions.get"=>{parameters.insert("owner".into(),json!("owned"));parameters.insert("name".into(),json!("fixture"));if spec.id=="discussions.get"{parameters.insert("number".into(),json!(7));}body=Value::Null;},
                "projects.list"|"projects.user_list"=>{parameters.insert("login".into(),json!("owned"));body=Value::Null;},
                id if super::super::graph_reads::field_type(id).is_some()=>{parameters.insert("id".into(),json!("I_owned"));parameters.insert("fieldId".into(),json!("F_owned"));parameters.insert("fieldName".into(),json!("Owned field"));body=Value::Null;},
                "projects.items"|"projects.get"|"projects.fields"|"projects.item_values"|"discussions.replies"=>{parameters.insert("id".into(),json!("node"));body=Value::Null;},
                _ if spec.path=="graphql"=>{
                    let input=match spec.id {
                        "discussions.create"=>json!({"repositoryId":"R_owned","categoryId":"C_owned","title":"Fixture","body":"Owned"}),
                        "discussions.comment"=>json!({"discussionId":"D_owned","body":"Owned"}),
                        "projects.add_item"=>json!({"projectId":"P_owned","contentId":"I_owned"}),
                        "projects.update_field"=>json!({"projectId":"P_owned","itemId":"I_owned","fieldId":"F_owned","value":{"text":"Owned"}}),
                        "discussions.update"=>json!({"discussionId":"D_owned","title":"Owned"}),
                        "discussions.close"|"discussions.reopen"=>json!({"discussionId":"D_owned"}),
                        "discussions.edit_comment"=>json!({"commentId":"C_owned","body":"Owned"}),
                        "discussions.answer"|"discussions.unanswer"=>json!({"id":"C_owned"}),
                        "projects.update"=>json!({"projectId":"P_owned","title":"Owned"}),
                        "projects.add_draft"=>json!({"projectId":"P_owned","title":"Owned"}),
                        "projects.clear_field"=>json!({"projectId":"P_owned","itemId":"I_owned","fieldId":"F_owned"}),
                        "projects.archive"|"projects.unarchive"=>json!({"projectId":"P_owned","itemId":"I_owned"}),
                        _=>panic!("Missing GraphQL fixture {}",spec.id),
                    };
                    parameters.insert("input".into(),input);body=Value::Null;
                },
                _=>{},
            }
            let repo=if spec.id.starts_with("secrets.") || spec.id.starts_with("notifications.")&&!spec.path.starts_with("repos/") || spec.id=="downloads.read" || spec.path=="graphql" && !super::super::graph_reads::repository_target(spec.id) {None}else{Some("owned/fixture".into())};
            let compiled=Request{operation:spec.id.into(),repo,parameters,body}.compile().unwrap_or_else(|e|panic!("{}: {e}",spec.id));
            assert_eq!(compiled.risk,spec.risk);
            if spec.method!="GET" && spec.method!="LOCAL" && spec.path!="graphql" { assert_ne!(spec.risk,Risk::Read); }
        }
    }
    #[test] fn endpoint_arguments_and_query_values_cannot_escape_host_or_repository() {
        for path in ["https://evil.test/x","//evil.test/x","repos/a/../b","repos/a/b%2f..","repos/{owner}/{repo}","user/tokens","app/installations","repos/a/b/actions/secrets","repos/a/b\\evil"] { assert!(endpoint(path).is_err(),"{path}"); }
        let mut request:Request=serde_json::from_value(json!({"operation":"api.rest","repo":"owned/fixture","parameters":{"method":"GET","endpoint":"repos/other/fixture"}})).unwrap();assert!(request.compile().is_err());
        request.parameters.insert("endpoint".into(),json!("repos/owned/fixture"));request.parameters.insert("q".into(),json!("a&token=evil#fragment"));
        assert!(request.compile().unwrap().endpoint.ends_with("q=a%26token%3Devil%23fragment"));
        request.parameters.insert("per_page".into(),json!(101));assert!(request.compile().is_err());
    }
    #[test] fn stale_head_secrets_schema_bombs_and_advanced_semantics_fail_closed() {
        let mut r:Request=serde_json::from_value(json!({"operation":"pulls.merge","repo":"owned/fixture","parameters":{"number":1},"body":{}})).unwrap();assert!(r.compile().is_err());
        r.body=json!({"sha":"a".repeat(40),"token":"secret"});assert!(r.compile().is_err());
        r.body=json!({"sha":"a".repeat(40)});assert!(r.compile().is_ok());
        let r:Request=serde_json::from_value(json!({"operation":"api.graphql","body":{"query":"#query\nmutation { destructive }"}})).unwrap();assert_eq!(r.compile().unwrap().risk,Risk::Consequential);
        let mut r=r;for _ in 0..18 {r.body=json!({"nested":r.body});}assert!(r.compile().is_err());
        assert!(serde_json::from_value::<Request>(json!({"operation":"repos.list","host":"evil.test"})).is_err());
    }
    #[test] fn graphql_target_labels_cannot_claim_unrelated_repository_authority() {
        let mut r:Request=serde_json::from_value(json!({"operation":"discussions.list","repo":"owned/fixture","parameters":{"owner":"other","name":"fixture"}})).unwrap();
        assert!(r.compile().is_err());
        r.parameters.insert("owner".into(),json!("owned"));
        assert_eq!(r.compile().unwrap().repo.as_deref(),Some("owned/fixture"));
        r.repo=None;
        assert_eq!(r.compile().unwrap().repo.as_deref(),Some("owned/fixture"));
        for (operation,parameters,body) in [
            ("projects.items",json!({"id":"P_owned"}),Value::Null),
            ("discussions.comment",json!({"input":{"discussionId":"D_owned","body":"Owned"}}),Value::Null),
            ("api.graphql",json!({}),json!({"query":"query { viewer { login } }"})),
        ] {
            let mut r=Request{operation:operation.into(),repo:Some("owned/fixture".into()),parameters:serde_json::from_value(parameters).unwrap(),body};
            assert!(r.compile().is_err());r.repo=None;
            assert!(r.compile().unwrap().repo.is_none());
        }
    }
    #[test] fn typed_graphql_inputs_validate_required_fields_types_and_single_field_updates() {
        let compile=|operation:&str,parameters:Value|serde_json::from_value::<Request>(json!({"operation":operation,"parameters":parameters})).unwrap().compile();
        for parameters in [json!({"login":42}),json!({"login":"owned","after":false}),json!({"login":"owned","unknown":"field"})] {
            assert!(compile("projects.list",parameters).is_err());
        }
        assert!(compile("projects.list",json!({"login":"owned","after":null})).is_ok());
        assert!(compile("discussions.create",json!({"input":{"title":"Missing target"}})).is_err());
        assert!(compile("discussions.comment",json!({"input":{"discussionId":"D_owned","body":"Line one\nLine two","replyToId":null}})).is_ok());
        let mut params=json!({"input":{"projectId":"P_owned","itemId":"I_owned","fieldId":"F_owned","value":{"number":3.5}}});
        assert!(compile("projects.update_field",params.clone()).is_ok());
        for value in [json!({}),json!({"number":"wrong"}),json!({"text":"two","number":1}),json!({"unknown":"field"}),json!({"multiSelectOptionIds":[null]})] {
            params["input"]["value"]=value;assert!(compile("projects.update_field",params.clone()).is_err());
        }
        params["input"]["value"]=json!({"multiSelectOptionIds":["option-one","option-two"]});
        assert!(compile("projects.update_field",params).is_ok());
    }
}
