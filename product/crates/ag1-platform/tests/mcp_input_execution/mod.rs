use super::*;
use ag1_platform::{mcp::input::{Answer,Entry,Form},resource_fs::ScanControl};
struct OwnedInput { mode:&'static str, seen:AtomicUsize, owner:std::sync::Mutex<Option<(std::sync::Weak<Manager>,String)>> }
impl Entry for OwnedInput {
    fn enter(&self,form:&Form,control:&ScanControl<'_>)->Result<Answer,DomainError>{
        control.check()?;self.seen.fetch_add(1,Ordering::SeqCst);
        assert_eq!(form.identity.original_name,"owned");assert_eq!(form.original_arguments,json!({"value":1}));assert_eq!(form.fields.len(),2);
        assert!(!format!("{form:?}").contains("owned-opaque-input-state"));
        if let Some((manager,id))=&*self.owner.lock().unwrap(){let manager=manager.upgrade().unwrap();assert!(manager.waiting_for_input(id).unwrap());manager.disconnect(id).unwrap();}
        Ok(match self.mode{"decline"=>Answer::Decline,"cancel"=>Answer::Cancel,_=>Answer::Accept(BTreeMap::from([("choice".into(),json!("brief")),("includeResolved".into(),json!(false))]))})
    }
}
fn input(mode:&'static str)->Arc<OwnedInput>{Arc::new(OwnedInput{mode,seen:AtomicUsize::new(0),owner:std::sync::Mutex::new(None)})}
#[test]
fn form_continuation_accept_decline_cancel_and_unsupported_schema_use_one_owned_operation(){
    for transport in ["stdio","http"] {
        for (mode,answer,calls,questions) in [("form_accept","accept",2,1),("form_decline","decline",2,1),("form_cancel","cancel",1,1),("form_bad","accept",1,0)]{
            let entry=input(answer);let case=Case::new_with_input(transport,Revision::Current20260728,mode,Some(entry.clone()));
            let ticket=case.ticket("input-op",10000,&json!({"value":1}));
            let outcome=case.execute("input-op",ticket,|event|{if event.stage=="start"{assert!(case.calls().is_empty());assert_eq!(executor::recover(&case.store,"workspace","session","input-op").unwrap().unwrap().status,Status::Intent);}}).unwrap();
            assert_eq!(case.calls().len(),calls,"{transport}/{mode}: {:?}",outcome.record);
            assert_eq!(entry.seen.load(Ordering::SeqCst),questions);
            assert!(!outcome.result.to_string().contains("owned-opaque-input-state"));
            assert_eq!(case.dir.path().join("effects").exists(),answer=="accept"&&mode!="form_bad");
            if mode=="form_accept"{
                assert_eq!(outcome.record.status,Status::Completed);
                let frames=case.calls();assert!(frames[0]["params"].get("requestState").is_none());assert_eq!(frames[1]["params"]["requestState"],"owned-opaque-input-state");
                assert_eq!(frames[0]["params"]["arguments"],frames[1]["params"]["arguments"]);
                assert_ne!(frames[0]["id"],frames[1]["id"]);
            }
        }
    }
}
#[test]
fn form_round_cap_and_revocation_prevent_unreviewed_continuation(){
    for (transport,mode,revoke,expected_calls,questions) in [("stdio","form_many",false,4,3),("http","form_revoke",true,1,1)]{
        let entry=input("accept");let case=Case::new_with_input(transport,Revision::Current20260728,mode,Some(entry.clone()));
        if revoke{*entry.owner.lock().unwrap()=Some((Arc::downgrade(&case.manager),case.connection.clone()));}
        let outcome=case.execute("input-op",case.ticket("input-op",10000,&json!({"value":1})),|_|{}).unwrap();
        assert_eq!(case.calls().len(),expected_calls,"{:?}",outcome.record);assert_eq!(entry.seen.load(Ordering::SeqCst),questions);assert!(!case.dir.path().join("effects").exists());
        assert!(!outcome.result.to_string().contains("owned-opaque-input-state"));
    }
}
