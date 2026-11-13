//! Public registry metadata only. No credential port, proxy, redirect or retry.
use super::*;
use crate::web_http::{SystemResolver, WebPolicy, WebResolver};
use std::net::SocketAddr;

pub(super) fn get(url: reqwest::Url) -> Result<(Vec<u8>, u64), DomainError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| invalid("registry network runtime"))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(12), async {
            let host = url.host_str().ok_or(DomainError::PermissionDenied)?;
            if host != "api.mcp.github.com" || url.scheme() != "https" || url.port_or_known_default() != Some(443) {
                return Err(DomainError::PermissionDenied);
            }
            let addresses = SystemResolver.resolve(host,443).await?;
            WebPolicy::default().validate(url.as_str(),&addresses)?;
            let addresses:Vec<_> = addresses.into_iter().map(|ip|SocketAddr::new(ip,443)).collect();
            let client = reqwest::Client::builder().no_proxy().no_gzip().no_brotli().no_deflate()
                .redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never())
                .pool_max_idle_per_host(0).connect_timeout(Duration::from_secs(3))
                .resolve_to_addrs(host,&addresses).build().map_err(|_|invalid("registry TLS client"))?;
            let mut response = client.get(url).header("accept","application/json").header("user-agent","AG1-MCP/0.1")
                .send().await.map_err(|_|DomainError::Unavailable("Discovery source could not be reached. Saved servers and cached results remain available.".into()))?;
            let headers=response.headers();
            if headers.len()>64 || headers.iter().map(|(k,v)|k.as_str().len()+v.as_bytes().len()).sum::<usize>()>16384 {
                return Err(invalid("registry response headers"));
            }
            let now=now()?;
            let mut retry_at=0;
            if headers.get("x-ratelimit-remaining").is_some_and(|v|v=="0") || response.status()==429 {
                let reset=headers.get("x-ratelimit-reset").and_then(|v|v.to_str().ok()).and_then(|v|v.parse::<u64>().ok()).unwrap_or(now+60);
                let after=headers.get("retry-after").and_then(|v|v.to_str().ok()).and_then(|v|v.parse::<u64>().ok()).unwrap_or(60);
                retry_at=reset.max(now.saturating_add(after)).min(now+86400);
            }
            if response.status()==429 { return Ok((Vec::new(),retry_at)); }
            if response.status()!=200 { return Err(DomainError::Unavailable(format!("Discovery source returned HTTP {}. Cached results remain available.",response.status().as_u16()))); }
            if headers.get("content-encoding").is_some_and(|v|v!="identity") || !headers.get("content-type").and_then(|v|v.to_str().ok()).is_some_and(|v|v.split(';').next()==Some("application/json")) {
                return Err(invalid("registry response encoding/type"));
            }
            if response.content_length().is_some_and(|v|v>MAX_PAGE_BYTES as u64) {return Err(DomainError::LimitExceeded("registry page".into()));}
            let mut bytes=Vec::new();
            while let Some(chunk)=response.chunk().await.map_err(|_|invalid("registry response body"))? {
                if bytes.len()+chunk.len()>MAX_PAGE_BYTES {return Err(DomainError::LimitExceeded("registry page".into()));}
                bytes.extend_from_slice(&chunk);
            }
            Ok((bytes,retry_at))
        }).await.map_err(|_|DomainError::TimedOut)?
    })
}
