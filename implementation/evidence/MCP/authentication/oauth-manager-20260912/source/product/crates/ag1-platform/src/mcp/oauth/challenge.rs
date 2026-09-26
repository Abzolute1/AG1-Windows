//! Bounded WWW-Authenticate parsing. Untrusted descriptions are never surfaced.
use super::{DomainError, invalid, scopes};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Eq)]
pub struct Challenge {
    pub resource_metadata: Option<String>,
    pub scopes: Option<BTreeSet<String>>,
    pub insufficient_scope: bool,
}
fn token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
fn parameter(input: &str) -> Result<(String, String), DomainError> {
    let input = input.trim();
    let end = input.bytes().take_while(|b| token(*b)).count();
    if end == 0 {
        return Err(invalid("OAuth challenge parameter"));
    }
    let name = input[..end].to_ascii_lowercase();
    let value = input[end..]
        .trim_start()
        .strip_prefix('=')
        .ok_or_else(|| invalid("OAuth challenge parameter"))?
        .trim();
    let value = if value.starts_with('"') {
        let bytes = value.as_bytes();
        if bytes.len() < 2 || *bytes.last().unwrap() != b'"' {
            return Err(invalid("OAuth quoted challenge"));
        }
        let mut output = String::new();
        let mut i = 1;
        while i < bytes.len() - 1 {
            let mut b = bytes[i];
            if b == b'\\' {
                i += 1;
                if i >= bytes.len() - 1 {
                    return Err(invalid("OAuth quoted challenge escape"));
                }
                b = bytes[i];
            } else if b == b'"' {
                return Err(invalid("OAuth quoted challenge"));
            }
            if !(0x20..=0x7e).contains(&b) {
                return Err(invalid("OAuth challenge encoding"));
            }
            output.push(char::from(b));
            i += 1;
        }
        output
    } else {
        if value.is_empty() || !value.bytes().all(token) {
            return Err(invalid("OAuth challenge token"));
        }
        value.to_owned()
    };
    Ok((name, value))
}
/// Each input is one native response header. No renderer-supplied header path.
pub fn parse(headers: &[&str]) -> Result<Option<Challenge>, DomainError> {
    if headers.len() > 16 || headers.iter().map(|s| s.len()).sum::<usize>() > 16 * 1024 {
        return Err(DomainError::LimitExceeded(
            "MCP OAuth challenge headers".into(),
        ));
    }
    let mut found = None;
    for header in headers {
        if header.bytes().any(|b| b < 0x20 && b != b'\t' || b >= 0x7f) {
            return Err(invalid("OAuth challenge encoding"));
        }
        let mut quoted = false;
        let mut escaped = false;
        let mut start = 0;
        let mut segments = Vec::new();
        for (i, b) in header.bytes().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }
            if quoted && b == b'\\' {
                escaped = true;
                continue;
            }
            if b == b'"' {
                quoted = !quoted;
            }
            if !quoted && b == b',' {
                segments.push(&header[start..i]);
                start = i + 1;
            }
        }
        if quoted || escaped {
            return Err(invalid("OAuth challenge quoting"));
        }
        segments.push(&header[start..]);
        if segments.len() > 64 {
            return Err(DomainError::LimitExceeded(
                "MCP OAuth challenge parameters".into(),
            ));
        }
        let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
        for segment in segments {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            let end = segment.bytes().take_while(|b| token(*b)).count();
            if end == 0 {
                return Err(invalid("OAuth challenge scheme"));
            }
            let rest = &segment[end..];
            if rest.trim_start().starts_with('=') {
                groups
                    .last_mut()
                    .ok_or_else(|| invalid("OAuth challenge without scheme"))?
                    .1
                    .push(segment);
            } else {
                if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
                    return Err(invalid("OAuth challenge scheme"));
                }
                groups.push((segment[..end].to_ascii_lowercase(), vec![rest.trim()]));
            }
        }
        for (scheme, parts) in groups {
            if scheme != "bearer" {
                continue;
            }
            let mut params = BTreeMap::new();
            for part in parts.into_iter().filter(|s| !s.is_empty()) {
                let (name, value) = parameter(part)?;
                if params.insert(name, value).is_some() {
                    return Err(invalid("duplicate OAuth challenge parameter"));
                }
            }
            let next = Challenge {
                resource_metadata: params.remove("resource_metadata"),
                scopes: params.get("scope").map(|s| scopes(s)).transpose()?,
                insufficient_scope: params
                    .get("error")
                    .is_some_and(|s| s == "insufficient_scope"),
            };
            if next
                .resource_metadata
                .as_ref()
                .is_some_and(|s| s.is_empty())
            {
                return Err(invalid("empty OAuth resource metadata URL"));
            }
            if found.as_ref().is_some_and(|old| old != &next) {
                return Err(invalid("ambiguous OAuth Bearer challenges"));
            }
            found = Some(next);
        }
    }
    Ok(found)
}
