//! The model sees executable AG1 envelopes, not untranslated OpenAPI requests.
use super::{operations::{Request, Spec, SPECS}, schemas};
use serde_json::{Value, json};

fn spec(id: &str) -> Option<&'static Spec> {
    // Slash spellings of catalog names have catalog semantics. In particular,
    // issues/list is repository-scoped; rest:issues/list is the upstream API.
    let dotted = id.replace('/', ".");
    let dotted = match dotted.as_str() { "repos.releases" => "releases.list", other => other };
    SPECS.iter().find(|s| s.id == id || s.id == dotted).or_else(|| {
        if id.starts_with("rest:") { return None; }
        SPECS.iter().find(|s| schemas::rest_row(s).is_some_and(|r| r["id"] == id || r["id"].as_str().is_some_and(|raw|raw.replace('/',".")==dotted)))
    })
}

fn fields(path: &str) -> Vec<&str> {
    path.split('/').filter_map(|s| s.strip_prefix('{')?.strip_suffix('}')).collect()
}

pub(super) fn normalize(mut request: Request) -> Result<Request, String> {
    if request.repo.as_deref()==Some(""){request.repo=None;}
    if request.operation.starts_with("git.") { return Ok(request); }
    let s = spec(&request.operation).ok_or("Unknown executable GitHub operation. Use operation=catalog, then operation=schema with parameters.id from that catalog. Upstream-only endpoints use api.rest.")?;
    request.operation = s.id.into();
    if s.path.contains("{repo}") {
        let owner = request.parameters.remove("owner");
        let repo = request.parameters.remove("repo");
        if owner.is_some() || repo.is_some() {
            let pair = match (owner.as_ref().and_then(Value::as_str), repo.as_ref().and_then(Value::as_str)) {
                (Some(o), Some(r)) => format!("{o}/{r}"),
                (None, Some(r)) if r.split('/').count()==2 => r.into(),
                _ => return Err("parameters.owner and parameters.repo must both be strings; alternatively use top-level repo='owner/name'.".into()),
            };
            if request.repo.as_ref().is_some_and(|r| !r.eq_ignore_ascii_case(&pair)) {
                return Err("Top-level repo conflicts with parameters.owner/repo.".into());
            }
            request.repo = Some(pair);
        }
        if let Some(row) = schemas::rest_row(s) {
            let upstream = fields(row["path"].as_str().unwrap());
            let native = fields(s.path);
            for (from, to) in upstream.into_iter().filter(|k| *k != "owner" && *k != "repo")
                .zip(native.into_iter().filter(|k| *k != "repo")) {
                if from != to {
                    if let Some(value) = request.parameters.remove(from) {
                        if request.parameters.get(to).is_some_and(|v| v != &value) {
                            return Err(format!("Conflicting parameters.{to} and its upstream alias."));
                        }
                        request.parameters.insert(to.into(), value);
                    }
                }
            }
        }
        if s.id == "repos.contents" { request.parameters.entry("path".into()).or_insert(json!("")); }
    }
    // Accept the layouts seen in actual model calls, but never silently choose
    // between contradictory values or reinterpret an operation's target.
    let body_schema=if s.id=="api.graphql" {json!({"properties":{"query":{},"variables":{},"operationName":{}}})}
        else {schemas::rest_row(s).map(|r|r["body"].clone()).unwrap_or(Value::Null)};
    if body_schema.is_object() {
        if request.parameters.get("body").is_some_and(Value::is_object) {
            let nested=request.parameters.remove("body").unwrap();
            merge_body(&mut request.body,nested)?;
        }
        if let Some(properties)=body_schema["properties"].as_object() {
            let query_names:Vec<_>=schemas::rest_row(s).and_then(|r|r["parameters"].as_array()).map(|ps|ps.iter().map(|p|p["name"].as_str().unwrap()).collect()).unwrap_or_default();
            let keys:Vec<_>=request.parameters.keys().filter(|key|properties.contains_key(*key)&&!query_names.contains(&key.as_str())).cloned().collect();
            for key in keys {let value=request.parameters.remove(&key).unwrap();merge_body(&mut request.body,json!({key:value}))?;}
        }
    }
    Ok(request)
}
fn merge_body(target:&mut Value,source:Value)->Result<(),String>{
    if target.is_null(){*target=json!({});}
    let target=target.as_object_mut().ok_or("body must be a JSON object.")?;
    let source=source.as_object().ok_or("parameters.body must be a JSON object.")?;
    for (key,value) in source {
        if target.get(key).is_some_and(|old|old!=value){return Err("Conflicting body fields between top-level body and parameters; use one consistent value.".into());}
        target.insert(key.clone(),value.clone());
    }
    Ok(())
}

fn operation_schema(id: &str) -> Result<Value, String> {
    if id.len() > 256 { return Err("Invalid GitHub schema identifier.".into()); }
    let Some(s) = spec(id) else {
        let row = schemas::get(id.strip_prefix("rest:").unwrap_or(id)).map_err(|_| "Unknown GitHub schema ID. For releases use releases.list, releases.get, releases.create, releases.update or releases.delete (repos/releases aliases releases.list). Search with {operation: schema, parameters: {query: releases}}; do not repeat the same missing ID.".to_string())?;
        return Ok(json!({"id":format!("rest:{}",row["id"].as_str().unwrap_or(id)),"execute_as":"api.rest",
            "note":"Upstream REST schema: substitute path parameters into parameters.endpoint; set parameters.method, put query values in parameters and JSON fields in body. Always consequential. This ID is for discovery, not direct execution.","upstream":row}));
    };
    if let Some(row) = schemas::rest_row(s).filter(|_| !super::downloads::remote(s.id) && s.id != "releases.upload_asset") {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        let upstream = fields(row["path"].as_str().unwrap());
        let native = fields(s.path);
        let aliases: Vec<_> = upstream.into_iter().filter(|k| *k != "owner" && *k != "repo")
            .zip(native.into_iter().filter(|k| *k != "repo")).collect();
        for p in row["parameters"].as_array().unwrap() {
            let name = p["name"].as_str().unwrap();
            if p["in"] == "path" && ["owner", "repo"].contains(&name) { continue; }
            let name = if p["in"] == "path" { aliases.iter().find(|(a,_)| *a == name).map(|(_,b)| *b).unwrap_or(name) } else { name };
            let mut property = p["schema"].clone();
            if name == "path" && s.id == "repos.contents" { property["default"] = json!(""); }
            else if p["required"] == true { required.push(name); }
            if name == "per_page" { property["maximum"] = json!(100); }
            if name == "page" { property["maximum"] = json!(1000); }
            properties.insert(name.into(), property);
        }
        let mut body = row["body"].clone();
        if let Some(key) = match s.id { "pulls.merge" => Some("sha"), "pulls.review" => Some("commit_id"), _ => None } {
            body["properties"][key] = json!({"type":"string","pattern":"^[a-fA-F0-9]{40}$","description":"Exact reviewed commit SHA required by AG1."});
            if !body["required"].is_array() { body["required"] = json!([]); }
            if !body["required"].as_array().unwrap().contains(&json!(key)) { body["required"].as_array_mut().unwrap().push(json!(key)); }
        }
        let mut result = json!({"id":s.id,"method":s.method,"path":s.path,"risk":s.risk,"upstream_id":row["id"],
            "repo":if s.path.contains("{repo}") { "Required top-level string: owner/name" } else { "Omit" },
            "parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
            "body":body,"bodyRequired":row["bodyRequired"] == true || matches!(s.id,"pulls.merge"|"pulls.review"),
            "usage":"Execute with operation equal to this id, top-level repo='owner/name' when required, parameters for path/query fields, and body for JSON request fields."});
        if result["parameters"]["properties"].get("page").is_some() {
            result["pagination"] = json!("Set parameters.page (starts at 1) and per_page (1–100; use 10 for exploration). When reply.next is present, repeat the same operation/filters with its next page. Schema-search pages start at 0.");
        }
        return Ok(result);
    }
    if s.id == "api.graphql" {
        return Ok(json!({"id":s.id,"risk":s.risk,"repo":"Omit: graph queries are account-wide or target explicit nodes.",
            "parameters":{"type":"object","additionalProperties":false},"bodyRequired":true,
            "body":{"type":"object","required":["query"],"additionalProperties":false,"properties":{"query":{"type":"string","minLength":1,"maxLength":32768},"variables":{"type":"object"},"operationName":{"type":"string"}}},
            "example":{"operation":"api.graphql","body":{"query":"query { viewer { login } rateLimit { cost remaining resetAt } }"}},
            "pagination":"Select GraphQL connection pageInfo { hasNextPage endCursor }; pass endCursor as after in the next query. Queries can select several fields in one call; advanced GraphQL requires approval unless the user enabled Bypass approvals for this chat."}));
    }
    if s.id == "api.rest" {
        return Ok(json!({"id":s.id,"risk":s.risk,"parameters":{"type":"object","required":["method","endpoint"],"properties":{"method":{"enum":["GET","POST","PUT","PATCH","DELETE"]},"endpoint":{"type":"string","description":"Relative API path, no leading slash, host or query. Put query fields alongside method/endpoint."}},"additionalProperties":true},
            "example":{"operation":"api.rest","parameters":{"method":"GET","endpoint":"issues","per_page":10}},"note":"Discover upstream-only schemas with parameters.id='rest:issues/list'. This example lists issues assigned to the account; issues.list lists repository issues."}));
    }
    let mut result = schemas::get(s.id)?;
    result["id"] = json!(s.id);
    result["risk"] = json!(s.risk);
    Ok(result)
}

pub(super) fn schema(id:&str)->Result<Value,String>{
    let mut result=operation_schema(id)?;
    let Some(s)=spec(id) else {return Ok(result)};
    let mut properties=json!({"operation":{"type":"string","const":s.id}});
    let mut required=vec!["operation"];
    if s.path.contains("{repo}"){properties["repo"]=json!({"type":"string","description":"owner/name"});required.push("repo");}
    if result["parameters"]["type"]=="object" {properties["parameters"]=result["parameters"].clone();if result["parameters"]["required"].as_array().is_some_and(|a|!a.is_empty()){required.push("parameters");}}
    if result["body"].is_object(){properties["body"]=result["body"].clone();if result["bodyRequired"]==true{required.push("body");}}
    if result["parameters"]["type"]=="object" {result["input_schema"]=json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});}
    if result.get("example").is_none() && schemas::rest_row(s).is_some() && result["parameters"]["type"]=="object" {
        let mut example=json!({"operation":s.id});
        if s.path.contains("{repo}"){example["repo"]=json!("owner/name");}
        let mut params=json!({});
        for key in result["parameters"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str){params[key]=sample(&result["parameters"]["properties"][key],key);}
        if result["parameters"]["properties"].get("per_page").is_some(){params["per_page"]=json!(10);params["page"]=json!(1);}
        if params.as_object().is_some_and(|p|!p.is_empty()){example["parameters"]=params;}
        if result["bodyRequired"]==true{example["body"]=sample(&result["body"],"body");}
        if s.id=="pulls.create"{example["body"]["title"]=json!("Example pull request");}
        result["example"]=example;
    }
    result["accepted_layouts"]=json!("Use example as the complete tool arguments. Slash/dot aliases are equivalent. For REST, owner/repo may also be paired in parameters or parameters.repo may be owner/name. A nested parameters.body object and unambiguous body fields in parameters are accepted; conflicting duplicate values are rejected.");
    Ok(result)
}
fn sample(schema:&Value,key:&str)->Value{
    if let Some(v)=schema["enum"].as_array().and_then(|a|a.first()){return v.clone();}
    if let Some(v)=schema["anyOf"].as_array().or_else(||schema["oneOf"].as_array()).and_then(|a|a.first()){return sample(v,key);}
    match schema["type"].as_str().unwrap_or("") {
        "object"=>{let mut v=json!({});for k in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str){v[k]=sample(&schema["properties"][k],k);}v},
        "integer"|"number"=>json!(1),"boolean"=>json!(false),"array"=>json!([]),
        _=>json!(match key {"sha"|"commit_id"=>"a".repeat(40),"base"|"ref"=>"main".into(),"head"=>"topic".into(),"tag_name"=>"v1.0.0".into(),"q"=>"example".into(),_=>"Example".into()})
    }
}

pub(super) fn catalog(args:&Value)->Result<Value,String>{
    let query=args["parameters"]["query"].as_str().unwrap_or("");
    if query.len()>200{return Err("Catalog query must be at most 200 characters.".into());}
    let query=query.to_ascii_lowercase().replace('/',".");
    let operations:Vec<_>=SPECS.iter().filter(|s|query.split_whitespace().all(|q|format!("{} {}",s.id,s.area).contains(q))).map(|s|{
        let mut row=json!({"id":s.id,"risk":s.risk});
        if s.path.contains("{repo}"){row["repo_required"]=json!(true);}
        if let Ok(schema)=operation_schema(s.id){
            for (from,to) in [("parameters","required_parameters"),("body","required_body_fields")] {
                if schema[from]["required"].as_array().is_some_and(|a|!a.is_empty()){row[to]=schema[from]["required"].clone();}
            }
        }
        let scope=match s.id {
            "issues.labels"=>Some("All repository labels; no issue number. For one issue use issues.issue_labels."),
            "issues.list"=>Some("Issues in repo, not assigned account issues."),
            "actions.workflow_runs"=>Some("Runs for one workflow; workflow_id required. Use actions.runs for all repository workflows."),
            "actions.runs"=>Some("Runs across all workflows in repo."),
            "api.graphql"=>Some("Account-wide graph: omit repo or use an empty string. Query in body.query (parameters.query is also accepted)."),
            "repos.list"|"rate_limit.get"=>Some("Account-level read; repo is unnecessary."),
            _=>None
        };
        if let Some(scope)=scope{row["scope"]=json!(scope);}
        row
    }).collect();
    Ok(json!({"operations":operations,"operation_count":SPECS.len(),
        "instructions":"IDs below are executable canonical IDs. Slash aliases also work; no guessing is needed. Discover a complete call with operation=schema and parameters.id equal to the chosen ID. Copy its example; input_schema describes the entire argument object. Missing required body fields are shown by schema. Unknown GitHub schema search is local.",
        "schema_example":{"operation":"schema","parameters":{"id":"releases.list"}},
        "read_example":{"operation":"releases.list","repo":"owner/name","parameters":{"per_page":10,"page":1}},
        "write_example":{"operation":"issues.create","repo":"owner/name","body":{"title":"Example issue"}},
        "pagination":"REST list/search operations expose page and per_page in schema; start at page 1 and use per_page 10. Preserve filters when following reply.next. GraphQL uses pageInfo/endCursor; schema search pages start at 0.",
        "quota":"rate_limit.get inspects separate quota buckets; respect rate_reset and retry times. GraphQL can combine several selected fields into one bounded query. Native account verification also uses core requests. Artifact/schema/catalog reads are local and cost zero GitHub requests.",
        "large_results":"Large lists return a compact preview with metadata and a local artifact readCall for the exact original JSON. Reading that artifact does not repeat the GitHub request.",
        "git":"Hosted files/issues/PRs/actions use the IDs above. git.status/diff/branches inspect a granted local checkout; git.stage/unstage/commit/branch_create change it. git.fetch/push/clone/fetch_pr/pull transfer Git data under native account/target checks; checkout/pull change working files. For all local Git arguments and limits call catalog with parameters.detail=true.",
        "scope":"issues.list (also issues/list) lists repository issues. For assigned account issues use api.rest with endpoint=issues; its upstream schema is rest:issues/list. Advanced calls remain consequential. repos/releases is a compatibility alias of releases.list.",
        "detail_call":{"operation":"catalog","parameters":{"detail":true}},"filter_call":{"operation":"catalog","parameters":{"query":"releases"}}}))
}

pub(super) fn search(query: &str, page: usize) -> Result<Value, String> {
    let mut result = schemas::search(query, page)?;
    for row in result["items"].as_array_mut().unwrap() {
        // Search is upstream discovery: use an explicit namespace even where a
        // slash name collides with a catalog shorthand (issues/list).
        row["id"] = json!(format!("rest:{}", row["id"].as_str().unwrap()));
        row["execute_as"] = json!("api.rest");
    }
    let words = query.to_ascii_lowercase();
    result["catalog_matches"] = json!(SPECS.iter().filter(|s| words.split_whitespace().all(|w| format!("{} {}",s.id,s.area).contains(w))).map(|s| s.id).collect::<Vec<_>>());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn compile(value: Value) -> super::super::operations::Operation {
        let request = super::super::tool::request(&value).unwrap();
        let operation = request.compile().unwrap();
        schemas::validate(&operation).unwrap();
        operation
    }
    #[test]
    fn empty_optional_scope_and_catalog_required_fields_match_live_model_calls() {
        let op=compile(json!({"operation":"api.graphql","repo":"","parameters":{"query":"{ viewer { login } }"}}));
        assert_eq!(op.repo,None);assert_eq!(op.risk,super::super::operations::Risk::Consequential);
        assert!(super::super::tool::request(&json!({"operation":"repos.get","repo":""})).unwrap().compile().unwrap_err().contains("Missing top-level repo"));
        let c=catalog(&json!({})).unwrap();let rows=c["operations"].as_array().unwrap();
        assert!(rows.iter().find(|r|r["id"]=="actions.workflow_runs").unwrap()["required_parameters"].as_array().unwrap().contains(&json!("workflow_id")));
        assert!(rows.iter().find(|r|r["id"]=="api.graphql").unwrap()["required_body_fields"].as_array().unwrap().contains(&json!("query")));
        let labels=compile(json!({"operation":"issues.issue_labels","repo":"owned/fixture","parameters":{"number":7}}));assert_eq!(labels.endpoint,"repos/owned/fixture/issues/7/labels");
    }
    #[test]
    fn observed_session_body_layouts_normalize_without_guessing_targets() {
        for args in [
            json!({"operation":"pulls/create","parameters":{"owner":"owned","repo":"fixture","title":"Example","head":"topic","base":"main","body":"Text"}}),
            json!({"operation":"pulls/create","parameters":{"owner":"owned","repo":"fixture","body":{"title":"Example","head":"topic","base":"main","body":"Text"}}}),
            json!({"operation":"pulls.create","parameters":{"repo":"owned/fixture","body":{"title":"Example","head":"topic","base":"main","body":"Text"}}}),
            json!({"operation":"pulls.create","repo":"owned/fixture","body":{"title":"Example","head":"topic","base":"main","body":"Text"}}),
        ] {
            let op=compile(args);assert_eq!(op.id,"pulls.create");assert_eq!(op.endpoint,"repos/owned/fixture/pulls");
            assert_eq!(op.body,Some(json!({"title":"Example","head":"topic","base":"main","body":"Text"})));
        }
        let graph=compile(json!({"operation":"api.graphql","parameters":{"query":"{ viewer { login } }"}}));
        assert_eq!(graph.risk,super::super::operations::Risk::Consequential);
        for args in [
            json!({"operation":"issues.create","repo":"owned/fixture","parameters":{"title":"Wrong"},"body":{"title":"Right"}}),
            json!({"operation":"pulls.merge","repo":"owned/fixture","parameters":{"number":7,"body":{"sha":"a".repeat(40)}},"body":{"sha":"b".repeat(40)}}),
            json!({"operation":"api.graphql","parameters":{"body":{"query":"query { viewer { login } }","accessToken":"private"}}}),
        ] {assert!(super::super::tool::request(&args).map_err(|e|format!("{e:?}")).and_then(|r|r.compile()).is_err());}
    }
    #[test]
    fn release_aliases_complete_examples_and_pagination_round_trip() {
        for id in ["repos/releases","repos.releases","releases/list","releases.list","repos/list-releases","repos.list-releases"] {
            let s=schema(id).unwrap();assert_eq!(s["id"],"releases.list");compile(s["example"].clone());
            assert_eq!(compile(json!({"operation":id,"repo":"owned/fixture"})).endpoint,"repos/owned/fixture/releases");
        }
        for id in ["releases.list","releases.get","releases.create","releases.update","releases.delete","repos.list","repos.search","repos.contents","issues.create","pulls.create","pulls.merge","rate_limit.get"] {
            let s=schema(id).unwrap();let example=&s["example"];
            schemas::validate_value(&s["input_schema"],example,0).unwrap();compile(example.clone());
            if ["repos.list","repos.search","releases.list"].contains(&id){assert!(s["pagination"].is_string());assert_eq!(example["parameters"]["per_page"],10);}
        }
        let c=super::super::tool::discovery(&json!({"operation":"catalog"})).unwrap();
        assert!(c.to_string().len()<16000);assert!(c["operations"].as_array().unwrap().iter().all(|r|!r["id"].as_str().unwrap().contains('/')));
        let filtered=catalog(&json!({"parameters":{"query":"releases"}})).unwrap();
        assert!(filtered["operations"].as_array().unwrap().len()<SPECS.len());
        assert!(filtered["operations"].as_array().unwrap().iter().any(|r|r["id"]=="releases.delete"));
    }
    #[test]
    fn reported_model_calls_compile_to_the_intended_routes() {
        for (id, parameters, body, endpoint) in [
            ("repos.get",json!({}),Value::Null,"repos/owned/fixture"),
            ("repos/get",json!({"owner":"owned","repo":"fixture"}),Value::Null,"repos/owned/fixture"),
            ("repos/contents",json!({}),Value::Null,"repos/owned/fixture/contents/"),
            ("repos/get-content",json!({"path":"src/lib.rs"}),Value::Null,"repos/owned/fixture/contents/src/lib.rs"),
            ("issues/list",json!({"per_page":10}),Value::Null,"repos/owned/fixture/issues?per_page=10"),
            ("issues/get",json!({"issue_number":7}),Value::Null,"repos/owned/fixture/issues/7"),
            ("issues/create",json!({}),json!({"title":"Fixture"}),"repos/owned/fixture/issues"),
            ("pulls/list",json!({}),Value::Null,"repos/owned/fixture/pulls"),
            ("pulls/get",json!({"pull_number":7}),Value::Null,"repos/owned/fixture/pulls/7"),
            ("pulls/create",json!({}),json!({"title":"Fixture","head":"topic","base":"main"}),"repos/owned/fixture/pulls"),
            ("pulls/merge",json!({"pull_number":7}),json!({"sha":"a".repeat(40)}),"repos/owned/fixture/pulls/7/merge"),
            ("actions/runs",json!({}),Value::Null,"repos/owned/fixture/actions/runs"),
            ("actions/workflows",json!({}),Value::Null,"repos/owned/fixture/actions/workflows"),
        ] {
            let operation = compile(json!({"operation":id,"repo":"owned/fixture","parameters":parameters,"body":body}));
            assert_eq!(operation.endpoint,endpoint,"{id}");
        }
        let graph = compile(json!({"operation":"api.graphql","body":{"query":"query { viewer { login } }"}}));
        assert_eq!(graph.endpoint,"graphql");
        assert_eq!(graph.risk,super::super::operations::Risk::Consequential);
    }
    #[test]
    fn every_catalog_id_and_slash_alias_is_discoverable_with_a_stable_id() {
        for s in SPECS {
            for id in [s.id.to_owned(),s.id.replace('.',"/")] {
                let result = super::super::tool::discovery(&json!({"operation":"schema","parameters":{"id":id}})).unwrap();
                assert_eq!(result["id"],s.id,"{id}");
            }
        }
    }
    #[test]
    fn native_schema_fields_are_executable_and_upstream_scope_is_explicit() {
        let get = schema("pulls/get").unwrap();
        assert!(get["parameters"]["properties"].get("number").is_some());
        assert!(get["parameters"]["properties"].get("owner").is_none());
        assert!(get["parameters"]["properties"].get("pull_number").is_none());
        assert_eq!(schema("issues/list").unwrap()["path"],"repos/{repo}/issues");
        let upstream = schema("rest:issues/list").unwrap();
        assert_eq!(upstream["upstream"]["path"],"issues");
        assert_eq!(upstream["execute_as"],"api.rest");
        assert_eq!(compile(json!({"operation":"api.rest","parameters":{"method":"GET","endpoint":"issues"}})).endpoint,"issues");
        assert_eq!(schema("issues.create").unwrap()["bodyRequired"],true);
        assert!(schema("pulls.merge").unwrap()["body"]["required"].as_array().unwrap().contains(&json!("sha")));
        let graph = schema("api.graphql").unwrap();
        assert!(graph["body"]["required"].as_array().unwrap().contains(&json!("query")));
        compile(graph["example"].clone());
        assert!(schema("repos.search").unwrap()["pagination"].is_string());
        assert_eq!(schema("search/repos").unwrap()["id"],"repos.search");
        assert!(search("issues list",0).unwrap()["items"].as_array().unwrap().iter().all(|r|r["id"].as_str().unwrap().starts_with("rest:")));
    }
    #[test]
    fn aliases_cannot_change_targets_or_weaken_consequential_requirements() {
        for value in [
            json!({"operation":"repos/get","repo":"owned/fixture","parameters":{"owner":"other","repo":"target"}}),
            json!({"operation":"pulls/get","repo":"owned/fixture","parameters":{"number":1,"pull_number":2}}),
            json!({"operation":"repos/get","parameters":{"owner":"owned"}}),
            json!({"operation":"repos/get","parameters":{"owner":"..","repo":"fixture"}}),
            json!({"operation":"pulls/merge","repo":"owned/fixture","parameters":{"pull_number":7},"body":{}}),
            json!({"operation":"api/graphql","body":{"query":"mutation { destructive }","accessToken":"private"}}),
        ] {
            let result=super::super::tool::request(&value).map_err(|e|format!("{e:?}")).and_then(|r|r.compile());
            assert!(result.is_err(),"{value}");
        }
    }
    #[test]
    fn failures_explain_required_fields_without_echoing_supplied_values() {
        let missing_repo=super::super::tool::request(&json!({"operation":"repos.get"})).unwrap().compile().unwrap_err();
        assert!(missing_repo.contains("top-level repo"));
        let missing_number=super::super::tool::request(&json!({"operation":"issues.get","repo":"owned/fixture"})).unwrap().compile().unwrap_err();
        assert!(missing_number.contains("parameters.number"));
        let missing_body=super::super::tool::request(&json!({"operation":"issues.create","repo":"owned/fixture"})).unwrap().compile().unwrap();
        assert!(schemas::validate(&missing_body).unwrap_err().contains("body is required"));
        for (body, expected) in [(json!({"body":"private-sentinel"}),"body.title"),(json!({"title":["private-sentinel"]}),"body.title")] {
            let operation=super::super::tool::request(&json!({"operation":"issues.create","repo":"owned/fixture","body":body})).unwrap().compile().unwrap();
            let error=schemas::validate(&operation).unwrap_err();
            assert!(error.contains(expected),"{error}");
            assert!(!error.contains("private-sentinel"));
        }
    }
    #[test]
    fn native_service_preserves_actionable_validation_errors_before_account_access() {
        use std::{sync::{Arc,atomic::AtomicBool},time::{Instant,Duration}};
        let dir=tempfile::tempdir().unwrap();
        let service=super::super::service::Service::new(dir.path().into(),Arc::new(super::super::transport::NativeTransport));
        let cancel=AtomicBool::new(false);
        let control=crate::resource_fs::ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(1)};
        for (args,expected) in [
            (json!({"operation":"repos.get"}),"top-level repo"),
            (json!({"operation":"issues.create","repo":"owned/fixture"}),"body is required"),
            (json!({"operation":"issues.create","repo":"owned/fixture","body":{"body":"private-sentinel"}}),"body.title"),
        ] {
            let request=super::super::tool::request(&args).unwrap();
            let error=match service.prepare(request,"owned-scope",&control) {Err(e)=>e,Ok(_)=>panic!("invalid request admitted")};
            assert!(error.message.contains(expected),"{}",error.message);
            assert!(!error.message.contains("private-sentinel"));
        }
    }
}
