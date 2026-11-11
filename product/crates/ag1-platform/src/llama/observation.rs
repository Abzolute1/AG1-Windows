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

fn idle_from_slots(value: &Value) -> Result<bool, DomainError> {
    let slots = value.as_array().filter(|slots| !slots.is_empty() && slots.len() <= 128)
        .ok_or_else(|| DomainError::InvalidInput("runtime slots JSON".into()))?;
    let mut idle = true;
    for slot in slots {
        let processing = slot["is_processing"].as_bool()
            .ok_or_else(|| DomainError::InvalidInput("runtime slot state".into()))?;
        idle &= !processing;
    }
    Ok(idle)
}

pub(crate) async fn generation_idle(http: &ProviderHttp, base: &str) -> Result<bool, DomainError> {
    let (status, bytes) = get(http, format!("{}/slots", base.trim_end_matches('/'))).await?;
    if !(200..300).contains(&status) {
        return Err(DomainError::Unavailable("Owned generation status unavailable".into()));
    }
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| DomainError::InvalidInput("runtime slots JSON".into()))?;
    idle_from_slots(&value)
}

/// Invoked only on the bounded blocking executor, never an async UI worker.
pub(crate) fn blocking_generation_idle(plan: &LaunchPlan) -> Result<bool, DomainError> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
        .map_err(|_| DomainError::Unavailable("runtime observation executor".into()))?;
    let base = origin(plan)?;
    let http = ProviderHttp::new(std::slice::from_ref(&base), Duration::from_secs(2))?;
    runtime.block_on(generation_idle(&http, &base))
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cancellation_requires_every_slot_to_be_explicitly_idle() {
        assert_eq!(idle_from_slots(&json!([{"is_processing":false}])), Ok(true));
        assert_eq!(idle_from_slots(&json!([{"is_processing":false},{"is_processing":true}])), Ok(false));
        for value in [json!([]), json!({}), json!([{}]), json!([{"is_processing":"false"}])] {
            assert!(idle_from_slots(&value).is_err());
        }
    }

    #[tokio::test]
    async fn cancellation_observation_is_a_read_only_slots_request() {
        use std::io::{BufRead, BufReader, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "GET /slots HTTP/1.1\r\n");
            loop {
                line.clear(); reader.read_line(&mut line).unwrap();
                if line == "\r\n" { break; }
            }
            let body = r#"[{"id":0,"is_processing":false}]"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let http = ProviderHttp::new(std::slice::from_ref(&base), Duration::from_secs(2)).unwrap();
        assert_eq!(generation_idle(&http, &base).await, Ok(true));
        server.join().unwrap();
    }
}
