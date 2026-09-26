//! Compiled only into isolated visual-test candidates. No live GitHub traffic.
use ag1_platform::{github::{GitHub,operations::Operation,service::{Account,ApiError,Outcome,Reply,Result,Service,Transport,check}},resource_fs::ScanControl};
use serde_json::json;
use std::{io::Write,path::{Path,PathBuf},sync::{Arc,Mutex}};
#[path="github_secret_fixture.rs"]
mod secret_fixture;
#[path="github_workflow_fixture.rs"]
mod workflow_fixture;
struct Fixture { workflows:Mutex<serde_json::Value>,secrets:Mutex<Vec<serde_json::Value>>, root:PathBuf, serial:Mutex<u64>, item_changes:Mutex<(serde_json::Value,serde_json::Value)>, graph_changes:Mutex<serde_json::Value>, uploads:Mutex<Vec<serde_json::Value>> }
impl Transport for Fixture {
    fn set_secret(&self,account:&Account,operation:&Operation,entry:&dyn ag1_platform::github::secrets::PrivateEntry,control:&ScanControl<'_>)->Result<Reply>{secret_fixture::set(self,account,operation,entry,control)}
    fn upload(&self,expected:&Account,operation:&Operation,upload:&ag1_platform::github::uploads::Upload,control:&ScanControl<'_>)->Result<Reply>{
        if &self.account(control)?!=expected{return Err(ApiError::new("stale_account","Fixture account changed."));}upload.verify(control)?;
        let op=serde_json::to_value(operation).map_err(|_|ApiError::new("fixture","Invalid upload."))?;
        let mut serial=self.serial.lock().map_err(|_|ApiError::new("fixture","Fixture busy."))?;*serial+=1;
        let mut log=std::fs::OpenOptions::new().create(true).append(true).open(self.root.join("github-requests.jsonl")).map_err(|_|ApiError::new("fixture","Cannot record upload."))?;
        writeln!(log,"{}",json!({"serial":*serial,"operation":op,"source":upload.preview()})).map_err(|_|ApiError::new("fixture","Cannot record upload."))?;
        let mut assets=self.uploads.lock().map_err(|_|ApiError::new("fixture","Fixture busy."))?;let id=1001+assets.len();
        let name=op["endpoint"].as_str().unwrap().split_once("?name=").unwrap().1;
        std::fs::write(self.root.join(format!("uploaded-{id}.bin")),upload.bytes()).map_err(|_|ApiError::new("fixture","Cannot retain fixture upload."))?;
        let data=json!({"id":id,"name":name,"size":upload.bytes().len(),"state":"uploaded","digest":format!("sha256:{}",upload.preview()["sha256"].as_str().unwrap())});assets.push(data.clone());
        Ok(Reply{data,status:201,next:None,rate_remaining:Some(4999),rate_reset:None,rate_limit:None,rate_resource:None,outcome:Outcome::Applied})
    }

    fn download(&self,expected:&Account,operation:&Operation,control:&ScanControl<'_>)->Result<ag1_platform::github::downloads::Payload> {
        if &self.account(control)?!=expected {return Err(ApiError::new("stale_account","Fixture identity changed."));}
        let op=serde_json::to_value(operation).map_err(|_|ApiError::new("fixture","Invalid fixture download."))?;
        let mut serial=self.serial.lock().map_err(|_|ApiError::new("fixture","Fixture busy."))?;*serial+=1;
        let mut log=std::fs::OpenOptions::new().create(true).append(true).open(self.root.join("github-requests.jsonl")).map_err(|_|ApiError::new("fixture","Cannot record download."))?;
        writeln!(log,"{}",json!({"serial":*serial,"operation":op})).map_err(|_|ApiError::new("fixture","Cannot record download."))?;
        let asset_id=op["endpoint"].as_str().unwrap_or("").rsplit('/').next().and_then(|s|s.parse::<usize>().ok()).unwrap_or(0);
        let bytes=if op["id"]=="releases.download_asset"&&asset_id>=1001 {std::fs::read(self.root.join(format!("uploaded-{asset_id}.bin"))).map_err(|_|ApiError::new("fixture","Missing uploaded fixture bytes."))?}else if op["id"]=="actions.download_job_log" {b"Owned job log\n<script>literal log</script>\r\n".to_vec()}else{(0..300000).map(|n|(n%256)as u8).collect()};
        Ok(ag1_platform::github::downloads::Payload{bytes})
    }

    fn account(&self,control:&ScanControl<'_>)->Result<Account> {
        check(control)?;
        Account::bound(9001,"owned-fixture".into(),"owned-fixture-credential-v1".into())
    }
    fn execute(&self,expected:&Account,operation:&Operation,control:&ScanControl<'_>)->Result<Reply> {
        if &self.account(control)?!=expected {return Err(ApiError::new("stale_account","Fixture identity changed."));}
        let op=serde_json::to_value(operation).map_err(|_|ApiError::new("fixture","Fixture request invalid."))?;
        let endpoint=op["endpoint"].as_str().unwrap_or("");let method=op["method"].as_str().unwrap_or("");
        let mut serial=self.serial.lock().map_err(|_|ApiError::new("fixture","Fixture busy."))?;*serial+=1;
        let mut log=std::fs::OpenOptions::new().create(true).append(true).open(self.root.join("github-requests.jsonl")).map_err(|_|ApiError::new("fixture","Cannot retain fixture request."))?;
        writeln!(log,"{}",json!({"serial":*serial,"operation":op})).map_err(|_|ApiError::new("fixture","Cannot retain fixture request."))?;
        let base=endpoint.split('?').next().unwrap_or("");
        if op["id"].as_str().is_some_and(|s|s.starts_with("secrets.")){return secret_fixture::metadata(self,&op,operation);}
        if let Some(reply)=workflow_fixture::execute(self,&op){return reply;}
        let mut issue=json!({"id":101,"number":7,"title":"Owned fixture issue","body":"Literal repository output <script>not executable</script>","state":"open","html_url":"https://github.com/owned/fixture/issues/7","user":{"login":"owned-fixture"}});
        let mut pull=json!({"id":201,"number":7,"title":"Owned fixture pull request","body":"Read the exact owned review page. <script>literal only</script>","state":"open","draft":false,"mergeable":null,"html_url":"https://github.com/owned/fixture/pull/7","head":{"sha":"a".repeat(40),"label":"owned:feature"},"base":{"sha":"b".repeat(40),"label":"owned:main","ref":"main"},"user":{"login":"owned-fixture"}});
        let mut changes=self.item_changes.lock().map_err(|_|ApiError::new("fixture","Fixture state busy."))?;
        if method=="PATCH" && ["repos/owned/fixture/issues/7","repos/owned/fixture/pulls/7"].contains(&base) {
            let body=op["body"].as_object().ok_or_else(||ApiError::new("fixture","Expected fixture update object."))?;
            let target=if base.contains("/issues/"){&mut changes.0}else{&mut changes.1};
            for (key,value) in body {target[key]=match key.as_str(){"assignees"=>json!(value.as_array().unwrap().iter().map(|v|json!({"login":v})).collect::<Vec<_>>()),"milestone" if !value.is_null()=>json!({"number":value,"title":format!("Owned milestone {value}")}),_=>value.clone()};}
        }
        if method=="POST" && base=="repos/owned/fixture/pulls/7/requested_reviewers" {
            changes.1["requested_reviewers"]=json!(op["body"]["reviewers"].as_array().unwrap().iter().map(|v|json!({"login":v})).collect::<Vec<_>>());
            changes.1["requested_teams"]=json!(op["body"]["team_reviewers"].as_array().unwrap().iter().map(|v|json!({"slug":v})).collect::<Vec<_>>());
        }
        for (key,value) in changes.0.as_object().unwrap(){issue[key]=value.clone();if ["labels","assignees","milestone"].contains(&key.as_str()){pull[key]=value.clone();}}
        for (key,value) in changes.1.as_object().unwrap(){pull[key]=value.clone();}
        let second=endpoint.split('?').nth(1).is_some_and(|q|q.split('&').any(|p|p=="page=2"));
        let list_state=endpoint.split('?').nth(1).and_then(|q|q.split('&').find_map(|p|p.strip_prefix("state="))).unwrap_or("open");
        let data=if op["id"]=="pulls.review" {json!({"id":301,"commit_id":op["body"]["commit_id"],"state":match op["body"]["event"].as_str(){Some("APPROVE")=>"APPROVED",Some("REQUEST_CHANGES")=>"CHANGES_REQUESTED",Some("COMMENT")=>"COMMENTED",_=>"PENDING"}})}
        else if op["id"]=="pulls.merge" {json!({"merged":true,"sha":"c".repeat(40),"message":"Owned fixture merged"})}
        else if method!="GET" && base!="graphql" {json!({"id":102,"number":8,"ok":true})}
        else {match base {
            "user/repos"=>json!([{"id":1,"full_name":"owned/fixture","description":"Owned offline GitHub fixture","private":true,"default_branch":"main","html_url":"https://github.com/owned/fixture"}]),
            "repos/owned/fixture"=>json!({"id":1,"full_name":"owned/fixture","private":true}),
            "repos/owned/fixture/issues"=>if list_state=="all"||issue["state"]==list_state{json!([issue])}else{json!([])},
            "repos/owned/fixture/issues/7"=>issue,
            "repos/owned/fixture/issues/7/comments"=>json!([{"id":if second{2}else{1},"body":if second{"Owned second comment page"}else{"Owned first comment <script>literal</script>"},"user":{"login":"owned-fixture"},"created_at":"2026-09-10T12:00:00Z"}]),
            "repos/owned/fixture/pulls"=>if list_state=="all"||pull["state"]==list_state{json!([pull])}else{json!([])},
            "repos/owned/fixture/pulls/7"=>pull,
            "repos/owned/fixture/pulls/7/files"=>if second {json!([{"filename":"second-page.bin","status":"added","additions":0,"deletions":0}])}else{json!([{"filename":"reviewed.txt","status":"modified","additions":620,"deletions":1,"patch":format!("@@ -1 +1,620 @@\n-original\n{}",(0..620).map(|i|format!("+Owned patch line {i} <script>literal</script>\n")).collect::<String>())}])},
            value if value.starts_with("repos/owned/fixture/commits/")&&value.ends_with("/check-runs")=>json!({"check_runs":[{"name":if second{"Owned second check"}else{"Owned head check"},"status":"completed","conclusion":"success"}]}),
            "repos/owned/fixture/actions/runs"=>json!({"workflow_runs":[{"id":55,"display_title":"Owned workflow","status":"completed","conclusion":"success"}]}),
            "repos/owned/fixture/actions/runs/55/jobs"=>json!({"jobs":[{"id":66,"name":"Owned job","status":"completed","conclusion":"success"}]}),
            "repos/owned/fixture/actions/runs/55/artifacts"=>json!({"artifacts":[{"id":77,"name":"Owned artifact","size_in_bytes":300000,"expired":false}]}),
            "repos/owned/fixture/releases"=>json!([{"id":88,"name":"Owned release","tag_name":"v-owned","body":"Owned release notes"}]),
            "repos/owned/fixture/releases/88/assets"=>{let mut rows=vec![json!({"id":99,"name":"owned-release.bin","size":300000})];rows.extend(self.uploads.lock().map_err(|_|ApiError::new("fixture","Fixture busy."))?.iter().cloned());json!(rows)},
            "graphql"=>graph(&op,&mut *self.graph_changes.lock().map_err(|_|ApiError::new("fixture","Graph fixture busy."))?),
            _=>json!([]),
        }};
        let next=if !second&&(base.ends_with("/files")||base.ends_with("/check-runs")||base.ends_with("/comments")){Some(format!("{base}?page=2&per_page=30"))}else{None};
        Ok(Reply{data,status:200,next,rate_remaining:Some(4999),rate_reset:None,rate_limit:None,rate_resource:None,outcome:if op["risk"]=="read" {Outcome::Read}else{Outcome::Applied}})
    }
}
fn graph(op:&serde_json::Value,changes:&mut serde_json::Value)->serde_json::Value {
    if op["risk"]!="read" { return graph_change(op,changes); }
    let vars=&op["body"]["variables"];let second=vars["after"].is_string();
    let page=|nodes:serde_json::Value,kind:&str|json!({"nodes":nodes,"pageInfo":{"hasNextPage":!second,"endCursor":if second{None}else{Some(format!("{kind}-cursor"))}}});
    let node=|field:&str,rows:serde_json::Value|{let mut value=json!({"id":vars["id"],"title":"Owned project"});value[field]=page(rows,field);json!({"node":value})};
    let mut data=match op["id"].as_str().unwrap_or("") {
        "projects.list"=>json!({"organization":{"login":vars["login"],"projectsV2":page(json!([{"id":"P_owned","title":"Owned project","number":1,"url":"https://github.com/orgs/owned/projects/1"}]),"projects")}}),
        "projects.user_list"=>json!({"user":{"login":vars["login"],"projectsV2":page(json!([{"id":"P_owned","title":"Owned project","number":1,"url":"https://github.com/users/owned/projects/1"}]),"projects")}}),
        "projects.get"=>json!({"node":{"id":vars["id"],"title":"Owned project","readme":"Owned graph detail <script>literal only</script>","closed":false}}),
        "projects.fields"=>node("fields",json!([{"id":if second{"F_second"}else{"F_status"},"__typename":"ProjectV2SingleSelectField","name":if second{"Second field"}else{"Status"},"dataType":"SINGLE_SELECT","options":[{"id":"O_done","name":"Done"}]}])),
        "projects.items"=>node("items",json!([{"id":"I_owned","content":{"title":if second{"Second owned task"}else{"Owned task"}}}])),
        "projects.item_values"=>node("fieldValues",json!([{"id":"V_status","__typename":"ProjectV2ItemFieldSingleSelectValue","name":if second{"Second value"}else{"Done"},"optionId":"O_done","field":{"id":"F_status","name":"Status"}},{"__typename":"ProjectV2ItemFieldUserValue","field":{"id":"F_people","name":"People"}},{"__typename":"ProjectV2ItemFieldLabelValue","field":{"id":"F_labels","name":"Labels"}},{"__typename":"ProjectV2ItemFieldPullRequestValue","field":{"id":"F_prs","name":"Linked PRs"}},{"__typename":"ProjectV2ItemFieldReviewerValue","field":{"id":"F_reviewers","name":"Reviewers"}}])),
        "projects.field_users"=>json!({"field":{"id":vars["fieldId"]},"node":{"id":vars["id"],"fieldValueByName":{"__typename":"ProjectV2ItemFieldUserValue","field":{"id":vars["fieldId"],"name":vars["fieldName"]},"values":page(json!([{"id":"N_owned","name":if second{"Second users value"}else{"First users value"}}]),"users")}}}),
        "projects.field_labels"=>json!({"field":{"id":vars["fieldId"]},"node":{"id":vars["id"],"fieldValueByName":{"__typename":"ProjectV2ItemFieldLabelValue","field":{"id":vars["fieldId"],"name":vars["fieldName"]},"values":page(json!([{"id":"N_owned","name":if second{"Second labels value"}else{"First labels value"}}]),"labels")}}}),
        "projects.field_pull_requests"=>json!({"field":{"id":vars["fieldId"]},"node":{"id":vars["id"],"fieldValueByName":{"__typename":"ProjectV2ItemFieldPullRequestValue","field":{"id":vars["fieldId"],"name":vars["fieldName"]},"values":page(json!([{"id":"N_owned","name":if second{"Second pull_requests value"}else{"First pull_requests value"}}]),"pull_requests")}}}),
        "projects.field_reviewers"=>json!({"field":{"id":vars["fieldId"]},"node":{"id":vars["id"],"fieldValueByName":{"__typename":"ProjectV2ItemFieldReviewerValue","field":{"id":vars["fieldId"],"name":vars["fieldName"]},"values":page(json!([{"id":"N_owned","name":if second{"Second reviewers value"}else{"First reviewers value"}}]),"reviewers")}}}),
        "discussions.list"=>json!({"repository":{"id":"R_owned","nameWithOwner":"owned/fixture","discussions":page(json!([{"id":"D_owned","number":7,"title":"Owned discussion","body":"Owned discussion body","url":"https://github.com/owned/fixture/discussions/7"}]),"discussions")}}),
        "discussions.get"=>json!({"repository":{"nameWithOwner":"owned/fixture","discussion":{"id":"D_owned","number":7,"title":"Owned discussion","body":"Literal <script>discussion</script>","closed":false,"comments":page(json!([{"id":"C_owned","body":if second{"Second comment"}else{"First comment"},"author":{"login":"owned-commenter"}}]),"comments")}}}),
        "discussions.categories"=>json!({"repository":{"nameWithOwner":"owned/fixture","discussionCategories":page(json!([{"id":"CAT_owned","name":if second{"Second category"}else{"Ideas"}}]),"categories")}}),
        "discussions.replies"=>json!({"node":{"id":vars["id"],"body":"Owned parent comment","replies":page(json!([{"id":"REPLY_owned","body":if second{"Second reply"}else{"First reply"},"author":{"login":"owned-replier"}}]),"replies")}}),
        _=>json!({"viewer":{"login":"owned-fixture"},"fixture":true}),
    };
    match op["id"].as_str().unwrap_or("") {
        "projects.get" => if let Some(fields)=changes["project"].as_object(){for(k,v)in fields{data["node"][k]=v.clone();}},
        "projects.items" => if let Some(item)=changes.get("draft"){data["node"]["items"]["nodes"].as_array_mut().unwrap().push(item.clone());},
        "projects.item_values" => {
            data["node"]["project"]=json!({"id":"P_owned","title":"Owned project"});data["node"]["isArchived"]=json!(changes["archived"]==true);
            if changes["fieldCleared"]==true {data["node"]["fieldValues"]["nodes"].as_array_mut().unwrap().retain(|v|v["field"]["id"]!="F_status");}
            else if let Some(text)=changes["fieldValue"]["text"].as_str(){data["node"]["fieldValues"]["nodes"][0]=json!({"id":"V_status","__typename":"ProjectV2ItemFieldTextValue","field":{"id":"F_status","name":"Status","dataType":"TEXT"},"text":text});}
        },
        "discussions.get" => {
            if let Some(fields)=changes["discussion"].as_object(){for(k,v)in fields{data["repository"]["discussion"][k]=v.clone();}}
            let rows=data["repository"]["discussion"]["comments"]["nodes"].as_array_mut().unwrap();
            rows[0]["isAnswer"]=json!(changes["answered"]==true);
            if let Some(body)=changes.get("commentBody"){rows[0]["body"]=body.clone();}
            if let Some(comment)=changes.get("newComment"){rows.push(comment.clone());}
        },
        "discussions.replies" => {data["node"]["discussion"]=json!({"id":"D_owned"});if let Some(reply)=changes.get("newReply"){data["node"]["replies"]["nodes"].as_array_mut().unwrap().push(reply.clone());}},
        _=>{},
    }
    data["rateLimit"]=json!({"cost":1,"remaining":999,"resetAt":"2026-09-10T23:59:00Z"});json!({"data":data})
}
fn graph_change(op:&serde_json::Value,changes:&mut serde_json::Value)->serde_json::Value {
    let input=&op["body"]["variables"]["input"];
    let mut node=json!({"id":"D_owned"});
    match op["id"].as_str().unwrap_or("") {
        "projects.update"=>{
            if !changes["project"].is_object(){changes["project"]=json!({"id":"P_owned"});}
            for(k,v)in input.as_object().unwrap(){if ["title","readme","shortDescription","closed","public"].contains(&k.as_str()){changes["project"][k]=v.clone();}}
            node=changes["project"].clone();
        },
        "projects.add_draft"=>{
            node=json!({"id":"I_draft","project":{"id":"P_owned"},"content":{"id":"DI_owned","title":input["title"],"body":input["body"]}});changes["draft"]=node.clone();
        },
        "projects.add_item"=>{node=json!({"id":"I_added","project":{"id":"P_owned"},"content":{"id":input["contentId"],"title":"Added content"}});changes["draft"]=node.clone();},
        "projects.update_field"|"projects.clear_field"=>{changes["fieldCleared"]=json!(op["id"]=="projects.clear_field");changes["fieldValue"]=input["value"].clone();node=json!({"id":input["itemId"],"project":{"id":input["projectId"]}});},
        "projects.archive"|"projects.unarchive"=>{changes["archived"]=json!(op["id"]=="projects.archive");node=json!({"id":input["itemId"],"project":{"id":input["projectId"]},"isArchived":changes["archived"]});},
        "discussions.update"=>{
            if !changes["discussion"].is_object(){changes["discussion"]=json!({"id":"D_owned"});}
            for(k,v)in input.as_object().unwrap(){if ["title","body"].contains(&k.as_str()){changes["discussion"][k]=v.clone();}}
            node=changes["discussion"].clone();
        },
        "discussions.close"|"discussions.reopen"=>{if !changes["discussion"].is_object(){changes["discussion"]=json!({});}changes["discussion"]["closed"]=json!(op["id"]=="discussions.close");node["closed"]=changes["discussion"]["closed"].clone();},
        "discussions.edit_comment"=>{changes["commentBody"]=input["body"].clone();node=json!({"id":input["commentId"],"body":input["body"],"discussion":{"id":"D_owned"}});},
        "discussions.answer"|"discussions.unanswer"=>{changes["answered"]=json!(op["id"]=="discussions.answer");node["answer"]=if changes["answered"]==true{json!({"id":input["id"]})}else{serde_json::Value::Null};},
        "discussions.comment"=>{node=json!({"id":"C_added","body":input["body"],"discussion":{"id":input["discussionId"]},"replyTo":if input["replyToId"].is_string(){json!({"id":input["replyToId"]})}else{serde_json::Value::Null},"author":{"login":"owned-new-comment"}});changes[if input["replyToId"].is_string(){"newReply"}else{"newComment"}]=node.clone();},
        _=>{},
    }
    json!({"data":{"result":{"node":node}}})
}
pub fn install(state:&Path)->std::result::Result<Option<GitHub>,String> {
    if std::env::var("AG1_VISUAL_GITHUB_FIXTURE").ok().as_deref()!=Some("owned-offline-v1") {return Ok(None);}
    // The isolated visual binary and its private candidate profile are required;
    // the normal build has neither this environment hook nor this module.
    let service=Service::compose(state,Arc::new(Fixture{workflows:Mutex::new(json!({})),secrets:Mutex::new(Vec::new()),root:state.into(),serial:Mutex::new(0),item_changes:Mutex::new((json!({}),json!({}))),graph_changes:Mutex::new(json!({})),uploads:Mutex::new(Vec::new())})).map_err(|e|e.message.to_string())?;
    std::fs::write(state.join("github-connected"),b"owned-offline-github-fixture-v1").map_err(|_|"Cannot prepare owned GitHub fixture".to_string())?;
    Ok(Some(GitHub::with_service(service)))
}
