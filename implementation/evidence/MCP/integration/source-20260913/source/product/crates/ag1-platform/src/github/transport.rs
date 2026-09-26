//! Fixed-origin HTTPS. gh only retrieves the credential from its private store;
//! tokens are never argv, renderer values, model arguments, artifacts or logs.
use super::{operations::{Operation, Risk}, service::{Account, ApiError, Outcome, Reply, Result, Transport, check}};
use crate::resource_fs::ScanControl;
use reqwest::{Client, Method, header::{HeaderValue, AUTHORIZATION}};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, process::Stdio, time::{Duration, Instant}};

#[path="download_transport.rs"]
mod downloads;
#[path="upload_transport.rs"]
mod uploads;
#[path="secret_transport.rs"]
mod secrets;
pub struct NativeTransport;
struct Token(Vec<u8>);
impl Drop for Token { fn drop(&mut self) { self.0.fill(0); } }
/// Opaque native credential: no Debug, Clone or serialization. It is consumed
/// only by the contained Git transport, never returned to a client or model.
pub struct GitCredential(Token);
impl GitCredential {
    pub(crate) fn config(&self) -> Vec<u8> {
        use base64::Engine;
        let mut basic=b"x-access-token:".to_vec();basic.extend_from_slice(&self.0.0);
        let encoded=base64::engine::general_purpose::STANDARD.encode(&basic);basic.fill(0);
        let config=format!("[core]\nrepositoryformatversion = 0\nbare = false\nhooksPath = /dev/null\nfsmonitor = false\n[credential]\nhelper =\n[http]\nfollowRedirects = false\nsslVerify = true\nproxy =\n[http \"https://github.com/\"]\nextraHeader = Authorization: Basic {encoded}\n[protocol]\nallow = never\n[protocol \"https\"]\nallow = always\n[fetch]\nunpackLimit = 0\nfsckObjects = true\n[transfer]\nfsckObjects = true\n[gc]\nauto = 0\n[maintenance]\nauto = false\n").into_bytes();
        // The caller also clears the complete config after passing it through
        // the owned stdin pipe into a readonly, namespace-only bind mount.
        encoded.into_bytes().fill(0);
        config
    }
    #[cfg(test)]
    pub(crate) fn fixture() -> Self { Self(Token(b"owned-synthetic-token".to_vec())) }
}
struct Child(std::process::Child);
impl Drop for Child { fn drop(&mut self) { super::stop_child(&mut self.0); } }

fn token(control: &ScanControl<'_>) -> Result<Token> {
    check(control)?;
    let mut command = super::gh();
    command.args(["auth", "token", "--hostname", "github.com"]).stdin(Stdio::null()).stderr(Stdio::null()).stdout(Stdio::piped());
    let mut child = Child(command.spawn().map_err(|_|ApiError::new("missing_cli", "Install GitHub CLI, then reconnect GitHub."))?);
    let mut pipe = child.0.stdout.take().ok_or_else(||ApiError::new("credentials", "Cannot read GitHub credentials."))?;
    use std::os::fd::AsRawFd;
    if unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 { return Err(ApiError::new("credentials", "Cannot read GitHub credentials.")); }
    let mut bytes = Token(Vec::new());
    let deadline = control.deadline.min(Instant::now()+Duration::from_secs(15));
    let mut exited = false;
    loop {
        check(control)?;
        if Instant::now() >= deadline { return Err(ApiError::new("credentials_timeout", "GitHub credential store did not respond.")); }
        let mut buffer = [0;1024];
        match pipe.read(&mut buffer) {
            Ok(0) if exited => break,
            Ok(n) => { if bytes.0.len()+n>8192 { return Err(ApiError::new("credentials", "Invalid GitHub credential.")); } bytes.0.extend_from_slice(&buffer[..n]); buffer.fill(0); },
            Err(e) if e.kind()==std::io::ErrorKind::WouldBlock => {},
            Err(_) => return Err(ApiError::new("credentials", "Cannot read GitHub credentials.")),
        }
        if !exited { if let Some(status)=child.0.try_wait().map_err(|_|ApiError::new("credentials", "GitHub credential helper interrupted."))? {
            if !status.success() { return Err(ApiError::new("reauthenticate", "Sign in to GitHub again.")); }
            exited = true;
        } }
        std::thread::sleep(Duration::from_millis(15));
    }
    while bytes.0.last().is_some_and(u8::is_ascii_whitespace) { bytes.0.pop(); }
    if bytes.0.is_empty() || !bytes.0.iter().all(|b|b.is_ascii_graphic()) { return Err(ApiError::new("credentials", "Invalid GitHub credential.")); }
    Ok(bytes)
}

fn client() -> Result<Client> {
    Client::builder().https_only(true).no_proxy().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(45)).user_agent("AG1-native-GitHub")
        .build().map_err(|_|ApiError::new("transport", "Cannot initialize GitHub HTTPS."))
}
async fn checked<T>(control: &ScanControl<'_>, work: impl std::future::Future<Output=Result<T>>) -> Result<T> {
    tokio::pin!(work);
    loop {
        check(control)?;
        tokio::select! { value = &mut work => return value, _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
    }
}
async fn send(client: &Client, token: &Token, operation: &Operation, control: &ScanControl<'_>) -> Result<Reply> {
    send_to(client,token,operation,control,"https://api.github.com").await
}
// Native-only injection seam for owned HTTP/TLS response fixtures. Production
// has exactly one caller, above, with a literal origin; neither caller arguments
// nor process environment can change it.
async fn send_to(client: &Client, token: &Token, operation: &Operation, control: &ScanControl<'_>, origin: &str) -> Result<Reply> {
    check(control)?;
    let mut auth = b"Bearer ".to_vec(); auth.extend_from_slice(&token.0);
    let header = HeaderValue::from_bytes(&auth); auth.fill(0);
    let mut header = header.map_err(|_|ApiError::new("credentials", "Invalid GitHub credential."))?;
    header.set_sensitive(true);
    let method = Method::from_bytes(operation.method.as_bytes()).map_err(|_|ApiError::new("arguments", "Invalid GitHub method."))?;
    let mut request = client.request(method,format!("{origin}/{}",operation.endpoint))
        .header(AUTHORIZATION,header).header("Accept","application/vnd.github+json")
        .header("X-GitHub-Api-Version",super::REST_VERSION);
    if let Some(body)=&operation.body { request = request.header("Content-Type","application/json").body(serde_json::to_vec(body).map_err(|_|ApiError::new("arguments", "Invalid GitHub request body."))?); }
    // Once send is polled, cancellation/network failure may follow a committed
    // server effect. No request is automatically retried, including on 429.
    checked(control, async {
        let mut response = request.send().await.map_err(|_|ApiError::new("network", "GitHub request interrupted. Check GitHub before repeating a change."))?;
        let status = response.status().as_u16();
        let header_number = |name: &str| response.headers().get(name).and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok());
        let remaining = header_number("x-ratelimit-remaining");
        let reset = header_number("x-ratelimit-reset");
        let retry_after = header_number("retry-after");
        if !(200..300).contains(&status) {
            let (code,message) = match status {
                301..=308 => ("redirect", "GitHub returned a redirect. Review its target separately; credentials were not forwarded."),
                401 => ("reauthenticate", "Sign in to GitHub again."),
                403 | 429 if remaining == Some(0) || retry_after.is_some() || status==429 => ("rate_limit", "GitHub rate limit reached. Wait until the reported retry time; do not repeat a write blindly."),
                403 => ("permission", "GitHub denied this operation. Check account, repository permissions, and organization policy."),
                404 => ("not_found", "Item not found or unavailable to this account."),
                409 | 412 => ("conflict", "GitHub state changed. Refresh and review the operation again."),
                422 => ("validation", "GitHub rejected the fields or reviewed commit. Refresh and review the request."),
                _ => ("http", "GitHub could not complete this operation."),
            };
            let mut error = ApiError::new(code,message); error.status=Some(status); error.retry_after_seconds=retry_after; error.retry_at_epoch_seconds=reset;
            return Err(error);
        }
        let rate_resource = response.headers().get("x-ratelimit-resource").and_then(|v|v.to_str().ok()).filter(|v| matches!(*v,"core"|"search"|"code_search"|"graphql"|"integration_manifest"|"source_import"|"actions_runner_registration"|"scim")).map(str::to_owned);
        let rate_limit = response.headers().get("x-ratelimit-limit").and_then(|v|v.to_str().ok()).and_then(|v|v.parse().ok());
        let next = response.headers().get("link").and_then(|h|h.to_str().ok()).and_then(next_link)
            .filter(|link|!std::str::from_utf8(&token.0).is_ok_and(|secret|!secret.is_empty() && link.contains(secret)));
        if response.content_length().is_some_and(|n|n>4*1024*1024) { return Err(ApiError::new("response_limit", "GitHub response is too large. Narrow the request or page it.")); }
        let mut bytes=Vec::new();
        while let Some(chunk)=response.chunk().await.map_err(|_|ApiError::new("interrupted", "GitHub response was interrupted."))? {
            if bytes.len()+chunk.len()>4*1024*1024 { return Err(ApiError::new("response_limit", "GitHub response is too large. Narrow the request or page it.")); }
            bytes.extend_from_slice(&chunk);
        }
        let mut data = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).map_err(|_|ApiError::new("malformed", "GitHub returned invalid JSON or non-JSON content."))? };
        sanitize(&mut data, &token.0,0)?;
        if operation.id=="pulls.merge" && data["merged"]!=true {
            return Err(ApiError::new("merge_not_applied","GitHub did not merge this pull request. Refresh its head, checks, and merge requirements."));
        }
        if operation.endpoint=="graphql" {
            if !data.is_object() || data.get("errors").is_some_and(|e|!e.is_array()) || data.get("data").is_none() && data.get("errors").is_none() {
                return Err(ApiError::new("malformed_graphql","GitHub returned an invalid GraphQL response envelope."));
            }
            if data.get("errors").and_then(Value::as_array).is_some_and(|a|!a.is_empty()) {
                return Err(ApiError::new("graphql", "GitHub GraphQL reported errors; a mutation may have partially applied. Review GitHub before repeating it."));
            }
        }
        Ok(Reply { data,status,next,rate_remaining:remaining,rate_reset:reset,rate_resource,rate_limit,outcome:if operation.risk==Risk::Read { Outcome::Read } else { Outcome::Applied } })
    }).await.map_err(|e|e.uncertain(operation))
}
fn next_link(header: &str) -> Option<String> {
    if header.len()>8192 { return None; }
    header.split(',').find_map(|part| {
        let (url,rel)=part.trim().split_once(';')?;
        if rel.trim()!="rel=\"next\"" { return None; }
        let url=reqwest::Url::parse(url.strip_prefix('<')?.strip_suffix('>')?).ok()?;
        (url.scheme()=="https" && url.host_str()==Some("api.github.com") && url.port_or_known_default()==Some(443) && url.username().is_empty() && url.password().is_none() && url.fragment().is_none())
            .then(||format!("{}{}",url.path().trim_start_matches('/'),url.query().map(|q|format!("?{q}")).unwrap_or_default()))
    })
}
fn sanitize(value: &mut Value, token: &[u8], depth: usize) -> Result<()> {
    if depth>32 { return Err(ApiError::new("response_depth", "GitHub response nesting exceeds the safe preview limit.")); }
    match value {
        Value::String(s) => if let Ok(token)=std::str::from_utf8(token) { if !token.is_empty() && s.contains(token) { *s=s.replace(token,"[redacted]"); } },
        Value::Array(items) => for v in items { sanitize(v,token,depth+1)?; },
        Value::Object(map) => {
            // Untrusted responses may echo a credential as a JSON key, not just
            // a value. Drop such fields rather than exporting the secret key.
            if let Ok(secret)=std::str::from_utf8(token) {if !secret.is_empty(){map.retain(|key,_|!key.contains(secret));}}
            for (key,v) in map { if super::credential_fields::sensitive(key) { *v=Value::String("[redacted]".into()); } else { sanitize(v,token,depth+1)?; } }
        },
        _ => {},
    }
    Ok(())
}
async fn identify(client: &Client, token: &Token, control: &ScanControl<'_>) -> Result<Account> {
    let request=Operation { id:"account".into(),method:"GET".into(),endpoint:"user".into(),body:None,risk:Risk::Read,repo:None };
    let reply=send(client,token,&request,control).await?;
    let id=reply.data["id"].as_u64().filter(|id|*id>0).ok_or_else(||ApiError::new("account", "Invalid GitHub account response."))?;
    let login=reply.data["login"].as_str().filter(|s|!s.is_empty() && s.len()<=128 && s.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-')).ok_or_else(||ApiError::new("account", "Invalid GitHub account response."))?.into();
    Ok(Account { id,login,credential_revision:format!("{:x}",Sha256::digest(&token.0)) })
}
fn network<T: Send>(task: impl FnOnce() -> Result<T> + Send) -> Result<T> {
    // This service is invoked from both async UI dispatch and blocking native
    // adapters. Own the current-thread runtime on an owned worker in either case.
    std::thread::scope(|scope|scope.spawn(task).join().map_err(|_|ApiError::new("transport", "GitHub worker interrupted."))?)
}
impl Transport for NativeTransport {
    fn set_secret(&self,account:&Account,operation:&Operation,entry:&dyn super::secrets::PrivateEntry,control:&ScanControl<'_>)->Result<Reply>{secrets::execute(account,operation,entry,control)}
    fn upload(&self,expected:&Account,operation:&Operation,upload:&super::uploads::Upload,control:&ScanControl<'_>)->Result<Reply>{uploads::execute(expected,operation,upload,control)}
    fn download(&self,expected:&Account,operation:&Operation,control:&ScanControl<'_>)->Result<super::downloads::Payload> {downloads::execute(expected,operation,control)}
    fn git_credential(&self, expected:&Account,control:&ScanControl<'_>)->Result<GitCredential> {
        network(|| {
            let token=token(control)?;
            let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|ApiError::new("transport","Cannot start GitHub worker."))?;
            let account=rt.block_on(identify(&client()?,&token,control))?;
            if account!=*expected{return Err(ApiError::new("stale_account","The active GitHub account or credential changed. Review the Git request again."));}
            check(control)?;Ok(GitCredential(token))
        })
    }
    fn account(&self, control: &ScanControl<'_>) -> Result<Account> {
        network(|| {
            let token=token(control)?;
            let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|ApiError::new("transport", "Cannot start GitHub worker."))?;
            rt.block_on(identify(&client()?,&token,control))
        })
    }
    fn execute(&self, expected: &Account, operation: &Operation, control: &ScanControl<'_>) -> Result<Reply> {
        network(|| {
            let token=token(control)?;
            let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|ApiError::new("transport", "Cannot start GitHub worker."))?;
            rt.block_on(async {
                let client=client()?;
                let account=identify(&client,&token,control).await?;
                if account!=*expected { return Err(ApiError::new("stale_account", "The active GitHub account or credential changed. Review the request again.")); }
                check(control)?;
                send(&client,&token,operation,control).await
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn pagination_cannot_forward_credentials() {
        assert_eq!(next_link("<https://api.github.com/user/repos?page=2>; rel=\"next\""),Some("user/repos?page=2".into()));
        for url in ["http://api.github.com/x","https://evil.test/x","https://api.github.com.evil.test/x","https://user@api.github.com/x","https://api.github.com:8443/x"] { assert!(next_link(&format!("<{url}>; rel=\"next\"")).is_none()); }
    }
    #[test] fn secrets_are_redacted_without_reflecting_helper_errors() {
        let mut value=serde_json::json!({"token":"secret","nested":[{"body":"prefix exact-secret suffix"}]});
        sanitize(&mut value,b"exact-secret",0).unwrap();
        assert_eq!(value["token"],"[redacted]"); assert_eq!(value["nested"][0]["body"],"prefix [redacted] suffix");
    }
}

#[cfg(test)]
#[path = "transport_fixture.rs"]
mod fixture;

#[cfg(test)]
mod credential_response_tests {
    use super::*;
    #[test]
    fn private_response_fields_are_redacted_across_case_and_nesting() {
        let mut value=serde_json::json!({"accessToken":"owned","nested":[{"clientSecret":"owned","secret":"owned","sourceAccessToken":"owned","encoded_jit_config":"owned","public":"kept"}],"body":"echo synthetic-credential"});
        sanitize(&mut value,b"synthetic-credential",0).unwrap();assert_eq!(value["accessToken"],"[redacted]");
        for key in ["clientSecret","secret","sourceAccessToken","encoded_jit_config"]{assert_eq!(value["nested"][0][key],"[redacted]");}assert_eq!(value["nested"][0]["public"],"kept");assert!(!value.to_string().contains("synthetic-credential"));
    }
}
