//! Pinned public schema discovery is local and grants no account authority.
use serde_json::{Value,json};
use std::sync::LazyLock;
use super::operations::Operation;
static DOCUMENT: LazyLock<Value> = LazyLock::new(||serde_json::from_str(include_str!("../../resources/github-requests-2026-03-10.json")).expect("generated pinned GitHub request schemas"));
pub fn search(query:&str,page:usize)->Result<Value,String> {
    if query.len()>200 || page>100 {return Err("Bounded GitHub schema search required.".into());}
    let query=query.to_ascii_lowercase();
    let rows:Vec<_>=DOCUMENT["operations"].as_array().unwrap().iter().filter(|row|query.split_whitespace().all(|q|format!("{} {} {}",row["id"],row["summary"],row["path"]).to_ascii_lowercase().contains(q))).collect();
    Ok(json!({"version":DOCUMENT["version"],"source_sha256":DOCUMENT["sha256"],"total":rows.len(),"next":if (page+1)*40<rows.len(){Some(page+1)}else{None},"items":rows.into_iter().skip(page*40).take(40).map(|r|json!({"id":r["id"],"method":r["method"],"path":r["path"],"summary":r["summary"]})).collect::<Vec<_>>()}))
}
pub(super) fn rest_row(spec:&super::operations::Spec)->Option<&'static Value> {
    let path=spec.path.replace("{repo}","{owner}/{repo}");
    DOCUMENT["operations"].as_array().unwrap().iter().find(|r|r["method"]==spec.method && shape(r["path"].as_str().unwrap())==shape(&path))
}
pub fn get(id:&str)->Result<Value,String> {
    if let Some(schema)=super::secrets::schema(id){return Ok(schema);}
    if let Some(schema)=super::uploads::schema(id){return Ok(schema);}
    if let Some(schema)=super::downloads::schema(id){return Ok(schema);}
    if id.len()>256 {return Err("Invalid GitHub schema identifier.".into());}
    if let Some(spec)=super::operations::SPECS.iter().find(|spec|spec.id==id) {
        let path=spec.path.replace("{repo}","{owner}/{repo}");
        if let Some(row)=DOCUMENT["operations"].as_array().unwrap().iter().find(|r|r["method"]==spec.method && shape(r["path"].as_str().unwrap())==shape(&path)) {return Ok(row.clone());}
        if spec.path=="graphql" {return Ok(super::operations::graphql_schema(id).unwrap_or_else(||json!({"id":id,"documentation":"https://docs.github.com/en/graphql/reference","note":"Advanced GraphQL is account-wide and always consequential; the GitHub server validates its graph schema. A selected repository does not constrain the query."})));}
    }
    DOCUMENT["operations"].as_array().unwrap().iter().find(|r|r["id"]==id).cloned().ok_or_else(||"GitHub operation is absent from the pinned schema.".into())
}
fn shape(path:&str)->String { path.split('/').map(|p|if p.starts_with('{') {"*"}else{p}).collect::<Vec<_>>().join("/") }
fn target<'a>(template:&str,path:&'a str)->Option<Vec<(&'a str,String)>> {
    let parts:Vec<_>=path.split('/').collect();let template:Vec<_>=template.split('/').collect();
    if parts.len()<template.len() {return None;}
    let mut values=vec![];
    for (index,segment) in template.iter().enumerate() {
        if segment.starts_with('{') {
            let name=segment.trim_start_matches('{').trim_end_matches('}');
            if index+1==template.len() && ["path","ref"].contains(&name) {
                values.push((path.splitn(index+1,'/').last()?,name.into()));return Some(values);
            }
            values.push((parts[index],name.into()));
        } else if parts[index]!=*segment {return None;}
    }
    (parts.len()==template.len()).then_some(values)
}
pub(super) fn private_endpoint(path:&str)->bool {
    // Match literal API resources in the pinned template, not user-selected
    // repository, label, ref or contents names that happen to say "secrets".
    let row=DOCUMENT["operations"].as_array().unwrap().iter().filter(|r|target(r["path"].as_str().unwrap(),path).is_some_and(|values|values.iter().all(|(value,name)|r["parameters"].as_array().unwrap().iter().find(|p|p["in"]=="path"&&p["name"]==*name).is_none_or(|p|p["schema"]["type"]!="integer"||value.parse::<u64>().is_ok())))).max_by_key(|r|r["path"].as_str().unwrap().split('/').filter(|s|!s.starts_with('{')).map(str::len).sum::<usize>());
    let template=row.and_then(|r|r["path"].as_str()).unwrap_or(path);
    template.split('/').filter(|s|!s.starts_with('{')).any(|s|super::credential_fields::sensitive(s)||["tokens","authorizations","access_tokens","secrets","keys"].contains(&s))
}
pub fn validate(operation:&Operation)->Result<(),String> {if operation.id.starts_with("secrets."){super::secrets::validate_schema(operation)}else{validate_inner(operation,false)}}
pub(super) fn validate_upload(operation:&Operation)->Result<(),String>{
    if operation.id!="releases.upload_asset"||operation.method!="POST"||operation.body.is_some(){return Err("Invalid native upload".into());}validate_inner(operation,true)
}
fn validate_inner(operation:&Operation,upload:bool)->Result<(),String>{
    if operation.endpoint=="graphql" || operation.id=="downloads.read" {return Ok(());}
    let url=reqwest::Url::parse(&format!("https://api.github.com/{}",operation.endpoint)).map_err(|_|"Invalid GitHub endpoint.")?;
    let path=url.path().trim_start_matches('/');
    // Prefer static/more-specific paths over generic parameter matches.
    let row=DOCUMENT["operations"].as_array().unwrap().iter().filter(|r|r["method"]==operation.method && target(r["path"].as_str().unwrap(),path).is_some()).max_by_key(|r|r["path"].as_str().unwrap().split('/').filter(|s|!s.starts_with('{')).map(str::len).sum::<usize>()).ok_or("GitHub endpoint/method is absent from the pinned API schema.")?;
    if !upload && row["binaryBody"]==true && (row["bodyRequired"]==true || operation.body.is_some() || matches!(operation.method.as_str(),"POST"|"PUT"|"PATCH")) {return Err("This endpoint requires the separate native binary-transfer capability.".into());}
    if let Some(body)=&operation.body {
        if row["body"].is_null() {return Err("This GitHub endpoint does not accept a JSON body.".into());}
        validate_value(&row["body"],body,0).map_err(|_|diagnostic(&row["body"],body,"body",0))?;
    } else if !upload && row["bodyRequired"]==true {return Err("GitHub request body is required by the pinned operation schema.".into());}
    let path_values=target(row["path"].as_str().unwrap(),path).unwrap();
    let query:Vec<_>=url.query_pairs().collect();
    for parameter in row["parameters"].as_array().unwrap() {
        let name=parameter["name"].as_str().unwrap();
        let raw=if parameter["in"]=="path" { path_values.iter().find(|(_,key)|key==name).map(|(value,_)|(*value).to_string()) }else{query.iter().find(|(key,_)|key==name).map(|(_,value)|value.to_string())};
        if let Some(raw)=raw {
            let schema=&parameter["schema"];
            let value=if schema["type"]=="integer" || schema["type"]=="number" || schema["type"]=="boolean" {serde_json::from_str(&raw).unwrap_or_else(|_|json!(raw))}
                else if schema["type"]=="array" {json!(raw.split(',').collect::<Vec<_>>())}else{json!(raw)};
            validate_value(schema,&value,0).map_err(|_|format!("GitHub parameter {name} does not match the pinned schema."))?;
        } else if parameter["required"]==true {return Err(format!("GitHub parameter {name} is required."));}
    }
    Ok(())
}
// Only schema-authored field names are reflected; never values or unknown keys.
fn diagnostic(schema:&Value,value:&Value,path:&str,depth:usize)->String {
    if depth < 16 {
        if let Some(required)=schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if value.get(key).is_none() {return format!("Missing required {path}.{key}. Put JSON request fields in top-level body.");}
            }
        }
        if let Some(properties)=schema["properties"].as_object() {
            for (key,child) in properties {
                if let Some(v)=value.get(key) {if validate_value(child,v,0).is_err(){return diagnostic(child,v,&format!("{path}.{key}"),depth+1);}}
            }
        }
    }
    format!("Invalid {path}: expected {} matching the operation schema constraints.",schema["type"].as_str().unwrap_or("a value"))
}
pub(super) fn validate_value(schema:&Value,value:&Value,depth:usize)->Result<(),()> {
    if depth>24 {return Err(());}
    if value.is_null() && schema["nullable"]==true {return Ok(());}
    if let Some(values)=schema["enum"].as_array() {if !values.contains(value) {return Err(());}}
    for (key,one) in [("anyOf",false),("oneOf",true)] {if let Some(rows)=schema[key].as_array() {
        if rows.len()>32 {return Err(());}let matches=rows.iter().filter(|s|validate_value(s,value,depth+1).is_ok()).count();if matches==0 || one && matches!=1 {return Err(());}
    }}
    if let Some(rows)=schema["allOf"].as_array() {for row in rows {validate_value(row,value,depth+1)?;}}
    let kind=schema["type"].as_str().unwrap_or("");
    if match kind {"null"=>!value.is_null(),"object"=>!value.is_object(),"array"=>!value.is_array(),"string"=>!value.is_string(),"integer"=>!(value.is_i64()||value.is_u64()),"number"=>!value.is_number(),"boolean"=>!value.is_boolean(),_=>false} {return Err(());}
    if let Some(text)=value.as_str() {
        let len=text.chars().count() as u64;
        if schema["minLength"].as_u64().is_some_and(|n|len<n)||schema["maxLength"].as_u64().is_some_and(|n|len>n) {return Err(());}
        if let Some(pattern)=schema["pattern"].as_str() {if let Ok(regex)=regex::RegexBuilder::new(pattern).size_limit(1024*1024).build() {if !regex.is_match(text) {return Err(());}}}
    }
    if let Some(number)=value.as_f64() {if schema["minimum"].as_f64().is_some_and(|n|number<n)||schema["maximum"].as_f64().is_some_and(|n|number>n) {return Err(());}}
    if let Some(items)=value.as_array() {
        if schema["minItems"].as_u64().is_some_and(|n|items.len()<n as usize)||schema["maxItems"].as_u64().is_some_and(|n|items.len()>n as usize) {return Err(());}
        for item in items {if !schema["items"].is_null() {validate_value(&schema["items"],item,depth+1)?;}}
    }
    if let Some(map)=value.as_object() {
        if schema["minProperties"].as_u64().is_some_and(|n|map.len()<n as usize)||schema["maxProperties"].as_u64().is_some_and(|n|map.len()>n as usize) {return Err(());}
        if let Some(keys)=schema["required"].as_array() {if keys.iter().any(|key|key.as_str().is_some_and(|key|!map.contains_key(key))) {return Err(());}}
        for (key,value) in map {if let Some(property)=schema["properties"].get(key) {validate_value(property,value,depth+1)?;}
            else if schema["additionalProperties"]==false {return Err(());}else if schema["additionalProperties"].is_object() {validate_value(&schema["additionalProperties"],value,depth+1)?;}}
    }
    Ok(())
}
#[cfg(test)] mod tests {
    use super::*;
    fn operation(id:&str,parameters:Value,body:Value)->Operation {serde_json::from_value::<super::super::operations::Request>(json!({"operation":id,"repo":"owned/fixture","parameters":parameters,"body":body})).unwrap().compile().unwrap()}
    #[test] fn every_rest_catalog_route_matches_the_pinned_official_description() {
        for spec in super::super::operations::SPECS {if spec.path=="graphql"||spec.id=="api.rest" {continue;}assert!(get(spec.id).is_ok(),"{}",spec.id);}
        // Search also legitimately matches issue comments/labels/milestones.
        let found=search("issues create",0).unwrap();
        assert!(found["items"].as_array().unwrap().iter().any(|row|row["id"]=="issues/create"));
        assert!(search("",0).unwrap()["items"].as_array().unwrap().len()<=40);
    }
    #[test] fn body_requirements_types_and_operation_methods_are_enforced() {
        assert!(validate(&operation("repos.contents",json!({"path":""}),Value::Null)).is_ok());
        assert!(validate(&operation("issues.create",json!({}),json!({"title":"Owned fixture"}))).is_ok());
        assert!(validate(&operation("issues.create",json!({}),json!({"body":"missing title"}))).is_err());
        assert!(validate(&operation("issues.create",json!({}),json!({"title":[],"body":"bad type"}))).is_err());
        assert!(validate(&operation("actions.dispatch",json!({"workflow_id":"owned.yml"}),json!({"ref":"main"}))).is_ok());
        assert!(serde_json::from_value::<super::super::operations::Request>(json!({"operation":"actions.dispatch","repo":"owned/fixture","parameters":{"workflow_id":"owned.yml"},"body":{"ref":false}})).unwrap().compile().is_err());
    }
    #[test] fn everyday_item_fields_match_pinned_schemas_and_reject_wrong_types() {
        for id in ["issues.update","pulls.update"] {
            for body in [json!({"title":"Literal <script>title</script>","body":""}),json!({"state":"open"}),json!({"state":"closed"})] {
                assert!(validate(&operation(id,json!({"number":7}),body)).is_ok(),"{id}");
            }
            assert!(validate(&operation(id,json!({"number":7}),json!({"state":"merged"}))).is_err());
        }
        for body in [json!({"labels":["help wanted","a,b"]}),json!({"labels":[]}),json!({"assignees":["owned"]}),json!({"assignees":[]}),json!({"milestone":4}),json!({"milestone":null})] {
            assert!(validate(&operation("issues.update",json!({"number":7}),body)).is_ok());
        }
        for body in [json!({"labels":false}),json!({"assignees":[false]}),json!({"milestone":{}})] {
            assert!(validate(&operation("issues.update",json!({"number":7}),body)).is_err());
        }
        assert!(validate(&operation("pulls.reviewers",json!({"number":7}),json!({"reviewers":["owned"],"team_reviewers":["core"]}))).is_ok());
        assert!(validate(&operation("pulls.reviewers",json!({"number":7}),json!({"reviewers":[{}]}))).is_err());
    }
}
