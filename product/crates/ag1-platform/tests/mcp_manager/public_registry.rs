use super::*;
use ag1_core::tools::registry::Scope;
use ag1_platform::mcp::{registry::Directory, service::Service};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
fn listing() -> Value {
    json!({"server":{"name":"org.example/owned","title":"Owned registry fixture","description":"Literal <script>metadata</script>","version":"1.2.3","remotes":[{"type":"streamable-http","url":"https://owned.invalid/mcp"}],"packages":[{"registryType":"pypi","identifier":"owned-package","version":"1.2.3","transport":{"type":"stdio"},"environmentVariables":[{"name":"OWNED_TOKEN","isSecret":true,"isRequired":true}]}],"_meta":{"publisher":{"readme":"omitted untrusted README"}}},"_meta":{"io.modelcontextprotocol.registry/official":{"status":"active","isLatest":true}}})
}
#[test]
fn public_discovery_cache_and_reviewed_setup_are_inert_and_version_bound() {
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager(&dir.path().join("state")));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let changed = Arc::new(AtomicBool::new(false));
    let seen = requests.clone();
    let mutate = changed.clone();
    let directory = Arc::new(Directory::with_reader(move |url| {
        assert_eq!(url.host_str(), Some("api.mcp.github.com"));
        seen.lock().unwrap().push(url.to_string());
        let mut row = listing();
        if mutate.load(Ordering::SeqCst) {
            row["server"]["remotes"][0]["url"] = json!("https://changed.invalid/mcp");
        }
        let body = if url.path().contains("/versions/") {
            row
        } else {
            json!({"servers":[row],"metadata":{}})
        };
        Ok((serde_json::to_vec(&body).unwrap(), 0))
    }));
    let service = Service::from_manager_with_directory(
        m.clone(),
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")).into(),
        directory,
    );
    let scope = Scope {
        workspace: "general".into(),
        session: "discovery".into(),
        epoch: 1,
        revision: 1,
    };
    let session = service
        .session(CatalogScope::GeneralChat, scope.clone(), None)
        .unwrap();
    session
        .dispatch(&scope, &json!({"action":"discover"}))
        .unwrap();
    assert!(requests.lock().unwrap().is_empty());
    let page = session
        .dispatch(
            &scope,
            &json!({"action":"discover","query":"owned","fetch":true}),
        )
        .unwrap();
    session
        .dispatch(
            &scope,
            &json!({"action":"discover","query":"owned","fetch":true}),
        )
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    let key = &page["page"]["entries"][0]["key"];
    let detail = session
        .dispatch(&scope, &json!({"action":"discover_detail","key":key}))
        .unwrap();
    assert_eq!(
        detail["server"]["description"],
        "Literal <script>metadata</script>"
    );
    assert!(detail["server"].get("_meta").is_none());
    assert!(m.records().unwrap().is_empty());
    assert_eq!(requests.lock().unwrap().len(), 1);
    let review = session
        .dispatch(
            &scope,
            &json!({"action":"discover_review","key":key,"sha256":detail["sha256"],"index":0}),
        )
        .unwrap();
    assert!(m.records().unwrap().is_empty());
    assert_eq!(requests.lock().unwrap().len(), 2);
    let saved = session
        .dispatch(
            &scope,
            &json!({"action":"discover_confirm","review":review["review"]}),
        )
        .unwrap();
    assert_eq!(saved["active"], false);
    assert!(
        session
            .dispatch(
                &scope,
                &json!({"action":"discover_confirm","review":review["review"]})
            )
            .is_err()
    );
    let record = m.records().unwrap().remove(0);
    assert!(!record.availability.enabled);
    assert!(matches!(
        record.draft.provenance,
        Provenance::Registry { .. }
    ));
    let local=session.dispatch(&scope,&json!({"action":"discover_local_review","key":key,"sha256":detail["sha256"],"index":0,"configuration":{"transport":"local","executable":"/does/not/exist","arguments":[],"cwd":dir.path(),"public_environment":{},"secret_environment":["FORGED_NAME"]}})).unwrap();
    assert_eq!(
        local["draft"]["configuration"]["secret_environment"],
        json!(["OWNED_TOKEN"])
    );
    session
        .dispatch(
            &scope,
            &json!({"action":"discover_confirm","review":local["review"]}),
        )
        .unwrap();
    changed.store(true, Ordering::SeqCst);
    assert!(
        session
            .dispatch(
                &scope,
                &json!({"action":"review","id":record.id,"revision":record.revision,"test":true})
            )
            .is_err()
    );
    assert!(
        session
            .dispatch(
                &scope,
                &json!({"action":"discover","source":"https://unreviewed.invalid"})
            )
            .is_err()
    );
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .all(|u| u.starts_with("https://api.mcp.github.com/"))
    );
    session.close().unwrap();
}
#[test]
fn discovery_rate_limit_retains_cache_and_rejects_hostile_shapes() {
    let rate = Arc::new(AtomicBool::new(false));
    let mode = rate.clone();
    let directory = Directory::with_reader(move |_| {
        if mode.load(Ordering::SeqCst) {
            Ok((Vec::new(), u64::MAX))
        } else {
            Ok((serde_json::to_vec(&json!({"servers":[listing(),{"server":{"name":"bad/entry","version":"1","description":{}}}],"metadata":{"nextCursor":"owned-next"}})).unwrap(),0))
        }
    });
    let first = directory.search("", "", true, false).unwrap();
    assert_eq!(first["page"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(first["page"]["excluded"], 1);
    rate.store(true, Ordering::SeqCst);
    let limited = directory.search("", "", true, true).unwrap();
    assert!(limited["error"].is_string());
    assert_eq!(limited["page"], first["page"]);
    let hostile = Directory::with_reader(|_| Ok((br#"{"servers":[],"servers":[]}"#.to_vec(), 0)));
    assert!(hostile.search("", "", true, false).unwrap()["error"].is_string());
    assert!(hostile.search(&"x".repeat(129), "", false, false).is_err());
}

#[test]
fn bulky_directory_pages_stay_below_bridge_limit_and_continue_past_thirty() {
    let directory = Directory::with_reader(|url| {
        let page = url
            .query_pairs()
            .find(|(k, _)| k == "cursor")
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let rows: Vec<_> = (0..10)
            .map(|i| {
                let mut row = listing();
                row["server"]["name"] = json!(format!("org/server-{}", page * 10 + i));
                row["server"]["description"] = json!("\u{1}".repeat(8192));
                row["server"]["packages"][0]["arguments"] =
                    json!(["public configuration".repeat(500)]);
                row
            })
            .collect();
        Ok((
            serde_json::to_vec(
                &json!({"servers":rows,"metadata":{"nextCursor":(page+1).to_string()}}),
            )
            .unwrap(),
            0,
        ))
    });
    let mut keys = std::collections::BTreeSet::new();
    for page in 0..4 {
        let response = directory
            .search(
                "",
                if page == 0 {
                    ""
                } else {
                    ["", "1", "2", "3"][page]
                },
                true,
                false,
            )
            .unwrap();
        assert!(response["error"].is_null());
        assert!(serde_json::to_vec(&response).unwrap().len() < 48000);
        for entry in response["page"]["entries"].as_array().unwrap() {
            keys.insert(entry["key"].as_str().unwrap().to_owned());
            assert!(entry["server"].get("packages").is_none());
            assert_eq!(entry["summary"]["credentials"], true);
        }
        let key = response["page"]["entries"][0]["key"].as_str().unwrap();
        let detail = directory.detail(key).unwrap();
        assert_eq!(
            detail["server"]["description"].as_str().unwrap().len(),
            8192
        );
        assert!(detail["server"]["packages"][0]["arguments"].is_array());
    }
    assert_eq!(keys.len(), 40);
}
