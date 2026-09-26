//! Everyday UI projections use the same typed, account-bound native reads.
use super::{
    operations::Request,
    service::{ApiError, Result, Service},
};
use crate::resource_fs::ScanControl;
use serde_json::{Value, json};
pub fn read(
    service: &Service,
    action: &str,
    args: &Value,
    scope: &str,
    control: &ScanControl<'_>,
) -> Result<Value> {
    if action == "pull_review" {
        return pull_review(service, args, scope, control);
    }
    let page = args
        .get("page")
        .map(|v| {
            v.as_u64()
                .filter(|p| (1..=100).contains(p))
                .ok_or_else(|| ApiError::new("arguments", "Select a page from 1 to 100."))
        })
        .transpose()?
        .unwrap_or(1);
    let repo = args.get("repo").cloned();
    let state = if matches!(action, "issues" | "pulls") {
        match args.get("state") {
            None => "open",
            Some(Value::String(s)) if ["open", "closed", "all"].contains(&s.as_str()) => s,
            _ => {
                return Err(ApiError::new(
                    "arguments",
                    "Select open, closed or all items.",
                ));
            }
        }
    } else {
        "open"
    };
    let request = |operation: &str, parameters: Value| -> Result<Request> {
        let mut value = json!({"operation":operation,"parameters":parameters});
        if operation != "repos.list" {
            value["repo"] = repo.clone().unwrap_or(Value::Null);
        }
        serde_json::from_value(value)
            .map_err(|_| ApiError::new("arguments", "Invalid GitHub browse request."))
    };
    let parameters = json!({"page":page,"per_page":30});
    let requests = match action {
        "repos" => vec![request(
            "repos.list",
            json!({"page":page,"per_page":30,"sort":"pushed"}),
        )?],
        "issues" => vec![request(
            "issues.list",
            json!({"page":page,"per_page":30,"state":state}),
        )?],
        "pulls" => vec![request(
            "pulls.list",
            json!({"page":page,"per_page":30,"state":state}),
        )?],
        "runs" => vec![request("actions.runs", parameters)?],
        "files" => vec![request(
            "pulls.files",
            json!({"page":page,"per_page":30,"number":args["number"]}),
        )?],
        "item" => vec![
            request(
                if args["pull"] == true {
                    "pulls.get"
                } else {
                    "issues.get"
                },
                json!({"number":args["number"]}),
            )?,
            request(
                "issues.comments",
                json!({"number":args["number"],"page":page,"per_page":30}),
            )?,
        ],
        _ => {
            return Err(ApiError::new(
                "arguments",
                "Unknown GitHub browse operation.",
            ));
        }
    };
    let mut replies = service.read(requests, scope, control)?.into_iter();
    let reply = replies
        .next()
        .ok_or_else(|| ApiError::new("response", "Missing GitHub response."))?;
    match action {
        "repos" => {
            let rows = reply
                .data
                .as_array()
                .ok_or_else(|| ApiError::new("response", "Invalid GitHub repository list."))?;
            Ok(
                json!({"items":rows.iter().map(|r|json!({"name":r["full_name"],"description":r["description"],"private":r["private"],"branch":r["default_branch"],"url":r["html_url"]})).collect::<Vec<_>>(),"has_more":reply.next.is_some()}),
            )
        }
        "item" => {
            let comments = replies
                .next()
                .ok_or_else(|| ApiError::new("response", "Missing GitHub comments."))?;
            if !reply.data.is_object()
                || reply.data["number"] != args["number"]
                || !reply.data["number"].as_u64().is_some_and(|n| n > 0)
                || !reply.data["title"].is_string()
                || !comments.data.as_array().is_some_and(|rows| {
                    rows.len() <= 30
                        && rows.iter().all(|r| {
                            r["id"].is_number()
                                && r["body"].is_string()
                                && r["user"]["login"].is_string()
                        })
                })
            {
                return Err(ApiError::new(
                    "response",
                    "Invalid GitHub item or comment page.",
                ));
            }
            Ok(
                json!({"item":reply.data,"comments":comments.data,"comments_page":page,"comments_next":comments.next.is_some()}),
            )
        }
        _ => Ok(reply.data),
    }
}

/// A bounded display page, not a mutation admission or an atomic server
/// snapshot. Both PR identities are rechecked around the files/checks reads;
/// all four requests use the service's same-account compound read contract.
fn pull_review(
    service: &Service,
    args: &Value,
    scope: &str,
    control: &ScanControl<'_>,
) -> Result<Value> {
    let invalid = || {
        ApiError::new(
            "arguments",
            "Select an exact pull request head/base and bounded review page.",
        )
    };
    if args.as_object().ok_or_else(invalid)?.keys().any(|k| {
        ![
            "repo",
            "number",
            "head_sha",
            "base_sha",
            "page",
            "check_page",
        ]
        .contains(&k.as_str())
    }) {
        return Err(invalid());
    }
    let repo = super::repo(args).map_err(|_| invalid())?;
    let number = args["number"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(invalid)?;
    let sha = |key: &str| {
        args[key]
            .as_str()
            .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(invalid)
    };
    let head = sha("head_sha")?;
    let base = sha("base_sha")?;
    let page = args
        .get("page")
        .map(|v| v.as_u64().ok_or_else(invalid))
        .transpose()?
        .unwrap_or(1);
    let check_page = args
        .get("check_page")
        .map(|v| v.as_u64().ok_or_else(invalid))
        .transpose()?
        .unwrap_or(1);
    if !(1..=100).contains(&page) || !(1..=100).contains(&check_page) {
        return Err(invalid());
    }
    let request = |operation: &str, parameters: Value| -> Result<Request> {
        serde_json::from_value(json!({"operation":operation,"repo":repo,"parameters":parameters}))
            .map_err(|_| invalid())
    };
    let replies = service.read(
        vec![
            request("pulls.get", json!({"number":number}))?,
            request(
                "pulls.files",
                json!({"number":number,"page":page,"per_page":30}),
            )?,
            request(
                "pulls.checks",
                json!({"ref":head,"page":check_page,"per_page":30}),
            )?,
            request("pulls.get", json!({"number":number}))?,
        ],
        scope,
        control,
    )?;
    if replies.len() != 4 {
        return Err(ApiError::new("response", "Incomplete GitHub review page."));
    }
    for item in [&replies[0].data, &replies[3].data] {
        if item["number"] != number || item["head"]["sha"] != head || item["base"]["sha"] != base {
            return Err(ApiError::new(
                "stale_head",
                "The pull request head or base changed. Refresh before reviewing another page or submitting a review.",
            ));
        }
    }
    let files = replies[1]
        .data
        .as_array()
        .filter(|rows| rows.len() <= 30)
        .ok_or_else(|| ApiError::new("response", "Invalid GitHub file page."))?;
    let checks = replies[2].data["check_runs"]
        .as_array()
        .filter(|rows| rows.len() <= 30)
        .ok_or_else(|| ApiError::new("response", "Invalid GitHub check page."))?;
    for file in files {
        if !file["filename"].is_string()
            || !file["patch"].is_null() && !file["patch"].is_string()
            || file["patch"]
                .as_str()
                .is_some_and(|s| s.len() > 1024 * 1024)
        {
            return Err(ApiError::new(
                "response_limit",
                "GitHub patch is malformed or exceeds the per-file display limit. Inspect the file on GitHub.",
            ));
        }
    }
    Ok(
        json!({"item":replies[3].data,"head_sha":head,"base_sha":base,"files":files,"checks":checks,"page":page,"check_page":check_page,"files_next":replies[1].next.is_some(),"checks_next":replies[2].next.is_some(),"limits":"30 files and 30 checks per page, up to 100 pages; 1 MiB per displayed patch. Missing patches may be binary, large or omitted by GitHub. The head/base are rechecked around each page, not an atomic server snapshot. This display does not approve a review or merge."}),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{
        operations::Operation,
        service::{Account, Outcome, Reply, Transport, check},
    };
    use super::*;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    struct Fixture {
        calls: Mutex<Vec<Operation>>,
        kind: &'static str,
        changed: AtomicBool,
    }
    impl Fixture {
        fn account_value(&self) -> Account {
            Account::bound(
                if self.changed.load(Ordering::Acquire) {
                    2
                } else {
                    1
                },
                "owned-fixture".into(),
                "owned-generation".into(),
            )
            .unwrap()
        }
    }
    impl Transport for Fixture {
        fn account(&self, c: &ScanControl<'_>) -> Result<Account> {
            check(c)?;
            Ok(self.account_value())
        }
        fn execute(&self, a: &Account, o: &Operation, c: &ScanControl<'_>) -> Result<Reply> {
            check(c)?;
            if *a != self.account_value() {
                return Err(ApiError::new("stale_account", "Account changed."));
            }
            let mut calls = self.calls.lock().unwrap();
            calls.push(o.clone());
            let call = calls.len();
            assert_eq!(o.risk, super::super::operations::Risk::Read);
            if self.kind == "cancel" && call == 2 {
                c.cancel.store(true, Ordering::Release);
                check(c)?;
            }
            if self.kind == "account" && call == 2 {
                self.changed.store(true, Ordering::Release);
            }
            let head = if self.kind == "head" && call == 4 {
                "c".repeat(40)
            } else {
                "a".repeat(40)
            };
            let base = if self.kind == "base" && call == 4 {
                "d".repeat(40)
            } else {
                "b".repeat(40)
            };
            let data = match o.id.as_str() {
                "issues.list" | "pulls.list" => json!([]),
                "pulls.get" | "issues.get" => {
                    json!({"number":7,"title":"Owned PR","head":{"sha":head},"base":{"sha":base}})
                }
                "issues.comments" => match self.kind {
                    "comments" => json!({"not":"an array"}),
                    "many_comments" => json!(
                        (0..31)
                            .map(|i| json!({"id":i,"body":"test","user":{"login":"owned"}}))
                            .collect::<Vec<_>>()
                    ),
                    "bad_comment" => json!([{"id":1,"body":{},"user":{"login":"owned"}}]),
                    _ => {
                        json!([{"id":1,"body":"Literal <script>comment</script>","user":{"login":"owned"}}])
                    }
                },
                "pulls.files" => {
                    if self.kind == "files" {
                        json!({"not":"a list"})
                    } else if self.kind == "large" {
                        json!([{"filename":"large","patch":"x".repeat(1024*1024+1)}])
                    } else {
                        json!([{"filename":"literal <script>.txt","patch":"@@ -1 +1 @@\n-old\n+new","additions":1,"deletions":1}])
                    }
                }
                "pulls.checks" => {
                    if self.kind == "checks" {
                        Value::Null
                    } else {
                        json!({"check_runs":[{"name":"owned check","status":"completed","conclusion":"success"}]})
                    }
                }
                _ => panic!("Unexpected review operation"),
            };
            Ok(Reply {
                data,
                status: 200,
                next: Some("next".into()),
                rate_remaining: Some(20),
                rate_reset: None, rate_resource: None, rate_limit: None,
                outcome: Outcome::Read,
            })
        }
    }
    #[test]
    fn item_state_filters_are_explicit_and_validate_before_traffic() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("github-connected"), b"owned-generation").unwrap();
        let transport = Arc::new(Fixture {
            calls: Mutex::new(vec![]),
            kind: "ok",
            changed: AtomicBool::new(false),
        });
        let service = Service::new(temp.path().into(), transport.clone());
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        for action in ["issues", "pulls"] {
            for state in ["open", "closed", "all"] {
                read(
                    &service,
                    action,
                    &json!({"repo":"owned/repo","state":state,"page":2}),
                    "scope",
                    &c,
                )
                .unwrap();
                assert!(
                    transport
                        .calls
                        .lock()
                        .unwrap()
                        .last()
                        .unwrap()
                        .endpoint
                        .ends_with(&format!("page=2&per_page=30&state={state}"))
                );
            }
            for state in [json!("merged"), json!(true), Value::Null] {
                let before = transport.calls.lock().unwrap().len();
                assert!(
                    read(
                        &service,
                        action,
                        &json!({"repo":"owned/repo","state":state}),
                        "scope",
                        &c
                    )
                    .is_err()
                );
                assert_eq!(transport.calls.lock().unwrap().len(), before);
            }
        }
    }
    #[test]
    fn item_comments_are_bounded_explicit_pages_without_mutation_authority() {
        for kind in ["ok", "cancel", "comments", "many_comments", "bad_comment"] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(temp.path().join("github-connected"), b"owned-generation").unwrap();
            let transport = Arc::new(Fixture {
                calls: Mutex::new(vec![]),
                kind,
                changed: AtomicBool::new(false),
            });
            let service = Service::new(temp.path().into(), transport.clone());
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            let args = json!({"repo":"owned/repo","number":7,"page":2});
            let result = read(&service, "item", &args, "owned-scope", &c);
            if kind == "ok" {
                let value = result.unwrap();
                assert_eq!(value["comments_page"], 2);
                assert_eq!(value["comments_next"], true);
                assert!(
                    value["comments"][0]["body"]
                        .as_str()
                        .unwrap()
                        .contains("<script>")
                );
            } else {
                assert!(result.is_err(), "{kind}");
            }
            assert!(
                transport.calls.lock().unwrap()[1]
                    .endpoint
                    .ends_with("comments?page=2&per_page=30")
            );
            cancel.store(false, Ordering::Release);
            for page in [json!(0), json!(101), json!("2"), Value::Null] {
                let mut args = args.clone();
                args["page"] = page;
                let before = transport.calls.lock().unwrap().len();
                assert!(read(&service, "item", &args, "owned-scope", &c).is_err());
                assert_eq!(transport.calls.lock().unwrap().len(), before);
            }
        }
    }
    #[test]
    fn review_pages_bind_head_base_account_and_independent_file_check_pages() {
        for kind in [
            "ok", "head", "base", "account", "cancel", "files", "checks", "large",
        ] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(temp.path().join("github-connected"), b"owned-generation").unwrap();
            let transport = Arc::new(Fixture {
                calls: Mutex::new(vec![]),
                kind,
                changed: AtomicBool::new(false),
            });
            let service = Service::new(temp.path().into(), transport.clone());
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            let args = json!({"repo":"owned/repo","number":7,"head_sha":"a".repeat(40),"base_sha":"b".repeat(40),"page":2,"check_page":3});
            let result = read(&service, "pull_review", &args, "owned-scope", &c);
            if kind == "ok" {
                let page = result.unwrap();
                assert_eq!(page["page"], 2);
                assert_eq!(page["check_page"], 3);
                assert_eq!(page["files_next"], true);
                assert_eq!(page["head_sha"], "a".repeat(40));
                assert_eq!(page["checks"][0]["conclusion"], "success");
            } else {
                assert!(result.is_err(), "{kind}");
            }
            let calls = transport.calls.lock().unwrap();
            assert!(calls[1].endpoint.contains("page=2"));
            if calls.len() > 2 {
                assert!(calls[2].endpoint.contains("page=3"));
                assert!(calls[2].endpoint.contains(&"a".repeat(40)));
            }
            drop(calls);
            cancel.store(false, Ordering::Release);
            for bad in [
                json!({"page":101}),
                json!({"page":"2"}),
                json!({"head_sha":"main"}),
                json!({"extra":"unreviewed"}),
            ] {
                let mut args = args.clone();
                for (k, v) in bad.as_object().unwrap() {
                    args[k] = v.clone();
                }
                let calls = transport.calls.lock().unwrap().len();
                assert!(read(&service, "pull_review", &args, "owned-scope", &c).is_err());
                assert_eq!(transport.calls.lock().unwrap().len(), calls);
            }
        }
    }
}
