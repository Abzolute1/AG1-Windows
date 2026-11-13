//! Owned workflow, release and paged inbox states; never paid CI or live writes.
use super::*;
use serde_json::Value;
pub(super) fn execute(f: &Fixture, op: &Value) -> Option<Result<Reply>> {
    let id = op["id"].as_str()?;
    let endpoint = op["endpoint"].as_str()?;
    let base = endpoint.split('?').next()?;
    let second = endpoint.contains("page=2");
    let mut changes = f.workflows.lock().unwrap();
    let run_id = if second { 56 } else { 55 };
    let job_id = if second { 67 } else { 66 };
    let run = |id: u64, state: &Value| json!({"id":id,"name":"Owned workflow","display_title":if id==55{"Owned workflow"}else{"Second owned run"},"workflow_id":44,"status":state["run_status"].as_str().unwrap_or("completed"),"conclusion":state["conclusion"].as_str().unwrap_or("success"),"head_branch":"main","head_sha":"a".repeat(40),"event":"workflow_dispatch","run_attempt":1,"actor":{"login":"owned-fixture"}});
    let release = |state: &Value| {
        let mut r = json!({"id":88,"name":"Owned release","tag_name":"v-owned","body":"Owned release notes","draft":true,"prerelease":false,"target_commitish":"main"});
        if let Some(fields) = state["release"].as_object() {
            for (k, v) in fields {
                r[k] = v.clone();
            }
        }
        r
    };
    let mut status = 200;
    let mut next = None;
    let data = match id {
        "actions.workflows" => {
            json!({"total_count":1,"workflows":[{"id":44,"name":"Owned workflow definition","path":".github/workflows/owned.yml","state":"active"}]})
        }
        "actions.workflow" => {
            json!({"id":44,"name":"Owned workflow definition","path":".github/workflows/owned.yml","state":"active"})
        }
        "actions.runs" | "actions.workflow_runs" => {
            if !second {
                next = Some(format!("{base}?page=2&per_page=30"));
            }
            json!({"total_count":2,"workflow_runs":[run(run_id,&changes)]})
        }
        "actions.run" => run(base.rsplit('/').next()?.parse().ok()?, &changes),
        "actions.jobs" => {
            if !second {
                next = Some(format!("{base}?page=2&per_page=30"));
            }
            json!({"total_count":2,"jobs":[{"id":job_id,"run_id":55,"name":if second{"Second owned job"}else{"Owned job"},"status":"completed","conclusion":"success"}]})
        }
        "actions.job" => {
            json!({"id":base.rsplit('/').next()?.parse::<u64>().ok()?,"run_id":55,"name":"Owned job","status":"completed","conclusion":"success","steps":[{"number":1,"name":"Owned setup <script>literal</script>","status":"completed","conclusion":"success"},{"number":2,"name":"Owned build step","status":"completed","conclusion":"success"}]})
        }
        "actions.artifacts" => {
            json!({"artifacts":[{"id":77,"name":"Owned artifact","size_in_bytes":300000,"expired":false}]})
        }
        "actions.dispatch" => {
            changes["run_status"] = json!("queued");
            json!({"workflow_run_id":55,"run_url":"https://api.github.com/repos/owned/fixture/actions/runs/55","html_url":"https://github.com/owned/fixture/actions/runs/55"})
        }
        "actions.rerun" => {
            changes["run_status"] = json!("queued");
            status = 201;
            Value::Null
        }
        "actions.cancel" => {
            changes["run_status"] = json!("completed");
            changes["conclusion"] = json!("cancelled");
            status = 202;
            Value::Null
        }
        "releases.list" => {
            let mut rows = vec![release(&changes)];
            if changes["new_release"].is_object() {
                rows.push(changes["new_release"].clone());
            }
            json!(rows)
        }
        "releases.get" => {
            if base.ends_with("/89") {
                changes["new_release"].clone()
            } else {
                release(&changes)
            }
        }
        "releases.create" => {
            let mut value = op["body"].clone();
            value["id"] = json!(89);
            changes["new_release"] = value.clone();
            status = 201;
            value
        }
        "releases.update" => {
            if base.ends_with("/89") {
                let mut value = op["body"].clone();
                value["id"] = json!(89);
                changes["new_release"] = value.clone();
                value
            } else {
                changes["release"] = op["body"].clone();
                release(&changes)
            }
        }
        "notifications.list" | "notifications.repository_list" => {
            let thread = if second { "71" } else { "70" };
            let unread = changes["read"][thread] != true;
            let include_read = endpoint.contains("all=true");
            if !second {
                next = Some(format!("{base}?page=2&per_page=30"));
            }
            if unread || include_read {
                json!([{"id":thread,"unread":unread,"reason":"mention","repository":{"full_name":"owned/fixture"},"subject":{"title":if second{"Owned second notification"}else{"Owned first notification"},"type":"Issue","url":"https://api.github.com/repos/owned/fixture/issues/7"}}])
            } else {
                json!([])
            }
        }
        "notifications.read" => {
            if !changes["read"].is_object() {
                changes["read"] = json!({});
            }
            changes["read"][base.rsplit('/').next()?] = json!(true);
            status = 205;
            Value::Null
        }
        "notifications.mark_all" | "notifications.mark_repository" => {
            changes["read"] = json!({"70":true,"71":true});
            status = 202;
            json!({"message":"Owned inbox change accepted"})
        }
        _ => return None,
    };
    Some(Ok(Reply {
        data,
        status,
        next,
        rate_remaining: Some(4999),
        rate_reset: None,
        rate_limit: None,
        rate_resource: None,
        outcome: if op["risk"] == "read" {
            Outcome::Read
        } else {
            Outcome::Applied
        },
    }))
}
