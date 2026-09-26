//! Only the fixed API origin receives account credentials. Signed download
//! locations stay native and are fetched with a fresh, credential-free request.
use super::*;
use crate::github::downloads::{Payload,MAX_BYTES};
fn redirect_url(raw:&str)->Result<reqwest::Url>{
    let invalid=||ApiError::new("download_redirect","GitHub returned an unsupported download location. No credentials were forwarded.");
    if raw.len()>16384{return Err(invalid());}
    let url=reqwest::Url::parse(raw).map_err(|_|invalid())?;
    let host=url.host_str().ok_or_else(invalid)?;
    let allowed=["release-assets.githubusercontent.com","objects.githubusercontent.com","github.com"].contains(&host)
        || host.ends_with(".blob.core.windows.net") || host.ends_with(".actions.githubusercontent.com");
    if !allowed || url.scheme()!="https" || url.port_or_known_default()!=Some(443) || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {return Err(invalid());}Ok(url)
}
async fn download(client:&Client,token:&Token,operation:&Operation,control:&ScanControl<'_>,origin:&str)->Result<Payload>{
    download_with(client,token,operation,control,origin,redirect_url).await
}
// The URL validator injection exists only as a native test seam. Production
// always supplies redirect_url above, never caller arguments or environment.
pub(super) async fn download_with(client:&Client,token:&Token,operation:&Operation,control:&ScanControl<'_>,origin:&str,redirect:fn(&str)->Result<reqwest::Url>)->Result<Payload>{
    if !crate::github::downloads::remote(&operation.id) || operation.method!="GET" || operation.body.is_some(){return Err(ApiError::new("arguments","Invalid native download operation."));}
    check(control)?;
    let mut auth=b"Bearer ".to_vec();auth.extend_from_slice(&token.0);
    let header=HeaderValue::from_bytes(&auth);auth.fill(0);
    let mut header=header.map_err(|_|ApiError::new("credentials","Invalid GitHub credential."))?;header.set_sensitive(true);
    let request=client.get(format!("{origin}/{}",operation.endpoint)).header(AUTHORIZATION,header)
        .header("Accept",if operation.id=="releases.download_asset"{"application/octet-stream"}else{"application/vnd.github+json"})
        .header("X-GitHub-Api-Version",crate::github::REST_VERSION);
    checked(control,async {
        let mut response=request.send().await.map_err(|_|ApiError::new("download_network","GitHub download interrupted."))?;
        for hop in 0..=2 {
            if response.status().as_u16()!=302 {break;}
            if hop==2 {return Err(ApiError::new("download_redirect","Too many download redirects."));}
            let raw=response.headers().get("location").and_then(|v|v.to_str().ok()).ok_or_else(||ApiError::new("download_redirect","Missing download location."))?;
            if std::str::from_utf8(&token.0).is_ok_and(|secret|!secret.is_empty()&&raw.contains(secret)){return Err(ApiError::new("download_redirect","Download location contains credential material."));}
            let target=redirect(raw)?;
            // No Authorization, cookies, referer or API headers on a signed URL.
            // The client has no cookie store, ambient proxy or auto-redirects.
            response=client.get(target).header("Accept","application/octet-stream").send().await.map_err(|_|ApiError::new("download_network","GitHub download interrupted."))?;
        }
        let status=response.status().as_u16();
        if status!=200 {
            let retry=response.headers().get("retry-after").and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok());
            let remaining=response.headers().get("x-ratelimit-remaining").and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok());
            let (code,message)=match status {403 if retry.is_some()||remaining==Some(0)=>("rate_limit","GitHub download rate limit reached. Wait before trying again."),401=>("reauthenticate","Sign in to GitHub again."),403=>("permission","GitHub denied this download."),404|410=>("download_unavailable","Download was not found, expired, or is unavailable to this account."),429=>("rate_limit","GitHub download rate limit reached. Wait before trying again."),301..=308=>("download_redirect","Unsupported download redirect; credentials were not forwarded."),_=>("download_http","GitHub could not provide this download.")};
            let mut error=ApiError::new(code,message);error.status=Some(status);error.retry_after_seconds=response.headers().get("retry-after").and_then(|v|v.to_str().ok()).and_then(|s|s.parse().ok());error.retry_at_epoch_seconds=response.headers().get("x-ratelimit-reset").and_then(|v|v.to_str().ok()).and_then(|s|s.parse().ok());return Err(error);
        }
        if response.content_length().is_some_and(|n|n>MAX_BYTES as u64){return Err(ApiError::new("download_limit","GitHub download exceeds 64 MiB."));}
        let mut payload=Payload{bytes:Vec::new()};
        while let Some(chunk)=response.chunk().await.map_err(|_|ApiError::new("download_interrupted","GitHub download ended before all bytes arrived."))? {
            check(control)?;
            if payload.bytes.len()+chunk.len()>MAX_BYTES{return Err(ApiError::new("download_limit","GitHub download exceeds 64 MiB."));}
            payload.bytes.extend_from_slice(&chunk);
        }
        if !token.0.is_empty() && payload.bytes.windows(token.0.len()).any(|part|part==token.0){return Err(ApiError::new("download_credentials","Downloaded content unexpectedly contains account credentials and was discarded."));}
        Ok(payload)
    }).await
}
pub(super) fn execute(expected:&Account,operation:&Operation,control:&ScanControl<'_>)->Result<Payload>{
    network(||{
        let token=token(control)?;
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|ApiError::new("transport","Cannot start GitHub worker."))?;
        rt.block_on(async {
            let client=client()?;let account=identify(&client,&token,control).await?;
            if account!=*expected{return Err(ApiError::new("stale_account","The active GitHub account or credential changed. Download again from the current account."));}
            download(&client,&token,operation,control,"https://api.github.com").await
        })
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn signed_locations_do_not_grant_arbitrary_origins(){
        for url in ["https://release-assets.githubusercontent.com/a?sig=x","https://owned.blob.core.windows.net/a","https://results-receiver.actions.githubusercontent.com/a"] {assert!(redirect_url(url).is_ok());}
        for url in ["http://release-assets.githubusercontent.com/a","https://release-assets.githubusercontent.com.evil.test/a","https://127.0.0.1/a","https://api.github.com/a","https://user:pass@github.com/a","https://github.com:8443/a","https://github.com/a#fragment","https://blob.core.windows.net/a"] {assert!(redirect_url(url).is_err());}
    }
}
