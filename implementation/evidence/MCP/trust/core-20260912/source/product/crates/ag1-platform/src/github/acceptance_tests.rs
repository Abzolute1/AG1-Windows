//! Cross-area authority/negative matrix. Positive semantics and real HTTP
//! parsing are covered by the feature and owned TLS fixtures respectively.
use super::{operations::{Operation,Request},service::{Account,ApiError,Outcome,Reply,Result,Service,Transport,check}};
use crate::resource_fs::ScanControl;
use serde_json::{Value,json};
use std::sync::{Arc,Mutex,Weak,atomic::{AtomicBool,AtomicUsize,Ordering}};
use std::time::{Duration,Instant};
fn cases()->Vec<Request>{
    [
        json!({"operation":"repos.create","body":{"name":"owned-created","private":true}}),
        json!({"operation":"issues.create","repo":"owned/repo","body":{"title":"Owned"}}),
        json!({"operation":"pulls.review","repo":"owned/repo","parameters":{"number":7},"body":{"commit_id":"a".repeat(40),"event":"COMMENT","body":"Owned"}}),
        json!({"operation":"pulls.merge","repo":"owned/repo","parameters":{"number":7},"body":{"sha":"a".repeat(40)}}),
        json!({"operation":"actions.dispatch","repo":"owned/repo","parameters":{"workflow_id":44},"body":{"ref":"main"}}),
        json!({"operation":"releases.create","repo":"owned/repo","body":{"tag_name":"v-owned","draft":true}}),
        json!({"operation":"discussions.create","parameters":{"input":{"repositoryId":"R","categoryId":"CAT","title":"Owned","body":"Owned"}}}),
        json!({"operation":"projects.add_item","parameters":{"input":{"projectId":"P","contentId":"ISSUE"}}}),
        json!({"operation":"notifications.read","parameters":{"thread_id":"70"}}),
        json!({"operation":"api.rest","parameters":{"method":"PATCH","endpoint":"repos/owned/repo"},"body":{"description":"Owned administration"}}),
        json!({"operation":"api.graphql","body":{"query":"query { viewer { login } }"}}),
    ].into_iter().map(|v|serde_json::from_value(v).unwrap()).collect()
}
struct Fixture{account:Mutex<Account>,sent:Mutex<Vec<Value>>,mode:AtomicUsize,service:Mutex<Weak<Service>>}
impl Transport for Fixture{
    fn account(&self,c:&ScanControl<'_>)->Result<Account>{check(c)?;Ok(self.account.lock().unwrap().clone())}
    fn execute(&self,a:&Account,op:&Operation,c:&ScanControl<'_>)->Result<Reply>{
        if self.account(c)?!=*a{return Err(ApiError::new("stale_account","Account changed."));}
        self.sent.lock().unwrap().push(serde_json::to_value(op).unwrap());
        match self.mode.load(Ordering::Acquire){
            0|1=>{let rate=self.mode.load(Ordering::Acquire)==1;let mut e=ApiError::new(if rate{"rate_limit"}else{"permission"},"Owned rejection.");e.status=Some(if rate{429}else{403});if rate{e.retry_after_seconds=Some(60);e.retry_at_epoch_seconds=Some(2000000000);}Err(e)},
            2=>Err(ApiError::new("interrupted","Owned interrupted write.").uncertain(op)),
            3=>{self.service.lock().unwrap().upgrade().unwrap().cancel();Ok(reply())},
            4=>{self.service.lock().unwrap().upgrade().unwrap().disconnect().unwrap();Ok(reply())},
            _=>Ok(reply()),
        }
    }
}
fn reply()->Reply{Reply{data:json!({"private":"must not escape cancelled operation"}),status:200,next:None,rate_remaining:None,rate_reset:None,rate_resource:None,rate_limit:None,outcome:Outcome::Applied}}
fn setup(mode:usize)->(tempfile::TempDir,Arc<Service>,Arc<Fixture>){
    let dir=tempfile::tempdir().unwrap();std::fs::write(dir.path().join("github-connected"),b"owned-generation").unwrap();
    let f=Arc::new(Fixture{account:Mutex::new(Account::bound(1,"owned".into(),"owned-credential".into()).unwrap()),sent:Mutex::new(vec![]),mode:AtomicUsize::new(mode),service:Mutex::new(Weak::new())});
    let service=Arc::new(Service::new(dir.path().into(),f.clone()));*f.service.lock().unwrap()=Arc::downgrade(&service);(dir,service,f)
}
#[test]fn every_remote_matrix_area_preserves_permission_rate_cancellation_and_uncertain_no_replay(){
    for request in cases(){for mode in 0..5{
        let (_dir,service,f)=setup(mode);let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
        let exact=serde_json::to_value(request.compile().unwrap()).unwrap();
        let preview=service.preview(request.clone(),"scope",&c).unwrap_or_else(|e|panic!("{} {}",request.operation,e.message));assert!(f.sent.lock().unwrap().is_empty());let id=preview["confirmation"].as_str().unwrap();
        let error=service.confirm(id,"scope",&c).unwrap_err();assert_eq!(error.outcome,if mode<2{Outcome::NotSent}else{Outcome::Unknown},"{} mode {mode}",request.operation);
        if mode==1{assert_eq!(error.status,Some(429));assert_eq!(error.retry_after_seconds,Some(60));assert_eq!(error.retry_at_epoch_seconds,Some(2000000000));}
        assert!(!serde_json::to_string(&error).unwrap().contains("must not escape"));assert!(service.confirm(id,"scope",&c).is_err());assert_eq!(*f.sent.lock().unwrap(),vec![exact]);
    }}
}
#[test]fn every_remote_matrix_area_rejects_changed_scope_account_credential_generation_and_early_cancel(){
    for request in cases(){for change in 0..5{
        let (dir,service,f)=setup(2);let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
        let preview=service.preview(request.clone(),"scope",&c).unwrap();let id=preview["confirmation"].as_str().unwrap();
        match change{0=>{},1=>f.account.lock().unwrap().id+=1,2=>f.account.lock().unwrap().credential_revision="other".into(),3=>std::fs::write(dir.path().join("github-connected"),b"changed-generation").unwrap(),_=>cancel.store(true,Ordering::Release)};
        assert_eq!(service.confirm(id,if change==0{"other"}else{"scope"},&c).unwrap_err().outcome,Outcome::NotSent);cancel.store(false,Ordering::Release);
        assert!(service.confirm(id,"scope",&c).is_err());assert!(f.sent.lock().unwrap().is_empty(),"{} change {change}",request.operation);
    }}
}
