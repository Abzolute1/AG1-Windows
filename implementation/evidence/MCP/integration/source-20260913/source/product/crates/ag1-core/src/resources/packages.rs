//! Package source identities and command configuration are pure data. Installation
//! and trust require separate host capabilities; parsing never performs network I/O.
use crate::{
    DomainError,
    tools::research::{rx, trim},
};
use serde::{Deserialize, Serialize};
use url::Url;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    Npm {
        spec: String,
        name: String,
        pinned: bool,
    },
    Local {
        path: String,
    },
    Git {
        repo: String,
        host: String,
        path: String,
        #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        pinned: bool,
    },
}
pub fn is_local(input: &str) -> bool {
    let text = trim(input);
    !["npm:", "git:", "github:", "http:", "https:", "ssh:"]
        .iter()
        .any(|p| text.starts_with(p))
}
fn safe(path: &str) -> bool {
    !rx(r"^[A-Za-z]:").is_match(path)
        && !path.contains('\\')
        && path.split('/').all(|s| !s.is_empty() && s != "..")
}
fn split_ref(input: &str) -> (String, Option<String>) {
    if let Some(tail) = input.strip_prefix("git@")
        && let Some((host, path)) = tail.split_once(':')
    {
        if let Some((repo, reference)) = path.split_once('@')
            && !repo.is_empty()
            && !reference.is_empty()
        {
            return (format!("git@{host}:{repo}"), Some(reference.into()));
        }
        return (input.into(), None);
    }
    if input.contains("://") {
        if let Ok(mut url) = Url::parse(input) {
            let path = url.path().trim_start_matches('/').to_owned();
            if let Some((path, reference)) = path.split_once('@')
                && !path.is_empty()
                && !reference.is_empty()
            {
                url.set_path(&format!("/{path}"));
                return (
                    url.to_string().trim_end_matches('/').into(),
                    Some(reference.into()),
                );
            }
        }
        return (input.into(), None);
    }
    if let Some((host, path)) = input.split_once('/')
        && let Some((repo, reference)) = path.split_once('@')
        && !repo.is_empty()
        && !reference.is_empty()
    {
        return (format!("{host}/{repo}"), Some(reference.into()));
    }
    (input.into(), None)
}
fn hosted(input: &str) -> Option<(String, String, Option<String>)> {
    let (without_fragment, fragment) = input
        .split_once('#')
        .map(|(a, b)| (a, (!b.is_empty()).then(|| b.to_owned())))
        .unwrap_or((input, None));
    let (host, path) = if let Some((host, path)) = [
        ("github:", "github.com"),
        ("gitlab:", "gitlab.com"),
        ("bitbucket:", "bitbucket.org"),
        ("gist:", "gist.github.com"),
    ]
    .iter()
    .find_map(|(prefix, host)| {
        without_fragment
            .strip_prefix(prefix)
            .map(|p| ((*host).to_owned(), p.to_owned()))
    }) {
        (host, path)
    } else if let Some(tail) = without_fragment.strip_prefix("git@") {
        let (host, path) = tail.split_once(':')?;
        (host.into(), path.into())
    } else if without_fragment.contains("://") {
        let url = Url::parse(without_fragment).ok()?;
        (url.host_str()?.into(), url.path().trim_matches('/').into())
    } else {
        let pieces = without_fragment.split('/').collect::<Vec<_>>();
        if pieces.len() == 2 && !pieces[0].contains(['.', ':']) {
            ("github.com".into(), without_fragment.into())
        } else {
            let (host, path) = without_fragment.split_once('/')?;
            (host.into(), path.into())
        }
    };
    let mut parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let mut reference = fragment;
    match host.as_str() {
        "github.com" => {
            if parts.len() >= 4 && parts[2] == "tree" {
                reference = Some(parts[3..].join("/"));
                parts.truncate(2);
            }
            if parts.len() != 2 {
                return None;
            }
        }
        "bitbucket.org" => {
            if parts.len() != 2 {
                return None;
            }
        }
        "gitlab.com" => {
            if parts.len() < 2 || parts.contains(&"-") {
                return None;
            }
        }
        "gist.github.com" => {
            if parts.len() != 2 {
                return None;
            }
        }
        _ => return None,
    }
    let path = parts.join("/");
    let path = path.strip_suffix(".git").unwrap_or(&path).to_owned();
    if !safe(&path) {
        return None;
    }
    Some((host, path, reference))
}
pub fn git(input: &str) -> Result<Option<Source>, DomainError> {
    if input.len() > 8192 || input.contains('\0') {
        return Err(DomainError::InvalidInput("package source".into()));
    }
    let trimmed = trim(input);
    let prefixed = trimmed.starts_with("git:");
    let input = if prefixed {
        trim(&trimmed[4..])
    } else {
        trimmed
    };
    if !prefixed && !rx(r"(?i)^(https?|ssh|git)://").is_match(input) {
        return Ok(None);
    }
    let (split, reference) = split_ref(input);
    let candidate = if let Some(reference) = &reference {
        format!("{split}#{reference}")
    } else {
        input.into()
    };
    if let Some((host, path, hosted_ref)) = hosted(&candidate) {
        let reference = hosted_ref.or(reference);
        let repo = if ["http://", "https://", "ssh://", "git://", "git@"]
            .iter()
            .any(|p| split.starts_with(p))
        {
            split
        } else {
            format!("https://{split}")
        };
        return Ok(Some(Source::Git {
            repo,
            host,
            path,
            pinned: reference.is_some(),
            reference,
        }));
    }
    let (mut repo, reference) = split_ref(input);
    let (host, path) = if let Some(tail) = repo.strip_prefix("git@") {
        if let Some((host, path)) = tail.split_once(':') {
            (host.to_owned(), path.to_owned())
        } else {
            return Ok(None);
        }
    } else if ["http://", "https://", "ssh://", "git://"]
        .iter()
        .any(|p| repo.starts_with(p))
    {
        let Ok(url) = Url::parse(&repo) else {
            return Ok(None);
        };
        let Some(host) = url.host_str() else {
            return Ok(None);
        };
        (
            host.to_owned(),
            url.path().trim_start_matches('/').to_owned(),
        )
    } else {
        let Some((host, path)) = repo.split_once('/') else {
            return Ok(None);
        };
        if !host.contains('.') && host != "localhost" {
            return Ok(None);
        }
        let host = host.to_owned();
        let path = path.to_owned();
        repo = format!("https://{repo}");
        (host, path)
    };
    let path = path
        .strip_suffix(".git")
        .unwrap_or(&path)
        .trim_matches('/')
        .to_owned();
    if host.is_empty() || path.split('/').count() < 2 || !safe(&path) {
        return Ok(None);
    }
    Ok(Some(Source::Git {
        repo,
        host,
        path,
        pinned: reference.is_some(),
        reference,
    }))
}
pub fn parse(input: &str) -> Result<Source, DomainError> {
    if input.len() > 8192 || input.contains('\0') {
        return Err(DomainError::InvalidInput("package source".into()));
    }
    if let Some(spec) = input.strip_prefix("npm:") {
        let spec = trim(spec);
        let captures = rx(r"^(@?[^@]+(?:/[^@]+)?)(?:@(.+))?$").captures(spec);
        let name = captures
            .as_ref()
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
            .unwrap_or(spec);
        if name.starts_with('/') || !safe(name.strip_prefix('@').unwrap_or(name)) {
            return Err(DomainError::InvalidInput(format!(
                "Refusing unsafe npm package name: {name}"
            )));
        }
        return Ok(Source::Npm {
            spec: spec.into(),
            name: name.into(),
            pinned: captures.as_ref().is_some_and(|c| c.get(2).is_some()),
        });
    }
    if !is_local(input)
        && let Some(source) = git(input)?
    {
        return Ok(source);
    }
    Ok(Source::Local { path: input.into() })
}
impl Source {
    /// Host supplies the already normalized, capability-admitted local path.
    pub fn identity(&self, local_path: &str) -> String {
        match self {
            Self::Npm { name, .. } => format!("npm:{name}"),
            Self::Git { host, path, .. } => format!("git:{host}/{path}"),
            Self::Local { .. } => format!("local:{local_path}"),
        }
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Command {
    pub command: String,
    pub args: Vec<String>,
}
pub fn npm_command(configured: Option<&[String]>) -> Result<Command, DomainError> {
    let configured = configured.unwrap_or(&[]);
    if configured.len() > 64
        || configured
            .iter()
            .any(|s| s.len() > 8192 || s.contains('\0'))
    {
        return Err(DomainError::LimitExceeded("npm command arguments".into()));
    }
    if configured.is_empty() {
        return Ok(Command {
            command: "npm".into(),
            args: vec![],
        });
    }
    if configured[0].is_empty() {
        return Err(DomainError::InvalidInput(
            "Invalid npmCommand: first array entry must be a non-empty command".into(),
        ));
    }
    Ok(Command {
        command: configured[0].clone(),
        args: configured[1..].to_vec(),
    })
}

/// Validate the bounded host-captured settings value before cloning it for an edit.
pub fn validate_settings_document(value: &serde_json::Value) -> Result<(), DomainError> {
    crate::tools::schema::bounded(value, 1024 * 1024, 64)
}

/// The SDK compares the requested version text exactly, including range strings.
/// An absent or malformed installed version never satisfies a pin.
pub fn pinned_npm_matches(
    source: &Source,
    manifest: Option<&serde_json::Value>,
) -> Result<bool, DomainError> {
    let Source::Npm { spec, .. } = source else {
        return Err(DomainError::InvalidInput("npm package source".into()));
    };
    let Some(installed) = manifest
        .and_then(|v| v.get("version"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return Ok(false);
    };
    let captures = rx(r"^(@?[^@]+(?:/[^@]+)?)(?:@(.+))?$").captures(spec);
    let pinned = captures.as_ref().and_then(|c| c.get(2)).map(|m| m.as_str());
    Ok(pinned.is_none_or(|p| p == installed))
}
