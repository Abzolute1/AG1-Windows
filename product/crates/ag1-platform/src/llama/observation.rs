//! Read-only runtime observation. HTTP health never confers process ownership.
use super::{
    LaunchPlan,
    lifecycle::{ModelIdentity, Observation},
};
use crate::provider::http::*;
use ag1_core::DomainError;
use serde_json::Value;
use std::{
    net::{IpAddr, SocketAddr, TcpListener},
    time::Duration,
};
use tokio::time::Instant;
pub fn origin(plan: &LaunchPlan) -> Result<String, DomainError> {
    let arg = |key: &str| {
        plan.args
            .windows(2)
            .find(|w| w[0] == key)
            .map(|w| w[1].as_str())
    };
    let host =
        arg("--host").ok_or_else(|| DomainError::InvalidInput("runtime host flag".into()))?;
    let address: IpAddr = host
        .parse()
        .map_err(|_| DomainError::InvalidInput("runtime host IP".into()))?;
    let port: u16 = arg("--port")
        .and_then(|s| s.parse().ok())
        .filter(|p| *p > 0)
        .ok_or_else(|| DomainError::InvalidInput("runtime port".into()))?;
    let address = if address.is_unspecified() {
        if address.is_ipv4() {
            "127.0.0.1".parse().unwrap()
        } else {
            "::1".parse().unwrap()
        }
    } else {
        address
    };
    Ok(format!("http://{}", SocketAddr::new(address, port)))
}
pub fn identity_from_props(value: &Value) -> ModelIdentity {
    ModelIdentity {
        path: value["model_path"].as_str().map(str::to_owned),
        alias: value["model_alias"].as_str().map(str::to_owned),
        model: value["default_generation_settings"]["model"]
            .as_str()
            .map(str::to_owned),
        context_window: value["default_generation_settings"]["n_ctx"].as_u64(),
        vision: value["modalities"]["vision"] == true,
    }
}
async fn get(http: &ProviderHttp, url: String) -> Result<(u16, Vec<u8>), DomainError> {
    let (_source, mut cancel) = cancellation();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut response = http
        .send(
            HttpRequest {
                url,
                method: HttpMethod::Get,
                headers: HostHeaders::default(),
                body: vec![],
                max_response_bytes: 2 * 1024 * 1024,
            },
            deadline,
            &mut cancel,
        )
        .await?;
    let status = response.status();
    let mut body = vec![];
    while let Some(bytes) = response.chunk(deadline, &mut cancel).await? {
        body.extend(bytes);
    }
    Ok((status, body))
}
pub async fn observe(http: &ProviderHttp, base: &str) -> Result<Observation, DomainError> {
    let url = reqwest::Url::parse(base)
        .map_err(|_| DomainError::InvalidInput("runtime observer URL".into()))?;
    endpoint_origin(base)?;
    if url.path() != "/" || url.query().is_some() {
        return Err(DomainError::InvalidInput("runtime observer origin".into()));
    }
    let health = get(http, format!("{}/health", base.trim_end_matches('/'))).await;
    match health {
        Ok((status, _)) if (200..300).contains(&status) => {
            let (status, bytes) =
                get(http, format!("{}/props", base.trim_end_matches('/'))).await?;
            if !(200..300).contains(&status) {
                return Ok(Observation::Occupied);
            }
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| DomainError::InvalidInput("runtime props JSON".into()))?;
            Ok(Observation::Healthy(identity_from_props(&value)))
        }
        Ok(_) => Ok(Observation::Occupied),
        Err(DomainError::Unavailable(_)) => {
            // Only a literal loopback address can be tested as vacant. A failed remote
            // request, DNS error or unreadable listener identity is never vacancy evidence.
            let address = url
                .host_str()
                .and_then(|s| s.trim_matches(['[', ']']).parse::<IpAddr>().ok());
            if let Some(address) = address
                && address.is_loopback()
                && let Some(port) = url.port_or_known_default()
                && let Ok(listener) = TcpListener::bind(SocketAddr::new(address, port))
            {
                drop(listener);
                return Ok(Observation::Vacant);
            }
            Ok(Observation::Occupied)
        }
        Err(error) => Err(error),
    }
}
/// A dedicated runtime for the native supervisor's bounded blocking executor.
/// The closure must not be invoked directly on an async executor worker.
pub fn blocking_observer()
-> Result<impl FnMut(&LaunchPlan) -> Result<Observation, DomainError>, DomainError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| DomainError::Unavailable("runtime observation executor".into()))?;
    Ok(move |plan: &LaunchPlan| {
        let origin = origin(plan)?;
        let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(2))?;
        runtime.block_on(observe(&http, &origin))
    })
}
