//! Native preview/consume authority shared by UI and agent adapters.
//! Prepared requests cannot be deserialized or altered by either caller.
use super::operations::{Operation, Request, Risk};
use crate::resource_fs::ScanControl;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex}, time::{Duration, Instant}};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome { NotSent, Read, Applied, Unknown }
#[derive(Clone, Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    pub status: Option<u16>,
    pub retry_after_seconds: Option<u64>,
    pub retry_at_epoch_seconds: Option<u64>,
    pub outcome: Outcome,
}
impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message:message.into(), status: None, retry_after_seconds: None, retry_at_epoch_seconds: None, outcome: Outcome::NotSent }
    }
    pub fn uncertain(mut self, operation: &Operation) -> Self {
        self.outcome = if operation.risk == Risk::Read { Outcome::Read } else { Outcome::Unknown }; self
    }
}
pub type Result<T> = std::result::Result<T, ApiError>;
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub id: u64,
    pub login: String,
    // Credential fingerprint is native-only. It invalidates an approval when
    // gh changes active accounts or credentials, without exporting the secret.
    pub(crate) credential_revision: String,
}
impl Account {
    pub fn public(&self) -> Value { json!({"id":self.id,"login":self.login,"host":"github.com"}) }
    /// A host-supplied transport binds its private credential incarnation here.
    /// This constructor does not connect or grant account access on its own.
    pub fn bound(id:u64,login:String,credential_revision:String)->Result<Self> {
        if id==0 || login.is_empty() || login.len()>128 || !login.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-') || credential_revision.is_empty() || credential_revision.len()>256 { return Err(ApiError::new("account","Invalid native account binding.")); }
        Ok(Self{id,login,credential_revision})
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Reply {
    pub data: Value,
    pub status: u16,
    pub next: Option<String>,
    pub rate_remaining: Option<u64>,
    pub rate_reset: Option<u64>,
    pub rate_resource: Option<String>,
    pub rate_limit: Option<u64>,
    pub outcome: Outcome,
}
pub trait Transport: Send + Sync {
    fn set_secret(&self,_account:&Account,_operation:&Operation,_entry:&dyn super::secrets::PrivateEntry,_control:&ScanControl<'_>)->Result<Reply>{Err(ApiError::new("unsupported","This native transport does not provide private secret entry."))}
    fn account(&self, control: &ScanControl<'_>) -> Result<Account>;
    /// Must bind the same credentials used to verify `account` to the request.
    /// Do not automatically replay a request after any uncertain send.
    fn execute(&self, account: &Account, operation: &Operation, control: &ScanControl<'_>) -> Result<Reply>;
    fn upload(&self,_account:&Account,_operation:&Operation,_upload:&super::uploads::Upload,_control:&ScanControl<'_>)->Result<Reply> { Err(ApiError::new("unsupported","This native transport does not provide uploads.")) }
    fn download(&self,_account:&Account,_operation:&Operation,_control:&ScanControl<'_>)->Result<super::downloads::Payload> { Err(ApiError::new("unsupported","This native transport does not provide downloads.")) }
    fn git_credential(&self,_account:&Account,_control:&ScanControl<'_>)->Result<super::transport::GitCredential> {
        Err(ApiError::new("unsupported","This native transport does not provide Git credentials."))
    }
}
/// Native-only account admission for a separately retained exact local Git
/// plan. This is not an API request and cannot be passed to API confirmation.
pub(crate) struct GitAdmission {
    account:Account,
    generation:Vec<u8>,
    scope:String,
    expires:Instant,
}
impl GitAdmission { pub(crate) fn account(&self)->Value {self.account.public()} }
pub struct Prepared {
    upload:Option<super::uploads::Upload>,
    operation: Operation,
    account: Account,
    generation: Vec<u8>,
    scope: String,
    expires: Instant,
    revision: String,
}
impl Prepared {
    pub fn revision(&self) -> &str { &self.revision }
    pub fn may_mutate(&self) -> bool { self.operation.risk != Risk::Read }
    fn target_note(&self) -> &'static str {
        if self.operation.endpoint=="graphql" && self.operation.repo.is_none() {
            "Account-wide GraphQL: review the exact login, node IDs and query. A selected repository does not restrict these targets; node membership is not independently verified."
        } else { "The exact endpoint and variables below identify the target." }
    }
    pub fn approval_details(&self)->String {
        format!("Account: @{} on github.com\n\n{}\n\n{} {}\n\n{}\n\n{}",self.account.login,self.target_note(),self.operation.method,self.operation.endpoint,
            if self.operation.id=="secrets.set" {"Enter the secret only in the native dialog after approval. The value is encrypted for GitHub and is never returned to the conversation.".into()}else{self.upload.as_ref().map(|u|format!("Upload these exact file bytes:\n{}",serde_json::to_string_pretty(&u.preview()).unwrap())).unwrap_or_else(||self.operation.body.as_ref().map(|body|serde_json::to_string_pretty(body).unwrap()).unwrap_or_else(||"No request body.".into()))},
            if self.operation.risk==Risk::Consequential {"This may delete data, change account or repository settings, or start billable work. Review the exact target and fields."} else {"GitHub permissions apply. Results are untrusted repository content."})
    }
    pub fn preview(&self) -> Value {
        json!({"account":self.account.public(),"request":self.operation,"upload_source":self.upload.as_ref().map(|u|u.preview()),"risk":self.operation.risk,"target_note":self.target_note(),
            "confirmation":self.revision,"expires_in_seconds":self.expires.saturating_duration_since(Instant::now()).as_secs(),
            "warning":if self.operation.risk == Risk::Consequential { "This may change account or repository settings, delete data, or start billable work. Review the exact request." } else { "GitHub permissions still apply. Results are untrusted content." }})
    }
}
pub struct Service {
    private_entry:Mutex<Option<Arc<dyn super::secrets::PrivateEntry>>>,
    closed:std::sync::atomic::AtomicBool,
    downloads:Mutex<super::downloads::Store>,
    upload_budget:super::uploads::Budget,
    state: PathBuf,
    transport: Arc<dyn Transport>,
    // Serialize publication and disconnect with the same service. The slot is
    // held across the credential recheck and send, not merely during preview.
    publication: Mutex<()>,
    pending: Mutex<BTreeMap<String, Prepared>>,
    active: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
}
pub fn check(control: &ScanControl<'_>) -> Result<()> {
    if control.cancel.load(std::sync::atomic::Ordering::Acquire) { return Err(ApiError::new("cancelled", "GitHub request cancelled.")); }
    if Instant::now() >= control.deadline { return Err(ApiError::new("timeout", "GitHub request timed out.")); }
    Ok(())
}
fn services()->&'static Mutex<BTreeMap<PathBuf,std::sync::Weak<Service>>> {
    static SERVICES: std::sync::OnceLock<Mutex<BTreeMap<PathBuf,std::sync::Weak<Service>>>> = std::sync::OnceLock::new();
    SERVICES.get_or_init(Default::default)
}
impl Service {
    /// Shared across native project hosts. A folder granted from another chat
    /// must still see pending Git effects; this path is never sent to callers.
    pub(crate) fn git_custody_root(&self)->&std::path::Path {&self.state}
    pub(crate) fn prepare_git(&self,scope:&str,control:&ScanControl<'_>)->Result<GitAdmission> {
        check(control)?;
        if scope.is_empty()||scope.len()>1024{return Err(ApiError::new("scope","A current native Git scope is required."));}
        let generation=self.generation()?;
        let account=self.controlled(control,|c|self.transport.account(c))?;
        if self.generation()?!=generation{return Err(ApiError::new("stale_account","GitHub connection changed."));}
        Ok(GitAdmission{account,generation,scope:scope.into(),expires:Instant::now()+Duration::from_secs(300)})
    }
    pub(crate) fn execute_git<T>(&self,plan:GitAdmission,scope:&str,control:&ScanControl<'_>,work:impl FnOnce(&super::transport::GitCredential,&ScanControl<'_>)->std::result::Result<T,ag1_core::DomainError>)->std::result::Result<T,ag1_core::DomainError> {
        use ag1_core::DomainError;
        let _publication=self.publication.try_lock().map_err(|_|DomainError::Conflict)?;
        if plan.scope!=scope||Instant::now()>=plan.expires||self.generation().ok().as_ref()!=Some(&plan.generation){return Err(DomainError::PermissionDenied);}
        let mut result=None;
        self.controlled(control,|c| {
            let credential=self.transport.git_credential(&plan.account,c)?;
            check(c)?;
            if self.generation()?!=plan.generation{return Err(ApiError::new("stale_account","GitHub connection changed before Git execution."));}
            result=Some(work(&credential,c));
            if check(c).is_err(){result=Some(Err(DomainError::DurabilityUnknown));}
            Ok(())
        }).map_err(super::tool::domain)?;
        // Once the callback starts, native Git owns its exact publication
        // boundary. A late revocation cannot turn a possible write into a safe
        // retry or export private results from the previous connection.
        if check(control).is_err()||self.generation().ok().as_ref()!=Some(&plan.generation){return Err(DomainError::DurabilityUnknown);}
        result.ok_or(DomainError::DurabilityUnknown)?
    }
    pub fn shared(state: &std::path::Path) -> Result<Arc<Self>> {
        let state=std::fs::canonicalize(state).map_err(|_|ApiError::new("connection_state", "Cannot locate private GitHub state."))?;
        let mut services=services().lock().map_err(|_|ApiError::new("busy", "GitHub service is busy."))?;
        services.retain(|_,s|s.strong_count()>0);
        if let Some(service)=services.get(&state).and_then(std::sync::Weak::upgrade) { return Ok(service); }
        let service=Arc::new(Self::new(state.clone(),Arc::new(super::transport::NativeTransport)));
        services.insert(state,Arc::downgrade(&service));
        Ok(service)
    }
    /// Explicit native host composition for owned transports. Cannot replace
    /// an active service and is not reachable through renderer/model arguments.
    pub fn compose(state:&std::path::Path,transport:Arc<dyn Transport>)->Result<Arc<Self>> {
        let state=std::fs::canonicalize(state).map_err(|_|ApiError::new("connection_state","Cannot locate private GitHub state."))?;
        let mut services=services().lock().map_err(|_|ApiError::new("busy","GitHub service is busy."))?;
        if services.get(&state).and_then(std::sync::Weak::upgrade).is_some() { return Err(ApiError::new("busy","GitHub transport is already bound.")); }
        let service=Arc::new(Self::new(state.clone(),transport));services.insert(state,Arc::downgrade(&service));Ok(service)
    }
    pub fn new(state: PathBuf, transport: Arc<dyn Transport>) -> Self {
        Self { private_entry:Mutex::new(None),upload_budget:Default::default(),downloads:Mutex::new(Default::default()),closed:std::sync::atomic::AtomicBool::new(false),state, transport, publication: Mutex::new(()), pending: Mutex::new(BTreeMap::new()), active:Mutex::new(None) }
    }
    pub fn set_private_entry(&self,entry:Arc<dyn super::secrets::PrivateEntry>)->Result<()> {
        let mut slot=self.private_entry.lock().map_err(|_|ApiError::new("busy","Private entry is busy."))?;
        if slot.is_some(){return Err(ApiError::new("private_entry","Native secret entry is already configured."));}*slot=Some(entry);Ok(())
    }
    pub fn shutdown(&self) {
        self.closed.store(true,std::sync::atomic::Ordering::Release);
        self.cancel();
        if let Ok(mut downloads)=self.downloads.lock(){downloads.clear();}
        if let Ok(mut plans)=self.pending.lock(){plans.clear();}
    }
    pub fn wait_stopped(&self,deadline:Instant)->Result<()> {
        while self.active.lock().map_err(|_|ApiError::new("busy","Cannot verify GitHub cleanup."))?.is_some() {
            if Instant::now()>=deadline{return Err(ApiError::new("shutdown_timeout","GitHub cleanup is still in progress. Wait before quitting again."));}
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    pub fn cancel(&self) { if let Ok(active)=self.active.lock() { if let Some(cancel)=active.as_ref() { cancel.store(true,std::sync::atomic::Ordering::Release); } } }
    fn controlled<T>(&self, outer:&ScanControl<'_>, work:impl FnOnce(&ScanControl<'_>)->Result<T>)->Result<T> {
        use std::sync::atomic::{AtomicBool,Ordering};
        let cancel=Arc::new(AtomicBool::new(false));
        { let mut active=self.active.lock().map_err(|_|ApiError::new("busy","GitHub control is busy."))?; if self.closed.load(Ordering::Acquire){return Err(ApiError::new("closed","GitHub is shutting down."));} if active.is_some() { return Err(ApiError::new("busy","A GitHub request is already in progress.")); } *active=Some(cancel.clone()); }
        struct Guard<'a>(&'a Service);
        impl Drop for Guard<'_> { fn drop(&mut self) { if let Ok(mut active)=self.0.active.lock() { *active=None; } } }
        let _guard=Guard(self);
        let done=AtomicBool::new(false);
        std::thread::scope(|threads| {
            struct Done<'a>(&'a AtomicBool);
            impl Drop for Done<'_> { fn drop(&mut self) {self.0.store(true,Ordering::Release);} }
            let done_guard=Done(&done);
            let monitor=threads.spawn(||while !done.load(Ordering::Acquire) { if outer.cancel.load(Ordering::Acquire) {cancel.store(true,Ordering::Release);break;} std::thread::sleep(Duration::from_millis(10)); });
            let control=ScanControl{cancel:&cancel,deadline:outer.deadline};
            let result=work(&control);
            drop(done_guard); let _=monitor.join(); result
        })
    }
    fn generation(&self) -> Result<Vec<u8>> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire){return Err(ApiError::new("closed","GitHub is shutting down."));}
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(self.state.join("github-connected"))
            .map_err(|_|ApiError::new("disconnected", "Connect GitHub before requesting account data."))?;
        let mut bytes = Vec::new();
        file.by_ref().take(4097).read_to_end(&mut bytes).map_err(|_|ApiError::new("connection_state", "Cannot read GitHub connection state."))?;
        if bytes.is_empty() || bytes.len() > 4096 { return Err(ApiError::new("connection_state", "Reconnect GitHub to refresh its connection state.")); }
        Ok(bytes)
    }
    pub fn account_view(&self,control:&ScanControl<'_>)->Result<Value> {
        let generation=self.generation()?;
        let account=self.controlled(control,|c|self.transport.account(c))?;
        if self.generation()?!=generation { return Err(ApiError::new("stale_account","GitHub connection changed.")); }
        Ok(account.public())
    }
    pub fn prepare(&self, request: Request, scope: &str, control: &ScanControl<'_>) -> Result<Prepared> {
        check(control)?;
        if scope.is_empty() || scope.len() > 1024 { return Err(ApiError::new("scope", "A current native conversation scope is required.")); }
        if request.operation=="releases.upload_asset"{return Err(ApiError::new("upload_source","Select a file through the native granted-folder upload channel."));}
        let operation = request.compile().map_err(|message|ApiError::new("arguments", message))?;
        super::schemas::validate(&operation).map_err(|message|ApiError::new("schema", message))?;
        self.prepare_operation(operation,scope,control)
    }
    fn prepare_operation(&self,operation:Operation,scope:&str,control:&ScanControl<'_>)->Result<Prepared>{
        check(control)?;
        if scope.is_empty()||scope.len()>1024{return Err(ApiError::new("scope","A current native conversation scope is required."));}
        let generation = self.generation()?;
        let account = self.controlled(control,|c|self.transport.account(c))?;
        check(control)?;
        if self.generation()? != generation { return Err(ApiError::new("stale_account", "GitHub connection changed. Review the request again.")); }
        Self::bind_operation(operation,account,generation,scope)
    }
    fn bind_operation(operation:Operation,account:Account,generation:Vec<u8>,scope:&str)->Result<Prepared>{
        let nonce = crate::client::nonce().map_err(|_|ApiError::new("unavailable", "Cannot create GitHub admission."))?;
        let revision = format!("github-{:x}",Sha256::digest(json!([nonce,scope,account.id,operation]).to_string()));
        Ok(Prepared { upload:None,operation, account, generation, scope: scope.into(), expires: Instant::now()+Duration::from_secs(300), revision })
    }
    pub fn prepare_upload(&self,request:Request,root:Arc<crate::workspace::Workspace>,grant:Arc<crate::folder_access::FolderAccess>,revision:u64,scope:&str,control:&ScanControl<'_>)->Result<Prepared>{
        let operation=request.compile().map_err(|_|ApiError::new("arguments","Invalid release upload fields."))?;
        super::schemas::validate_upload(&operation).map_err(|_|ApiError::new("schema","Upload does not match the pinned release asset schema."))?;
        self.pending.lock().map_err(|_|ApiError::new("busy","GitHub previews are busy."))?.retain(|_,p|p.expires>Instant::now());
        check(control)?;
        if scope.is_empty()||scope.len()>1024{return Err(ApiError::new("scope","A current native conversation scope is required."));}
        let generation=self.generation()?;
        let plan=self.controlled(control,|c|{
            let account=self.transport.account(c)?;
            let upload=super::uploads::Upload::capture(&request,root,grant,revision,&self.upload_budget,c)?;
            check(c)?;
            if self.generation()?!=generation{return Err(ApiError::new("stale_account","GitHub account changed while preparing the upload."));}
            let mut plan=Self::bind_operation(operation,account,generation.clone(),scope)?;
            plan.revision=format!("github-{:x}",Sha256::digest(json!([plan.revision,upload.preview()]).to_string()));plan.upload=Some(upload);Ok(plan)
        })?;
        check(control)?;
        if self.generation()?!=generation{return Err(ApiError::new("stale_account","GitHub disconnected; upload preview was discarded."));}
        Ok(plan)
    }
    pub fn preview_upload(&self,request:Request,root:Arc<crate::workspace::Workspace>,grant:Arc<crate::folder_access::FolderAccess>,revision:u64,scope:&str,control:&ScanControl<'_>)->Result<Value>{
        let plan=self.prepare_upload(request,root,grant,revision,scope,control)?;self.retain_preview(plan)
    }
    pub fn execute(&self, plan: Prepared, scope: &str, control: &ScanControl<'_>) -> Result<Reply> {
        let _guard = self.publication.try_lock().map_err(|_|ApiError::new("busy", "A GitHub operation is already in progress."))?;
        check(control)?;
        if scope != plan.scope || Instant::now() >= plan.expires { return Err(ApiError::new("stale_approval", "The GitHub request expired or its conversation changed. Review it again.")); }
        if self.generation()? != plan.generation { return Err(ApiError::new("stale_account", "GitHub connection changed. Review the request again.")); }
        // Transport obtains credentials afresh and compares the account plus
        // credential identity before sending the exact prepared operation.
        let reply = self.controlled(control,|c| {
            let reply=if plan.operation.id=="secrets.set" {
                let entry=self.private_entry.lock().map_err(|_|ApiError::new("busy","Private entry is busy."))?.clone().ok_or_else(||ApiError::new("private_entry","Open AG1 desktop to enter this secret in its native dialog."))?;
                self.transport.set_secret(&plan.account,&plan.operation,entry.as_ref(),c)?
            }else if let Some(upload)=&plan.upload {
                upload.verify(c)?;
                let reply=self.transport.upload(&plan.account,&plan.operation,upload,c)?;
                upload.validate_reply(&plan.operation,&reply.data)?;reply
            }else{self.execute_operation(&plan.account,&plan.generation,scope,&plan.operation,c)?};
            check(c).map_err(|e|e.uncertain(&plan.operation))?;
            super::workflow_responses::validate(&plan.operation,&reply)?;
            super::pull_responses::validate(&plan.operation,&reply)?;
            super::secrets::validate(&plan.operation,&reply)?;
            super::graph_reads::validate(&plan.operation,&reply.data)?;
            super::graph_mutations::validate(&plan.operation,&reply.data)?;
            Ok(reply)
        })?;
        check(control).map_err(|e|e.uncertain(&plan.operation))?;
        if self.generation().ok().as_ref() != Some(&plan.generation) { return Err(ApiError::new("stale_account", "GitHub disconnected; its private result was discarded.").uncertain(&plan.operation)); }
        Ok(reply)
    }
    fn execute_operation(&self,account:&Account,generation:&[u8],scope:&str,operation:&Operation,control:&ScanControl<'_>)->Result<Reply> {
        if operation.id=="downloads.read" {
            if self.transport.account(control)?!=*account || self.generation()?!=generation {return Err(ApiError::new("stale_account","GitHub account changed; download bytes were discarded."));}
            check(control)?;
            return self.downloads.lock().map_err(|_|ApiError::new("busy","Download store is busy."))?.page(account,generation,scope,operation);
        }
        if super::downloads::remote(&operation.id) {
            self.downloads.lock().map_err(|_|ApiError::new("busy","Download store is busy."))?.capacity(account,generation)?;
            let payload=self.transport.download(account,operation,control)?;
            check(control)?;
            if self.generation()?!=generation {return Err(ApiError::new("stale_account","GitHub disconnected; downloaded bytes were discarded."));}
            let mut downloads=self.downloads.lock().map_err(|_|ApiError::new("busy","Download store is busy."))?;
            check(control)?;
            if self.generation()?!=generation {return Err(ApiError::new("stale_account","GitHub disconnected; downloaded bytes were discarded."));}
            return downloads.retain(account,generation,scope,operation,payload);
        }
        self.transport.execute(account,operation,control)
    }
    pub fn preview(&self, request: Request, scope: &str, control: &ScanControl<'_>) -> Result<Value> {
        let plan = self.prepare(request,scope,control)?;
        self.retain_preview(plan)
    }
    fn retain_preview(&self,plan:Prepared)->Result<Value>{
        let view = plan.preview();
        let mut pending = self.pending.lock().map_err(|_|ApiError::new("busy", "GitHub request queue is busy."))?;
        if self.generation().ok().as_ref()!=Some(&plan.generation){return Err(ApiError::new("stale_account","GitHub connection changed while preparing this preview."));}
        pending.retain(|_,p|p.expires > Instant::now());
        if pending.len() >= 32 { return Err(ApiError::new("limit", "Too many pending GitHub previews. Cancel one first.")); }
        pending.insert(plan.revision.clone(),plan);
        Ok(view)
    }
    /// Explicit UI browsing only. All plans must be catalogued read operations,
    /// and a compound detail response is pinned to one account/generation.
    pub fn read(&self, requests:Vec<Request>,scope:&str,control:&ScanControl<'_>)->Result<Vec<Reply>> {
        if requests.is_empty() || requests.len()>4 { return Err(ApiError::new("limit","GitHub detail request limit exceeded.")); }
        check(control)?;
        if scope.is_empty()||scope.len()>1024{return Err(ApiError::new("scope","A current native conversation scope is required."));}
        // Reject writes before credential discovery, not after preparing them.
        let operations=requests.into_iter().map(|request| {
            let operation=request.compile().map_err(|_|ApiError::new("arguments","Invalid GitHub read request."))?;
            if operation.risk!=Risk::Read {return Err(ApiError::new("approval","GitHub changes require explicit native confirmation."));}
            super::schemas::validate(&operation).map_err(|_|ApiError::new("schema","GitHub read does not match its pinned schema."))?;
            Ok(operation)
        }).collect::<Result<Vec<_>>>()?;
        let _publication=self.publication.try_lock().map_err(|_|ApiError::new("busy","A GitHub operation is already in progress."))?;
        let generation=self.generation()?;
        // One control lease spans account discovery and every response in the
        // batch, including the gaps between requests. Cancel/quit cannot lose
        // the operation between separate prepare/execute control lifetimes.
        let replies=self.controlled(control,|c| {
            let account=self.transport.account(c)?;
            let mut replies=Vec::with_capacity(operations.len());
            for operation in &operations {
                check(c)?;
                if self.generation()?!=generation{return Err(ApiError::new("stale_account","GitHub account changed while loading details."));}
                let reply=self.execute_operation(&account,&generation,scope,operation,c)?;
                check(c)?;
                super::workflow_responses::validate(operation,&reply)?;
                super::secrets::validate(operation,&reply)?;
                super::graph_reads::validate(operation,&reply.data)?;
                if self.generation()?!=generation{return Err(ApiError::new("stale_account","GitHub disconnected; its private detail result was discarded."));}
                replies.push(reply);
            }
            Ok(replies)
        })?;
        check(control)?;
        if self.generation()?!=generation{return Err(ApiError::new("stale_account","GitHub disconnected; its private detail result was discarded."));}
        Ok(replies)
    }
    /// Taking the token is destructive even on failure: an uncertain result
    /// cannot be retried by submitting the same confirmation a second time.
    pub fn confirm(&self, revision: &str, scope: &str, control: &ScanControl<'_>) -> Result<Reply> {
        let plan = self.pending.lock().map_err(|_|ApiError::new("busy", "GitHub request queue is busy."))?.remove(revision)
            .ok_or_else(||ApiError::new("stale_approval", "This GitHub confirmation was used, cancelled, or expired."))?;
        self.execute(plan,scope,control)
    }
    pub fn dismiss(&self, revision: &str) { if let Ok(mut pending)=self.pending.lock() { pending.remove(revision); } }
    pub fn disconnect(&self) -> Result<()> {
        self.cancel();
        if let Ok(mut downloads)=self.downloads.lock(){downloads.clear();}
        match std::fs::remove_file(self.state.join("github-connected")) {
            Ok(()) => {}, Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
            Err(_) => return Err(ApiError::new("connection_state", "Cannot disconnect GitHub.")),
        }
        self.pending.lock().map_err(|_|ApiError::new("busy", "GitHub request queue is busy."))?.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[test]
    fn compound_reads_keep_one_control_lease_through_account_and_every_response() {
        struct Batch {
            service:Mutex<std::sync::Weak<Service>>,
            active:Mutex<Option<Arc<AtomicBool>>>,
            calls:AtomicUsize,
            accounts:AtomicUsize,
            cancel:bool,
        }
        impl Transport for Batch {
            fn account(&self,c:&ScanControl<'_>)->Result<Account>{
                check(c)?;self.accounts.fetch_add(1,Ordering::AcqRel);
                let service=self.service.lock().unwrap().upgrade().unwrap();
                *self.active.lock().unwrap()=service.active.lock().unwrap().clone();
                assert!(self.active.lock().unwrap().is_some());
                Account::bound(1,"owned-fixture".into(),"fixture-generation".into())
            }
            fn execute(&self,_:&Account,_:&Operation,c:&ScanControl<'_>)->Result<Reply>{
                check(c)?;
                let service=self.service.lock().unwrap().upgrade().unwrap();
                let held=self.active.lock().unwrap().clone().unwrap();let current=service.active.lock().unwrap().clone().unwrap();
                assert!(Arc::ptr_eq(&held,&current),"the complete read batch keeps the same cancellation/quit lease");
                let call=self.calls.fetch_add(1,Ordering::AcqRel)+1;
                if self.cancel&&call==2 {service.cancel();}
                Ok(Reply{data:json!({"private":"owned"}),status:200,next:None,rate_remaining:None,rate_reset:None,rate_resource:None,rate_limit:None,outcome:Outcome::Read})
            }
        }
        for cancelled in [false,true] {
            let temp=tempfile::tempdir().unwrap();std::fs::write(temp.path().join("github-connected"),b"owned").unwrap();
            let transport=Arc::new(Batch{service:Mutex::new(std::sync::Weak::new()),active:Mutex::new(None),calls:AtomicUsize::new(0),accounts:AtomicUsize::new(0),cancel:cancelled});
            let service=Arc::new(Service::new(temp.path().into(),transport.clone()));*transport.service.lock().unwrap()=Arc::downgrade(&service);
            let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
            let requests=(0..4).map(|_|serde_json::from_value(json!({"operation":"repos.get","repo":"owned/repo"})).unwrap()).collect();
            let result=service.read(requests,"owned-scope",&c);
            if cancelled {assert_eq!(result.unwrap_err().code,"cancelled");assert_eq!(transport.calls.load(Ordering::Acquire),2);}else{assert_eq!(result.unwrap().len(),4);assert_eq!(transport.calls.load(Ordering::Acquire),4);}
            assert_eq!(transport.accounts.load(Ordering::Acquire),1);assert!(service.active.lock().unwrap().is_none());
        }
    }
    struct Fixture { account: Mutex<Account>, calls: AtomicUsize, fail: AtomicBool, revoke: Option<PathBuf> }
    impl Transport for Fixture {
        fn git_credential(&self,a:&Account,c:&ScanControl<'_>)->Result<super::super::transport::GitCredential> {
            check(c)?;if self.account.lock().unwrap().clone()!=*a{return Err(ApiError::new("stale_account","Changed account."));}
            Ok(super::super::transport::GitCredential::fixture())
        }
        fn account(&self,c:&ScanControl<'_>)->Result<Account> { check(c)?; Ok(self.account.lock().unwrap().clone()) }
        fn execute(&self,a:&Account,o:&Operation,c:&ScanControl<'_>)->Result<Reply> {
            check(c)?;
            if self.account.lock().unwrap().clone()!=*a { return Err(ApiError::new("stale_account","Changed account.")); }
            self.calls.fetch_add(1,Ordering::Relaxed);
            if let Some(path)=&self.revoke { std::fs::remove_file(path).unwrap(); }
            if self.fail.load(Ordering::Relaxed) { return Err(ApiError::new("network","Interrupted.").uncertain(o)); }
            Ok(Reply { data:json!({"private":"owned fixture"}),status:200,next:Some("repos/owned/fixture/issues?page=2".into()),rate_remaining:Some(4999),rate_reset:Some(1),rate_resource:None,rate_limit:None,outcome:if o.risk==Risk::Read { Outcome::Read } else { Outcome::Applied } })
        }
    }
    fn setup(revoke: bool) -> (tempfile::TempDir,Service,Arc<Fixture>) {
        let dir=tempfile::tempdir().unwrap();
        let marker=dir.path().join("github-connected");std::fs::write(&marker,b"owned-generation-1").unwrap();
        let fixture=Arc::new(Fixture { account:Mutex::new(Account{id:1,login:"owned".into(),credential_revision:"credential-1".into()}),calls:AtomicUsize::new(0),fail:AtomicBool::new(false),revoke:revoke.then_some(marker) });
        let service=Service::new(dir.path().into(),fixture.clone());(dir,service,fixture)
    }
    fn request() -> Request { serde_json::from_value(json!({"operation":"issues.create","repo":"owned/fixture","body":{"title":"fixture","body":"test"}})).unwrap() }
    #[test] fn confirmation_is_exact_scoped_one_shot_and_native_only() {
        let (_dir,service,fixture)=setup(false);let cancelled=AtomicBool::new(false);let c=ScanControl{cancel:&cancelled,deadline:Instant::now()+Duration::from_secs(5)};
        let preview=service.preview(request(),"conversation-1",&c).unwrap();let key=preview["confirmation"].as_str().unwrap();
        assert!(!preview.to_string().contains("credential-1"));
        assert_eq!(fixture.calls.load(Ordering::Relaxed),0);
        assert_eq!(service.confirm(key,"conversation-2",&c).unwrap_err().code,"stale_approval");
        assert!(service.confirm(key,"conversation-1",&c).is_err());
        let preview=service.preview(request(),"conversation-1",&c).unwrap();let key=preview["confirmation"].as_str().unwrap();
        assert_eq!(service.confirm(key,"conversation-1",&c).unwrap().outcome,Outcome::Applied);
        assert!(service.confirm(key,"conversation-1",&c).is_err());assert_eq!(fixture.calls.load(Ordering::Relaxed),1);
    }
    #[test] fn account_credential_and_connection_changes_reject_old_approvals() {
        let (dir,service,fixture)=setup(false);let cancelled=AtomicBool::new(false);let c=ScanControl{cancel:&cancelled,deadline:Instant::now()+Duration::from_secs(5)};
        for kind in 0..3 {
            let plan=service.prepare(request(),"scope",&c).unwrap();
            match kind { 0=>fixture.account.lock().unwrap().id+=1,1=>fixture.account.lock().unwrap().credential_revision="credential-2".into(),_=>std::fs::write(dir.path().join("github-connected"),b"owned-generation-2").unwrap() }
            assert_eq!(service.execute(plan,"scope",&c).unwrap_err().code,"stale_account");
        }
        assert_eq!(fixture.calls.load(Ordering::Relaxed),0);
    }
    #[test] fn cancellation_disconnect_and_uncertain_outcome_never_replay() {
        let (_dir,service,fixture)=setup(false);let cancelled=AtomicBool::new(false);let c=ScanControl{cancel:&cancelled,deadline:Instant::now()+Duration::from_secs(5)};
        let plan=service.prepare(request(),"scope",&c).unwrap();cancelled.store(true,Ordering::Release);
        assert_eq!(service.execute(plan,"scope",&c).unwrap_err().outcome,Outcome::NotSent);assert_eq!(fixture.calls.load(Ordering::Relaxed),0);
        cancelled.store(false,Ordering::Release);fixture.fail.store(true,Ordering::Release);
        let preview=service.preview(request(),"scope",&c).unwrap();let key=preview["confirmation"].as_str().unwrap();
        assert_eq!(service.confirm(key,"scope",&c).unwrap_err().outcome,Outcome::Unknown);assert!(service.confirm(key,"scope",&c).is_err());assert_eq!(fixture.calls.load(Ordering::Relaxed),1);
        let preview=service.preview(request(),"scope",&c).unwrap();service.disconnect().unwrap();
        assert!(service.confirm(preview["confirmation"].as_str().unwrap(),"scope",&c).is_err());assert!(service.prepare(request(),"scope",&c).is_err());
    }
    #[test] fn revoked_private_results_are_discarded_and_queue_is_bounded() {
        let (_dir,service,_)=setup(true);let cancelled=AtomicBool::new(false);let c=ScanControl{cancel:&cancelled,deadline:Instant::now()+Duration::from_secs(5)};
        let plan=service.prepare(request(),"scope",&c).unwrap();let error=service.execute(plan,"scope",&c).unwrap_err();assert_eq!(error.code,"stale_account");assert!(!serde_json::to_string(&error).unwrap().contains("owned fixture"));
        let (_dir,service,_)=setup(false);
        for _ in 0..32 { service.preview(request(),"scope",&c).unwrap(); }
        assert_eq!(service.preview(request(),"scope",&c).unwrap_err().code,"limit");
    }
    #[test] fn ordinary_browsing_cannot_admit_writes_or_advanced_requests() {
        let (_dir,service,fixture)=setup(false);let cancelled=AtomicBool::new(false);let c=ScanControl{cancel:&cancelled,deadline:Instant::now()+Duration::from_secs(5)};
        assert_eq!(service.read(vec![request()],"scope",&c).unwrap_err().code,"approval");
        let advanced=serde_json::from_value(json!({"operation":"api.rest","parameters":{"method":"GET","endpoint":"user/repos"}})).unwrap();
        assert_eq!(service.read(vec![advanced],"scope",&c).unwrap_err().code,"approval");
        assert_eq!(fixture.calls.load(Ordering::Relaxed),0);
        let read=serde_json::from_value(json!({"operation":"repos.list"})).unwrap();
        assert_eq!(service.read(vec![read],"scope",&c).unwrap()[0].outcome,Outcome::Read);
    }
    #[test] fn active_native_work_is_cancelled_and_reaped_before_returning() {
        struct Hanging {started:AtomicBool}
        impl Transport for Hanging {
            fn account(&self,_:&ScanControl<'_>)->Result<Account> { Account::bound(1,"owned".into(),"fixture".into()) }
            fn execute(&self,_:&Account,op:&Operation,c:&ScanControl<'_>)->Result<Reply> { self.started.store(true,Ordering::Release); loop { check(c).map_err(|e|e.uncertain(op))?;std::thread::sleep(Duration::from_millis(2)); } }
        }
        for closing in [false,true] {
        let dir=tempfile::tempdir().unwrap();std::fs::write(dir.path().join("github-connected"),b"generation").unwrap();
        let fixture=Arc::new(Hanging{started:AtomicBool::new(false)});let service=Service::new(dir.path().into(),fixture.clone());
        let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
        let plan=service.prepare(request(),"scope",&c).unwrap();
        std::thread::scope(|threads| {
            let work=threads.spawn(||service.execute(plan,"scope",&c));
            while !fixture.started.load(Ordering::Acquire) { assert!(Instant::now()<c.deadline);std::thread::sleep(Duration::from_millis(2)); }
            if closing {service.shutdown();}else{service.disconnect().unwrap();}
            service.wait_stopped(c.deadline).unwrap();
            let error=work.join().unwrap().unwrap_err();assert_eq!(error.code,"cancelled");assert_eq!(error.outcome,Outcome::Unknown);
        });
        assert!(service.active.lock().unwrap().is_none());
        assert_eq!(dir.path().join("github-connected").exists(),closing);
        }
    }
    #[test] fn shutdown_retires_pending_work_without_erasing_saved_connection() {
        let (dir,service,fixture)=setup(false);let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
        let preview=service.preview(request(),"scope",&c).unwrap();
        service.shutdown();
        assert!(service.pending.lock().unwrap().is_empty());
        assert!(service.confirm(preview["confirmation"].as_str().unwrap(),"scope",&c).is_err());
        assert_eq!(service.prepare(request(),"scope",&c).err().unwrap().code,"closed");
        assert_eq!(service.account_view(&c).unwrap_err().code,"closed");
        assert!(dir.path().join("github-connected").exists());
        assert_eq!(fixture.calls.load(Ordering::Acquire),0);
    }
    #[test] fn native_git_admissions_bind_scope_account_credential_and_generation() {
        for change in 0..5 {
            let (dir,service,fixture)=setup(false);let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
            let mut plan=service.prepare_git("scope",&c).unwrap();let calls=AtomicUsize::new(0);
            match change {0=>fixture.account.lock().unwrap().id+=1,1=>fixture.account.lock().unwrap().credential_revision="changed".into(),2=>std::fs::write(dir.path().join("github-connected"),b"new").unwrap(),3=>plan.expires=Instant::now(),_=>{}}
            assert!(service.execute_git(plan,if change==4{"other"}else{"scope"},&c,|_,_|{calls.fetch_add(1,Ordering::AcqRel);Ok(())}).is_err());
            assert_eq!(calls.load(Ordering::Acquire),0);
        }
        let (_dir,service,_)=setup(false);let cancel=AtomicBool::new(false);let c=ScanControl{cancel:&cancel,deadline:Instant::now()+Duration::from_secs(5)};
        let plan=service.prepare_git("scope",&c).unwrap();
        assert!(!plan.account().to_string().contains("credential"));
        assert_eq!(service.execute_git(plan,"scope",&c,|_,_|Ok("exact once")).unwrap(),"exact once");
        let plan=service.prepare_git("scope",&c).unwrap();
        assert_eq!(service.execute_git(plan,"scope",&c,|_,_|{service.disconnect().unwrap();Ok("private result")}).unwrap_err(),ag1_core::DomainError::DurabilityUnknown);
    }
}
