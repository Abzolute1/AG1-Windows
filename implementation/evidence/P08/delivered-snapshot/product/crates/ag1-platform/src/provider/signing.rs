//! AWS Signature V4 for Bedrock. Credentials are supplied by the host vault and
//! signed only after final serialization. No unsigned Bedrock transport is sent.
use super::http::*;
use ag1_core::{DomainError, SecretBytes};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct AwsCredentials {
    pub access_key: SecretBytes,
    pub secret_key: SecretBytes,
    pub session_token: Option<SecretBytes>,
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn mac(key: &[u8], text: &str) -> Vec<u8> {
    let mut h = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts arbitrary key sizes");
    h.update(text.as_bytes());
    h.finalize().into_bytes().to_vec()
}
fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn canonical_path(path: &str) -> String {
    let mut parts = vec![];
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    let raw = format!(
        "{}{}{}",
        if path.starts_with('/') { "/" } else { "" },
        parts.join("/"),
        if !parts.is_empty() && path.ends_with('/') {
            "/"
        } else {
            ""
        }
    );
    encode(&raw).replace("%2F", "/")
}
/// The date is a host-clock UTC timestamp (YYYYMMDDTHHMMSSZ). It cannot be
/// supplied by a renderer or reused across retries without rebuilding the request.
pub fn sign_bedrock(
    request: &mut HttpRequest,
    credentials: &AwsCredentials,
    region: &str,
    date: &str,
) -> Result<(), DomainError> {
    if region.is_empty()
        || region.len() > 64
        || !region
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(invalid("AWS region"));
    }
    if date.len() != 16
        || date.as_bytes()[8] != b'T'
        || date.as_bytes()[15] != b'Z'
        || !date
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 8 || i == 15 || b.is_ascii_digit())
    {
        return Err(invalid("AWS signing timestamp"));
    }
    let access = std::str::from_utf8(credentials.access_key.expose())
        .map_err(|_| invalid("AWS access key"))?;
    if access.is_empty()
        || access.len() > 256
        || !access.bytes().all(|b| b.is_ascii_alphanumeric())
        || credentials.secret_key.expose().is_empty()
        || credentials.secret_key.expose().len() > 16 * 1024
    {
        return Err(invalid("AWS credentials"));
    }
    endpoint_origin(&request.url)?;
    let url = reqwest::Url::parse(&request.url).map_err(|_| invalid("AWS URL"))?;
    if url.scheme() != "https" {
        return Err(DomainError::PermissionDenied);
    }
    let mut host = url
        .host_str()
        .ok_or_else(|| invalid("AWS hostname"))?
        .to_owned();
    if let Some(port) = url.port() {
        host = format!("{host}:{port}");
    }
    request.headers.headers.remove("authorization");
    request.headers.headers.remove("x-amz-security-token");
    request.headers.insert_public("x-amz-date", date)?;
    let payload = hash(&request.body);
    request
        .headers
        .insert_public("x-amz-content-sha256", &payload)?;
    if let Some(token) = &credentials.session_token {
        request
            .headers
            .insert_secret("x-amz-security-token", token)?;
    }
    let mut canonical = BTreeMap::from([("host".to_owned(), host)]);
    for (name, value) in &request.headers.headers {
        let key = name.as_str();
        if [
            "authorization",
            "cache-control",
            "connection",
            "expect",
            "from",
            "keep-alive",
            "max-forwards",
            "pragma",
            "referer",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "user-agent",
            "x-amzn-trace-id",
        ]
        .contains(&key)
            || key.starts_with("proxy-")
            || key.starts_with("sec-")
        {
            continue;
        }
        canonical.insert(
            key.into(),
            value
                .to_str()
                .map_err(|_| invalid("AWS canonical header"))?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    let mut queries: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key.to_lowercase() != "x-amz-signature")
        .map(|(k, v)| format!("{}={}", encode(&k), encode(&v)))
        .collect();
    queries.sort();
    let names = canonical.keys().cloned().collect::<Vec<_>>().join(";");
    let headers = canonical
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let method = match request.method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
    };
    let canonical = format!(
        "{method}\n{}\n{}\n{headers}\n\n{names}\n{payload}",
        canonical_path(url.path()),
        queries.join("&")
    );
    let scope = format!("{}/{region}/bedrock/aws4_request", &date[..8]);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        hash(canonical.as_bytes())
    );
    let mut initial = b"AWS4".to_vec();
    initial.extend(credentials.secret_key.expose());
    let date_key = mac(&initial, &date[..8]);
    let region_key = mac(&date_key, region);
    let service_key = mac(&region_key, "bedrock");
    let signing_key = mac(&service_key, "aws4_request");
    let signature = mac(&signing_key, &to_sign)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    request.headers.insert_secret("authorization",&SecretBytes::new(format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={names}, Signature={signature}").into_bytes()))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    #[test]
    fn signatures_match_original_aws_sdk_with_encoded_paths_queries_and_session_tokens() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../../../fixtures/providers/aws.json"))
                .unwrap();
        let creds = &fixture["credentials"];
        let credentials = AwsCredentials {
            access_key: SecretBytes::new(
                creds["accessKeyId"].as_str().unwrap().as_bytes().to_vec(),
            ),
            secret_key: SecretBytes::new(
                creds["secretAccessKey"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
            ),
            session_token: Some(SecretBytes::new(
                creds["sessionToken"].as_str().unwrap().as_bytes().to_vec(),
            )),
        };
        for group in fixture["groups"].as_array().unwrap() {
            let input = &group["input"];
            let mut url = reqwest::Url::parse(&format!(
                "https://{}{}",
                input["hostname"].as_str().unwrap(),
                input["path"].as_str().unwrap()
            ))
            .unwrap();
            for (k, v) in input["query"].as_object().unwrap() {
                if let Some(a) = v.as_array() {
                    for v in a {
                        url.query_pairs_mut().append_pair(k, v.as_str().unwrap());
                    }
                } else {
                    url.query_pairs_mut().append_pair(k, v.as_str().unwrap());
                }
            }
            let mut headers = HostHeaders::default();
            for (k, v) in input["headers"].as_object().unwrap() {
                if k != "host" {
                    headers.insert_public(k, v.as_str().unwrap()).unwrap();
                }
            }
            let mut request = HttpRequest {
                url: url.to_string(),
                method: HttpMethod::Post,
                headers,
                body: input["body"].as_str().unwrap().as_bytes().to_vec(),
                max_response_bytes: 1024,
            };
            sign_bedrock(
                &mut request,
                &credentials,
                group["region"].as_str().unwrap(),
                group["date"].as_str().unwrap(),
            )
            .unwrap();
            for name in [
                "authorization",
                "x-amz-date",
                "x-amz-content-sha256",
                "x-amz-security-token",
            ] {
                assert_eq!(
                    request.headers.headers[name].to_str().unwrap(),
                    group["headers"][name],
                    "{name} {}",
                    input["path"]
                );
            }
        }
    }
}
