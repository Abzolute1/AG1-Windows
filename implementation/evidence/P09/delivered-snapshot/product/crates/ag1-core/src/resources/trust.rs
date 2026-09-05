//! Project extension trust selection. Inputs are captured by the host from its
//! configuration, environment and USER settings. Project manifests cannot grant
//! trust. A selection is data, not an executable process capability.
use crate::DomainError;
use serde::Serialize;
pub struct ExtensionPath {
    pub path: String,
    pub canonical: String,
}
pub struct Policy<'a> {
    pub project_config_dir: &'a str,
    pub canonical_project_dir: &'a str,
    pub explicit: Option<bool>,
    pub environment: Option<&'a str>,
    pub user_trusted_projects: &'a [String],
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub project_config_dir: String,
    pub extension_paths: Vec<String>,
}
#[derive(Serialize)]
pub struct Error {
    pub path: String,
    pub error: String,
}
#[derive(Serialize)]
pub struct Selection {
    pub allowed: Vec<String>,
    pub errors: Vec<Error>,
    #[serde(rename = "pendingTrustPrompt", skip_serializing_if = "Option::is_none")]
    pub pending: Option<Prompt>,
}
fn valid(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 8192
        && !path.contains('\0')
        && !path.split('/').any(|s| s == "." || s == "..")
}
fn within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root.trim_end_matches('/'))
            .is_some_and(|s| s.starts_with('/'))
}
pub fn select(paths: &[ExtensionPath], policy: Policy<'_>) -> Result<Selection, DomainError> {
    if paths.len() > 256
        || policy.user_trusted_projects.len() > 4096
        || policy.environment.is_some_and(|s| s.len() > 8192)
    {
        return Err(DomainError::LimitExceeded("extension trust inputs".into()));
    }
    if !valid(policy.project_config_dir)
        || !valid(policy.canonical_project_dir)
        || paths
            .iter()
            .any(|p| !valid(&p.path) || !valid(&p.canonical))
        || policy.user_trusted_projects.iter().any(|p| !valid(p))
    {
        return Err(DomainError::InvalidInput(
            "host-normalized extension trust paths".into(),
        ));
    }
    let environment = policy
        .environment
        .and_then(|s| match s.to_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        });
    let trusted = policy.explicit.or(environment).unwrap_or_else(|| {
        policy
            .user_trusted_projects
            .iter()
            .any(|p| p == policy.project_config_dir)
    });
    let mut allowed = vec![];
    let mut blocked = vec![];
    for path in paths {
        // Resolve alias provenance too. A user-directory symlink to an untrusted
        // project entry must not bypass the project gate by changing its spelling.
        if !trusted
            && (within(&path.path, policy.project_config_dir)
                || within(&path.canonical, policy.canonical_project_dir))
        {
            blocked.push(path.path.clone());
        } else {
            allowed.push(path.path.clone());
        }
    }
    let mut errors = vec![];
    let pending = if blocked.is_empty() {
        None
    } else {
        errors.push(Error{path:policy.project_config_dir.into(),error:format!("Skipped {} project-local extension(s) under {}: project is not trusted. Trust the project in interactive mode, add it to trustedProjects in user settings, or set AG1_TRUST_PROJECT=1 to load project extensions.",blocked.len(),policy.project_config_dir)});
        Some(Prompt {
            project_config_dir: policy.project_config_dir.into(),
            extension_paths: blocked,
        })
    };
    Ok(Selection {
        allowed,
        errors,
        pending,
    })
}
