//! Release uploads use exactly uploads.github.com, with no redirects/retries.
use super::*;
use crate::github::uploads::Upload;
async fn upload_to(client:&Client,token:&Token,operation:&Operation,upload:&Upload,control:&ScanControl<'_>,origin:&str)->Result<Reply>{
    if operation.id!="releases.upload_asset"||operation.method!="POST"||operation.body.is_some(){return Err(ApiError::new("arguments","Invalid native upload."));}
    upload.verify(control)?;
    let mut auth=b"Bearer ".to_vec();auth.extend_from_slice(&token.0);let header=HeaderValue::from_bytes(&auth);auth.fill(0);
    let mut header=header.map_err(|_|ApiError::new("credentials","Invalid GitHub credential."))?;header.set_sensitive(true);
    let request=client.post(format!("{origin}/{}",operation.endpoint)).header(AUTHORIZATION,header)
        .header("Accept","application/vnd.github+json").header("X-GitHub-Api-Version",crate::github::REST_VERSION)
        .header("Content-Type",&upload.content_type).body(upload.payload.bytes.clone());
    // This final source/grant check follows account identity and body creation.
    upload.verify(control)?;
    checked(control,async {
        let mut response=request.send().await.map_err(|_|ApiError::new("upload_network","Upload interrupted; inspect release assets before trying again."))?;
        let status=response.status().as_u16();
        let remaining=response.headers().get("x-ratelimit-remaining").and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok());
        let retry=response.headers().get("retry-after").and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok());
        let reset=response.headers().get("x-ratelimit-reset").and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok());
        if status!=201 {
            let (code,message)=match status {301..=308=>("upload_redirect","Upload redirect refused. File bytes and credentials were not forwarded."),401=>("reauthenticate","Sign in to GitHub again."),403|429 if retry.is_some()||remaining==Some(0)||status==429=>("rate_limit","GitHub upload rate limit reached. Inspect the release before trying again."),403=>("permission","GitHub denied this release upload."),404=>("not_found","Release was not found or is unavailable to this account."),422=>("upload_conflict","GitHub rejected this upload, possibly because an asset already has this name. Existing assets were not automatically deleted."),_=>("upload_http","GitHub did not confirm the upload. An incomplete asset may remain; inspect the release before trying again.")};
            let mut error=ApiError::new(code,message);error.status=Some(status);error.retry_after_seconds=retry;error.retry_at_epoch_seconds=reset;return Err(error);
        }
        if response.content_length().is_some_and(|n|n>1024*1024){return Err(ApiError::new("upload_response_limit","Upload receipt exceeds the response limit."));}
        let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|ApiError::new("upload_response","Upload receipt was interrupted."))? {if bytes.len()+chunk.len()>1024*1024{return Err(ApiError::new("upload_response_limit","Upload receipt exceeds the response limit."));}bytes.extend_from_slice(&chunk);}
        let mut data:Value=serde_json::from_slice(&bytes).map_err(|_|ApiError::new("upload_response","GitHub returned an invalid upload receipt."))?;sanitize(&mut data,&token.0,0)?;upload.validate_reply(operation,&data)?;
        Ok(Reply{data,status,next:None,rate_remaining:remaining,rate_reset:reset,rate_resource:None,rate_limit:None,outcome:Outcome::Applied})
    }).await.map_err(|e|e.uncertain(operation))
}
pub(super) fn execute(expected:&Account,operation:&Operation,upload:&Upload,control:&ScanControl<'_>)->Result<Reply>{
    network(||{
        let token=token(control)?;
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|ApiError::new("transport","Cannot start GitHub worker."))?;
        rt.block_on(async {
            let client=client()?;
            if identify(&client,&token,control).await?!=*expected{return Err(ApiError::new("stale_account","The active GitHub account or credential changed. Review this upload again."));}
            // GitHub.com release upload relation has this fixed origin and path;
            // untrusted upload_url values can never select a credential target.
            upload_to(&client,&token,operation,upload,control,"https://uploads.github.com").await
        })
    })
}
#[cfg(test)]
#[path="upload_transport_fixture.rs"]
mod fixture;
