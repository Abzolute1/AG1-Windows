//! Ephemeral native download custody. Bytes and signed URLs never enter ordinary
//! API replies. A random receipt is useful only in its original account/scope.
use super::{operations::{Operation, Request, Risk}, service::{Account, ApiError, Outcome, Reply, Result}};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use std::{collections::BTreeMap,time::{Duration,Instant}};
use base64::Engine;
pub const MAX_BYTES:usize=64*1024*1024;
const TOTAL_BYTES:usize=128*1024*1024;
pub const PAGE_BYTES:usize=256*1024;
/// Host-only payload; intentionally no serialization or Debug implementation.
pub struct Payload { pub bytes:Vec<u8> }
impl Drop for Payload {fn drop(&mut self){self.bytes.fill(0);}}
pub(super) fn remote(id:&str)->bool {matches!(id,"actions.download_artifact"|"actions.download_logs"|"actions.download_job_log"|"releases.download_asset")}
pub(super) fn local(request:&Request)->std::result::Result<Option<Operation>,String>{
    if request.operation!="downloads.read" {return Ok(None);}
    let p=&request.parameters;
    if request.repo.is_some() || !request.body.is_null() || p.keys().any(|k|!["id","offset","length"].contains(&k.as_str()))
        || !p.get("id").and_then(Value::as_str).is_some_and(|s|s.starts_with("download-")&&s.len()<=128&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'))
        || !p.get("offset").and_then(Value::as_u64).is_some_and(|n|n<=MAX_BYTES as u64)
        || p.get("length").is_some_and(|v|!v.as_u64().is_some_and(|n|(1..=PAGE_BYTES as u64).contains(&n))) {return Err("Select a native download receipt and bounded byte page; omit repo and body.".into());}
    Ok(Some(Operation{id:request.operation.clone(),method:"LOCAL".into(),endpoint:"native/download".into(),body:Some(json!(p)),risk:Risk::Read,repo:None}))
}
pub(super) fn schema(id:&str)->Option<Value>{
    if id=="downloads.read" {return Some(json!({"id":id,"parameters":{"id":"Native download receipt ID","offset":"Byte offset (required)","length":"1–262144 bytes; default 65536"},"limits":"Original account and conversation only; retained at most 15 minutes in memory; unavailable after disconnect or restart."}));}
    remote(id).then(||json!({"id":id,"parameters":match id {"actions.download_artifact"=>json!({"artifact_id":"Positive artifact ID"}),"actions.download_logs"=>json!({"run_id":"Positive run ID"}),"actions.download_job_log"=>json!({"job_id":"Positive job ID"}),_=>json!({"asset_id":"Positive release asset ID"})},"repo":"owner/name","limits":"64 MiB per file, 128 MiB and 32 retained downloads per service. Receipt only; use downloads.read for exact bounded byte pages. Archives are never extracted or executed."}))
}
struct Retained {account:Account,generation:Vec<u8>,scope:String,expires:Instant,payload:Payload,view:Value}
#[derive(Default)]
pub(super) struct Store {items:BTreeMap<String,Retained>}
impl Store {
    pub fn clear(&mut self){self.items.clear();}
    fn prune(&mut self,account:&Account,generation:&[u8]) {self.items.retain(|_,v|v.expires>Instant::now()&&v.account==*account&&v.generation==generation);}
    pub fn capacity(&mut self,account:&Account,generation:&[u8])->Result<()> {
        self.prune(account,generation);
        if self.items.len()>=32 || self.items.values().map(|v|v.payload.bytes.len()).sum::<usize>()>TOTAL_BYTES-MAX_BYTES {return Err(ApiError::new("download_quota","Retained downloads reached the private memory limit. Wait for expiry or disconnect to clear them."));}Ok(())
    }
    pub fn retain(&mut self,account:&Account,generation:&[u8],scope:&str,op:&Operation,payload:Payload)->Result<Reply>{
        self.capacity(account,generation)?;
        if payload.bytes.len()>MAX_BYTES{return Err(ApiError::new("download_limit","GitHub download exceeds 64 MiB."));}
        let id=format!("download-{}",crate::client::nonce().map_err(|_|ApiError::new("unavailable","Cannot retain this download."))?);
        let extension=if op.id=="actions.download_job_log"{"txt"}else if op.id=="releases.download_asset"{"bin"}else{"zip"};
        let number=op.endpoint.split('/').rev().find(|s|s.bytes().all(|b|b.is_ascii_digit())).unwrap_or("file");
        let name=format!("{}-{number}.{extension}",op.id.replace('.',"-"));
        let mime=if extension=="zip"{"application/zip"}else if extension=="txt"{"text/plain"}else{"application/octet-stream"};
        let view=json!({"id":id,"name":name,"bytes":payload.bytes.len(),"sha256":format!("{:x}",Sha256::digest(&payload.bytes)),"mime":mime,"operation":op.id,"repo":op.repo,"expires_in_seconds":900});
        self.items.insert(id,Retained{account:account.clone(),generation:generation.into(),scope:scope.into(),expires:Instant::now()+Duration::from_secs(900),payload,view:view.clone()});
        Ok(reply(json!({"download":view})))
    }
    pub fn page(&mut self,account:&Account,generation:&[u8],scope:&str,op:&Operation)->Result<Reply>{
        self.prune(account,generation);
        let p=op.body.as_ref().ok_or_else(missing)?;
        let retained=self.items.get(p["id"].as_str().unwrap_or("")).filter(|v|v.scope==scope).ok_or_else(missing)?;
        let offset=p["offset"].as_u64().ok_or_else(missing)? as usize;
        let length=p["length"].as_u64().unwrap_or(65536) as usize;
        if offset>retained.payload.bytes.len()||length==0||length>PAGE_BYTES{return Err(missing());}
        let end=offset.saturating_add(length).min(retained.payload.bytes.len());
        Ok(reply(json!({"download":retained.view,"offset":offset,"next_offset":end,"complete":end==retained.payload.bytes.len(),"base64":base64::engine::general_purpose::STANDARD.encode(&retained.payload.bytes[offset..end])})))
    }
}
fn missing()->ApiError {ApiError::new("download_unavailable","Download expired, belongs to another account or conversation, or has an invalid byte range. Download it again in this conversation.")}
fn reply(data:Value)->Reply {Reply{data,status:200,next:None,rate_remaining:None,rate_reset:None,rate_resource:None,rate_limit:None,outcome:Outcome::Read}}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{github::service::{Service,Transport,check},resource_fs::ScanControl};
    use std::sync::{Arc,Mutex,atomic::{AtomicBool,AtomicUsize,Ordering}};
    struct Fixture {account:Mutex<Account>,calls:AtomicUsize,revoke:Option<std::path::PathBuf>}
    impl Transport for Fixture {
        fn account(&self,c:&ScanControl<'_>)->Result<Account>{check(c)?;Ok(self.account.lock().unwrap().clone())}
        fn execute(&self,_:&Account,_:&Operation,_:&ScanControl<'_>)->Result<Reply>{panic!("No generic JSON route for binary data")}
        fn download(&self,a:&Account,_:&Operation,c:&ScanControl<'_>)->Result<Payload>{check(c)?;if *self.account.lock().unwrap()!=*a{return Err(ApiError::new("stale_account","Changed"));}self.calls.fetch_add(1,Ordering::Relaxed);if let Some(path)=&self.revoke{std::fs::remove_file(path).unwrap();}Ok(Payload{bytes:vec![0,255,13,10,65]})}
    }
    fn request(value:Value)->Request{serde_json::from_value(value).unwrap()}
    #[test]fn downloads_share_native_scope_confirmation_and_exact_binary_custody(){
        for revoke in [false,true] {
            let dir=tempfile::tempdir().unwrap();let marker=dir.path().join("github-connected");std::fs::write(&marker,b"generation").unwrap();
            let account=Account::bound(7,"owned".into(),"cred-1".into()).unwrap();
            let fixture=Arc::new(Fixture{account:Mutex::new(account),calls:AtomicUsize::new(0),revoke:revoke.then_some(marker.clone())});
            let service=Service::new(dir.path().into(),fixture.clone());let cancel=AtomicBool::new(false);let control=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(10)};
            let download=||request(json!({"operation":"actions.download_artifact","repo":"owned/repo","parameters":{"artifact_id":12}}));
            let preview=service.preview(download(),"scope-1",&control).unwrap();assert_eq!(fixture.calls.load(Ordering::Relaxed),0);
            let confirmation=preview["confirmation"].as_str().unwrap();let result=service.confirm(confirmation,"scope-1",&control);
            assert!(service.confirm(confirmation,"scope-1",&control).is_err());assert_eq!(fixture.calls.load(Ordering::Relaxed),1);
            if revoke {assert!(result.is_err());continue;}
            let result=result.unwrap();let view=result.data["download"].clone();assert!(!result.data.to_string().contains("base64"));assert_eq!(view["bytes"],5);
            let page=|offset,length|request(json!({"operation":"downloads.read","parameters":{"id":view["id"],"offset":offset,"length":length}}));
            assert!(service.read(vec![page(0,3)],"scope-2",&control).is_err());
            let first=service.read(vec![page(0,3)],"scope-1",&control).unwrap().remove(0);assert_eq!(first.data["base64"],"AP8N");assert_eq!(first.data["complete"],false);
            let second=service.read(vec![page(3,3)],"scope-1",&control).unwrap().remove(0);assert_eq!(second.data["base64"],"CkE=");assert_eq!(second.data["complete"],true);
            assert!(service.read(vec![page(6,3)],"scope-1",&control).is_err());
            let plan=service.prepare(page(0,1),"scope-1",&control).unwrap();fixture.account.lock().unwrap().credential_revision="cred-2".into();assert_eq!(service.execute(plan,"scope-1",&control).unwrap_err().code,"stale_account");
            assert!(service.read(vec![page(0,1)],"scope-1",&control).is_err());
            let new=service.read(vec![download()],"scope-1",&control).unwrap().remove(0);let new_id=new.data["download"]["id"].clone();
            cancel.store(true,Ordering::Release);assert!(service.read(vec![download()],"scope-1",&control).is_err());cancel.store(false,Ordering::Release);
            service.disconnect().unwrap();std::fs::write(marker,b"generation-2").unwrap();assert!(service.read(vec![request(json!({"operation":"downloads.read","parameters":{"id":new_id,"offset":0}}))],"scope-1",&control).is_err());
        }
    }
    #[test]fn retention_quotas_expiry_and_arguments_are_bounded(){
        let account=Account::bound(7,"owned".into(),"credential".into()).unwrap();let generation=b"g";let mut store=Store::default();
        let op=request(json!({"operation":"actions.download_logs","repo":"owned/repo","parameters":{"run_id":1}})).compile().unwrap();
        for _ in 0..32 {store.retain(&account,generation,"s",&op,Payload{bytes:vec![]}).unwrap();}
        assert_eq!(store.capacity(&account,generation).unwrap_err().code,"download_quota");
        for value in store.items.values_mut(){value.expires=Instant::now()-Duration::from_secs(1);}
        store.capacity(&account,generation).unwrap();assert!(store.items.is_empty());
        store.retain(&account,generation,"s",&op,Payload{bytes:vec![0;MAX_BYTES]}).unwrap();store.retain(&account,generation,"s",&op,Payload{bytes:vec![1;MAX_BYTES]}).unwrap();assert!(store.capacity(&account,generation).is_err());store.clear();
        for bad in [json!({"operation":"downloads.read","repo":"owned/repo","parameters":{"id":"download-owned","offset":0}}),json!({"operation":"downloads.read","parameters":{"id":"download-owned","offset":0,"length":PAGE_BYTES+1}}),json!({"operation":"actions.download_logs","repo":"owned/repo","parameters":{"run_id":0}}),json!({"operation":"actions.download_artifact","repo":"owned/repo","parameters":{"artifact_id":1,"url":"https://evil.test"}})] {assert!(request(bad).compile().is_err());}
    }
}
