//! Everyday Actions/release/inbox responses must confirm their exact targets.
use super::{operations::Operation,service::{ApiError,Reply,Result}};
use serde_json::Value;
fn repository(s:&str)->bool{s.split('/').count()==2&&s.split('/').all(|p|!p.is_empty()&&p.len()<=100&&p!="."&&p!=".."&&!p.starts_with('-')&&p.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_.".contains(&b)))}
fn positive(v:&Value)->bool{v.as_u64().is_some_and(|n|n>0&&n<=9_007_199_254_740_991)}
fn rows(v:&Value,key:Option<&str>,item:impl Fn(&Value)->bool)->bool{let v=key.map_or(v,|k|&v[k]);v.as_array().is_some_and(|a|a.len()<=100&&a.iter().all(item))}
fn named(v:&Value)->bool{positive(&v["id"])&&v["name"].is_string()}
pub(super) fn validate(op:&Operation,reply:&Reply)->Result<()> {
 let path=op.endpoint.split('?').next().unwrap();let target=path.rsplit('/').next().unwrap();let d=&reply.data;
 let exact=||d["id"].as_u64().is_some_and(|id|target.parse::<u64>()==Ok(id));
 let release=||positive(&d["id"])&&(d["name"].is_string()||d["name"].is_null())&&d["tag_name"].as_str().is_some_and(|s|!s.is_empty());
 let fields=||op.body.as_ref().and_then(Value::as_object).is_none_or(|body|body.iter().filter(|(k,_)|["name","tag_name","body","draft","prerelease"].contains(&k.as_str())).all(|(k,v)|d.get(k)==Some(v)));
 let okay=match op.id.as_str(){
  "actions.workflow"=>reply.status==200&&named(d)&&(exact()||d["path"].as_str().is_some_and(|p|p.rsplit('/').next()==Some(target))),
  "actions.run"|"actions.job"=>reply.status==200&&exact()&&d["status"].is_string(),
  "actions.workflows"=>reply.status==200&&rows(d,Some("workflows"),named),
  "actions.runs"|"actions.workflow_runs"=>reply.status==200&&rows(d,Some("workflow_runs"),|r|positive(&r["id"])&&r["status"].is_string()),
  "actions.jobs"=>reply.status==200&&rows(d,Some("jobs"),|r|named(r)&&r["status"].is_string()),
  "actions.artifacts"=>reply.status==200&&rows(d,Some("artifacts"),named),
  "actions.dispatch"=>reply.status==200&&positive(&d["workflow_run_id"])&&op.repo.as_ref().is_some_and(|repo|d["run_url"].as_str().is_some_and(|url|url.eq_ignore_ascii_case(&format!("https://api.github.com/repos/{repo}/actions/runs/{}",d["workflow_run_id"])))&&d["html_url"].as_str().is_some_and(|url|url.eq_ignore_ascii_case(&format!("https://github.com/{repo}/actions/runs/{}",d["workflow_run_id"])))),
  "actions.rerun"=>reply.status==201,
  "actions.cancel"=>reply.status==202,
  "releases.get"=>reply.status==200&&exact()&&release(),
  "releases.create"=>reply.status==201&&release()&&fields(),
  "releases.update"=>reply.status==200&&exact()&&release()&&fields(),
  "notifications.list"|"notifications.repository_list"=>reply.status==200&&rows(d,None,|r|r["id"].as_str().is_some_and(|s|!s.is_empty()&&s.len()<=128&&s.bytes().all(|b|b.is_ascii_digit()))&&r["unread"].is_boolean()&&r["subject"]["title"].as_str().is_some_and(|s|s.len()<=10000)&&r["repository"]["full_name"].as_str().is_some_and(|s|repository(s)&&(op.id!="notifications.repository_list"||op.repo.as_deref().is_some_and(|repo|repo.eq_ignore_ascii_case(s))))),
  "notifications.read"=>reply.status==205&&d.is_null(),
  "notifications.mark_all"|"notifications.mark_repository"=>[202,205].contains(&reply.status),
  _=>return Ok(()),
 };
 if okay{Ok(())}else{Err(ApiError::new("workflow_response","GitHub returned an unexpected workflow, release or notification response. Refresh its state before another change.").uncertain(op))}
}
#[cfg(test)]
mod tests {
 use super::*;use super::super::{operations::Request,service::{Service,Transport,Account,Outcome,check}};
 use serde_json::json;use std::{sync::{Arc,atomic::{AtomicBool,AtomicUsize,Ordering}},time::{Instant,Duration}};use crate::resource_fs::ScanControl;
 fn request(id:&str,params:Value,body:Value)->Request{serde_json::from_value(json!({"operation":id,"repo":if id.starts_with("notifications.")&&!id.contains("repository"){None}else{Some("owned/repo")},"parameters":params,"body":body})).unwrap()}
 fn reply(data:Value,status:u16)->Reply{Reply{data,status,next:None,rate_remaining:None,rate_reset:None,rate_resource:None,rate_limit:None,outcome:Outcome::Applied}}
 #[test]fn workflow_release_and_inbox_targets_validate_schema_status_and_semantics(){
  let cases=[
   (request("actions.workflow",json!({"workflow_id":44}),Value::Null),reply(json!({"id":44,"name":"Owned","path":".github/workflows/owned.yml"}),200)),
   (request("actions.workflow_runs",json!({"workflow_id":44,"page":2,"per_page":30}),Value::Null),reply(json!({"workflow_runs":[{"id":55,"status":"queued"}]}),200)),
   (request("actions.run",json!({"run_id":55}),Value::Null),reply(json!({"id":55,"status":"queued"}),200)),
   (request("actions.job",json!({"job_id":66}),Value::Null),reply(json!({"id":66,"status":"completed"}),200)),
   (request("actions.dispatch",json!({"workflow_id":"owned.yml"}),json!({"ref":"main","inputs":{"owned":true}})),reply(json!({"workflow_run_id":55,"run_url":"https://api.github.com/repos/owned/repo/actions/runs/55","html_url":"https://github.com/owned/repo/actions/runs/55"}),200)),
   (request("actions.rerun",json!({"run_id":55}),Value::Null),reply(Value::Null,201)),
   (request("actions.cancel",json!({"run_id":55}),Value::Null),reply(Value::Null,202)),
   (request("releases.create",json!({}),json!({"tag_name":"v-owned","draft":true})),reply(json!({"id":88,"name":null,"tag_name":"v-owned","draft":true}),201)),
   (request("releases.update",json!({"release_id":88}),json!({"name":"Owned","body":"notes","draft":false})),reply(json!({"id":88,"name":"Owned","tag_name":"v-owned","body":"notes","draft":false}),200)),
   (request("notifications.repository_list",json!({"page":2,"per_page":30,"all":true,"participating":true}),Value::Null),reply(json!([{"id":"70","unread":true,"repository":{"full_name":"owned/repo"},"subject":{"title":"Owned"}}]),200)),
   (request("notifications.read",json!({"thread_id":"70"}),Value::Null),reply(Value::Null,205)),
   (request("notifications.mark_all",json!({}),json!({"read":true,"last_read_at":"2026-09-10T12:00:00Z"})),reply(json!({"message":"Processing"}),202)),
   (request("notifications.mark_repository",json!({}),json!({"read":true})),reply(Value::Null,205)),
  ];
  for (r,good)in cases{let op=r.compile().unwrap();super::super::schemas::validate(&op).unwrap_or_else(|e|panic!("{} {e}",r.operation));validate(&op,&good).unwrap_or_else(|e|panic!("{} {}",r.operation,e.message));let mut wrong=good.clone();wrong.status=299;assert!(validate(&op,&wrong).is_err());}
  for fields in [json!({"ref":false}),json!({"ref":" "}),json!({"ref":"main","inputs":{"bad":{}}}),json!({"ref":"main","inputs":(0..26).map(|n|(format!("k{n}"),json!(true))).collect::<serde_json::Map<_,_>>()})]{assert!(request("actions.dispatch",json!({"workflow_id":44}),fields).compile().is_err());}
  let op=request("actions.dispatch",json!({"workflow_id":44}),json!({"ref":"main"})).compile().unwrap();let wrong=reply(json!({"workflow_run_id":55,"run_url":"https://evil.invalid/token","html_url":"https://github.com/other/repo/actions/runs/55"}),200);assert_eq!(validate(&op,&wrong).unwrap_err().outcome,Outcome::Unknown);
  let op=request("releases.update",json!({"release_id":88}),json!({"draft":false})).compile().unwrap();assert!(validate(&op,&reply(json!({"id":88,"tag_name":"v1","name":"Owned","draft":true}),200)).is_err());
  let mut r=request("notifications.read",json!({"thread_id":"70"}),Value::Null);r.repo=Some("owned/repo".into());assert!(r.compile().is_err());
 }
 struct Fixture{writes:AtomicUsize,malformed:bool}
 impl Transport for Fixture{
  fn account(&self,c:&ScanControl<'_>)->Result<Account>{check(c)?;Account::bound(1,"owned".into(),"cred".into())}
  fn execute(&self,a:&Account,op:&Operation,c:&ScanControl<'_>)->Result<Reply>{assert!(self.account(c)?==*a);self.writes.fetch_add(1,Ordering::AcqRel);Ok(if self.malformed{reply(json!({"ok":true}),200)}else{match op.id.as_str(){"actions.cancel"=>reply(Value::Null,202),"notifications.read"=>reply(Value::Null,205),"releases.update"=>reply(json!({"id":88,"name":"Owned","tag_name":"v1","draft":false}),200),_=>panic!()}})}
 }
 #[test]fn everyday_changes_require_one_use_approval_and_malformed_replies_remain_uncertain(){
  for r in [request("actions.cancel",json!({"run_id":55}),Value::Null),request("notifications.read",json!({"thread_id":"70"}),Value::Null),request("releases.update",json!({"release_id":88}),json!({"draft":false}))]{for malformed in [false,true]{let dir=tempfile::tempdir().unwrap();std::fs::write(dir.path().join("github-connected"),b"owned").unwrap();let f=Arc::new(Fixture{writes:AtomicUsize::new(0),malformed});let service=Service::new(dir.path().into(),f.clone());let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};let preview=service.preview(r.clone(),"s",&c).unwrap();assert_eq!(f.writes.load(Ordering::Acquire),0);let id=preview["confirmation"].as_str().unwrap();let result=service.confirm(id,"s",&c);if malformed{assert_eq!(result.unwrap_err().outcome,Outcome::Unknown);}else{assert_eq!(result.unwrap().outcome,Outcome::Applied);}assert!(service.confirm(id,"s",&c).is_err());assert_eq!(f.writes.load(Ordering::Acquire),1);}}
 }
}
