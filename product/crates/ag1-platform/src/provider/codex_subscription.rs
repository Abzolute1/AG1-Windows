//! ChatGPT subscription discovery. Credentials stay in the native host; Codex
//! remains the owner of its login/refresh file, which AG1 never copies or edits.
use super::http::*;
use ag1_core::{DomainError, SecretBytes};
#[cfg(target_os = "linux")]
use base64::Engine;
use serde_json::{Value, json};
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{path::Path, time::Duration};

pub const BASE: &str = "https://chatgpt.com/backend-api";
const CATALOG: &str = "https://chatgpt.com/backend-api/codex/models?client_version=0.154.0";
fn invalid() -> DomainError {
    DomainError::InvalidInput("ChatGPT subscription session or model catalog".into())
}

/// Let the official login owner refresh its own token, without starting a turn,
/// copying refresh tokens, opening a browser, or falling back to API billing.
#[cfg(target_os = "linux")]
pub fn credential_with_refresh(
    path: &Path,
    cli: Option<&Path>,
    model: &Value,
) -> Result<SecretBytes, DomainError> {
    match credential(path, model) {
        Err(DomainError::Unavailable(message))
            if message.starts_with("Your ChatGPT session needs refreshing") && cli.is_some() =>
        {
            refresh_session(path, cli.unwrap())?;
            credential(path, model)
        }
        result => result,
    }
}

#[cfg(windows)]
pub fn credential_with_refresh(
    _: &Path,
    _: Option<&Path>,
    _: &Value,
) -> Result<SecretBytes, DomainError> {
    // Do not inspect a Codex-owned credential file until the Windows ACL and
    // owned-child implementation are in place. A visible unavailable result is
    // safer than accepting a file with Unix-only permission checks removed.
    Err(DomainError::Unavailable(
        "ChatGPT subscription sign-in is not yet available in the Windows-native runtime".into(),
    ))
}

#[cfg(target_os = "linux")]
fn refresh_session(path: &Path, cli: &Path) -> Result<(), DomainError> {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };
    if !cli.is_absolute() || path.file_name().is_none_or(|n| n != "auth.json") {
        return Err(DomainError::PermissionDenied);
    }
    let meta = std::fs::metadata(cli).map_err(crate::io_error)?;
    if !meta.is_file()
        || ![0, unsafe { libc::geteuid() }].contains(&meta.uid())
        || meta.mode() & 0o022 != 0
    {
        return Err(DomainError::PermissionDenied);
    }
    struct Owned(std::process::Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }
    let mut command = Command::new(cli);
    command
        .args(["-c", "forced_login_method=\"chatgpt\"", "app-server"])
        .env_clear()
        .env("CODEX_HOME", path.parent().ok_or_else(invalid)?)
        .env("PATH", "/usr/bin:/bin")
        .current_dir(path.parent().ok_or_else(invalid)?)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(home) = std::env::var_os("HOME") {
        command.env("HOME", home);
    }
    let mut child = Owned(command.spawn().map_err(|_| {
        DomainError::Unavailable("Cannot start the configured Codex sign-in manager".into())
    })?);
    let mut input = child.0.stdin.take().ok_or_else(invalid)?;
    let output = child.0.stdout.take().ok_or_else(invalid)?;
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        for _ in 0..32 {
            let mut line = Vec::new();
            if output
                .by_ref()
                .take(65537)
                .read_until(b'\n', &mut line)
                .is_err()
                || line.is_empty()
                || line.len() > 65536
            {
                break;
            }
            if send.send(serde_json::from_slice::<Value>(&line)).is_err() {
                break;
            }
        }
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let result = (|| {
        writeln!(input,"{}",json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"ag1","version":"0.1.0"},"capabilities":{"experimentalApi":false}}})).map_err(crate::io_error)?;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let value = receive
                .recv_timeout(remaining)
                .map_err(|_| {
                    DomainError::Unavailable(
                        "ChatGPT sign-in refresh did not complete; sign in through Codex and retry"
                            .into(),
                    )
                })?
                .map_err(|_| invalid())?;
            if value.get("error").is_some() {
                return Err(DomainError::Unavailable(
                    "ChatGPT sign-in refresh failed; sign in through Codex and retry".into(),
                ));
            }
            if value["id"] == 1 {
                writeln!(input, "{}", json!({"method":"initialized"})).map_err(crate::io_error)?;
                writeln!(
                    input,
                    "{}",
                    json!({"id":2,"method":"account/read","params":{"refreshToken":true}})
                )
                .map_err(crate::io_error)?;
            }
            if value["id"] == 2 {
                if value["result"]["account"]["type"] != "chatgpt" {
                    return Err(DomainError::Unavailable(
                        "Sign in to Codex with ChatGPT to use your subscription in AG1".into(),
                    ));
                }
                return Ok(());
            }
        }
    })();
    drop(input);
    drop(child);
    let _ = reader.join();
    result
}

#[cfg(target_os = "linux")]
pub fn credential(path: &Path, model: &Value) -> Result<SecretBytes, DomainError> {
    if model["provider"] != "openai-codex"
        || model["api"] != "openai-codex-responses"
        || model["baseUrl"] != BASE
    {
        return Err(DomainError::PermissionDenied);
    }
    if !path.is_absolute() {
        return Err(DomainError::PermissionDenied);
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(crate::io_error)?;
    let metadata = file.metadata().map_err(crate::io_error)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(DomainError::PermissionDenied);
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(crate::io_error)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if value["auth_mode"] != "chatgpt" {
        return Err(DomainError::Unavailable(
            "Sign in to Codex with ChatGPT to use your subscription in AG1".into(),
        ));
    }
    let token = value["tokens"]["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)?;
    let payload = token.split('.').nth(1).ok_or_else(invalid)?;
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| invalid())?,
    )
    .map_err(|_| invalid())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_secs();
    if claims["exp"].as_u64().is_none_or(|exp| exp <= now + 30) {
        return Err(DomainError::Unavailable(
            "Your ChatGPT session needs refreshing in Codex; retry after refreshing the sign-in"
                .into(),
        ));
    }
    let secret = SecretBytes::new(token.as_bytes().to_vec());
    let account = super::auth::codex_account_id(&secret)?;
    if value["tokens"]["account_id"]
        .as_str()
        .is_some_and(|id| id != account)
    {
        return Err(invalid());
    }
    Ok(secret)
}

#[cfg(windows)]
pub fn credential(_: &Path, model: &Value) -> Result<SecretBytes, DomainError> {
    if model["provider"] != "openai-codex"
        || model["api"] != "openai-codex-responses"
        || model["baseUrl"] != BASE
    {
        return Err(DomainError::PermissionDenied);
    }
    Err(DomainError::Unavailable(
        "ChatGPT subscription sign-in is not yet available in the Windows-native runtime".into(),
    ))
}

/// Only picker-visible text models are admitted. API-key availability is a
/// different entitlement; e.g. subscription-only Spark must not be filtered out.
pub fn models(value: &Value) -> Result<Vec<Value>, DomainError> {
    let rows = value["models"]
        .as_array()
        .filter(|r| r.len() <= 1024)
        .ok_or_else(invalid)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for row in rows {
        if row["visibility"] != "list" {
            continue;
        }
        let id = row["slug"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
            .ok_or_else(invalid)?;
        if !seen.insert(id) {
            return Err(invalid());
        }
        let input = row["input_modalities"].as_array().ok_or_else(invalid)?;
        if !input.contains(&json!("text")) {
            continue;
        }
        let context = row["context_window"]
            .as_u64()
            .filter(|n| *n >= 1024 && *n <= 16_000_000)
            .ok_or_else(invalid)?;
        let mut modes = Vec::new();
        for level in row["supported_reasoning_levels"]
            .as_array()
            .ok_or_else(invalid)?
        {
            let effort = level["effort"].as_str().ok_or_else(invalid)?;
            // Ultra enables Codex subagent orchestration; it is not a wire effort.
            if ["low", "medium", "high", "xhigh", "max"].contains(&effort)
                && !modes.contains(&json!(effort))
            {
                modes.push(json!(effort));
            }
        }
        let default = row["default_reasoning_level"]
            .as_str()
            .ok_or_else(invalid)?;
        if modes.is_empty() || !modes.contains(&json!(default)) {
            return Err(invalid());
        }
        out.push(json!({"id":id,"name":row["display_name"],"provider":"openai-codex","api":"openai-codex-responses","baseUrl":BASE,"reasoning":true,"thinking":{"modes":modes,"default":default},"input":input.iter().filter(|v| **v=="text" || **v=="image").collect::<Vec<_>>(),"contextWindow":context,"maxTokens":row["max_output_tokens"].as_u64().unwrap_or(32768).min(context),"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}}));
    }
    if out.is_empty() {
        return Err(DomainError::Unavailable(
            "No subscription models are available for this account".into(),
        ));
    }
    Ok(out)
}

pub async fn discover(path: &Path) -> Result<Vec<Value>, DomainError> {
    let token = credential(
        path,
        &json!({"provider":"openai-codex","api":"openai-codex-responses","baseUrl":BASE}),
    )?;
    let mut headers = HostHeaders::default();
    headers.insert_secret(
        "authorization",
        &SecretBytes::new([b"Bearer ".as_slice(), token.expose()].concat()),
    )?;
    headers.insert_secret(
        "chatgpt-account-id",
        &SecretBytes::new(super::auth::codex_account_id(&token)?.into_bytes()),
    )?;
    headers.insert_public("accept", "application/json")?;
    headers.insert_public("originator", "ag1")?;
    let http = ProviderHttp::new(&["https://chatgpt.com".into()], Duration::from_secs(10))?;
    let (_source, mut cancel) = cancellation();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut response = http
        .send(
            HttpRequest {
                url: CATALOG.into(),
                method: HttpMethod::Get,
                headers,
                body: vec![],
                max_response_bytes: 4 * 1024 * 1024,
            },
            deadline,
            &mut cancel,
        )
        .await?;
    if response.status() != 200 {
        return Err(DomainError::Unavailable(format!(
            "ChatGPT model discovery returned HTTP {}; refresh your ChatGPT sign-in if needed",
            response.status()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk(deadline, &mut cancel).await? {
        bytes.extend(chunk);
    }
    models(&serde_json::from_slice(&bytes).map_err(|_| invalid())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscription_catalog_preserves_real_efforts_and_subscription_only_models() {
        let fixture = json!({"models":[{"slug":"fixture","display_name":"Fixture","visibility":"list","supported_in_api":false,"input_modalities":["text"],"context_window":128000,"default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"},{"effort":"xhigh"},{"effort":"max"},{"effort":"ultra"}]},{"slug":"hidden","visibility":"hide"}]});
        let models = models(&fixture).unwrap();
        assert_eq!(models.len(), 1);
        let m = &models[0];
        let controls = super::super::reasoning_controls::declared(m);
        assert_eq!(controls["default"], "high");
        assert_eq!(controls["modes"].as_array().unwrap().len(), 4);
        for mode in ["low", "high", "xhigh", "max"] {
            assert_eq!(
                super::super::requests::session_options(m, mode, None).unwrap()["reasoningEffort"],
                mode
            );
        }
        let mut bad = fixture.clone();
        bad["models"][0]["default_reasoning_level"] = json!("ultra");
        assert!(super::models(&bad).is_err());
    }
    #[test]
    fn subscription_credentials_never_follow_a_model_to_another_origin() {
        assert!(matches!(
            credential(
                Path::new("/unread"),
                &json!({"provider":"openai-codex","api":"openai-codex-responses","baseUrl":"https://untrusted.invalid"})
            ),
            Err(DomainError::PermissionDenied)
        ));
    }
    #[test]
    fn api_and_oauth_efforts_are_exact_and_unsupported_choices_are_rejected() {
        for api in [
            "openai-responses",
            "azure-openai-responses",
            "openai-codex-responses",
        ] {
            let mut model = json!({"id":"fixture-model-without-name-heuristics","api":api,"maxTokens":4096,"thinking":{"modes":["low","medium","high","xhigh","max"],"default":"medium"}});
            for effort in ["low", "medium", "high", "xhigh", "max"] {
                assert_eq!(
                    super::super::requests::session_options(
                        &model,
                        effort,
                        Some(&json!({"reasoningEffort":"low"}))
                    )
                    .unwrap()["reasoningEffort"],
                    effort
                );
            }
            model["thinking"]["modes"] = json!(["low", "medium", "high", "xhigh"]);
            for unsupported in ["max", "off", "minimal"] {
                assert!(
                    super::super::requests::session_options(&model, unsupported, None).is_err()
                );
            }
            model["thinking"]["modes"] = json!([{"id":"off","nativeValue":"none"},"minimal"]);
            assert_eq!(
                super::super::requests::session_options(&model, "off", None).unwrap()["reasoningEffort"],
                "none"
            );
            assert_eq!(
                super::super::requests::session_options(&model, "minimal", None).unwrap()["reasoningEffort"],
                "minimal"
            );
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn subscription_refresh_uses_the_login_owner_and_rereads_its_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let token = |expiry| {
            format!("fixture.{}.signature",base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json!({"exp":expiry,"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}}).to_string()))
        };
        let expired = token(1u64);
        let fresh = token(9_999_999_999u64);
        std::fs::write(&path,json!({"auth_mode":"chatgpt","tokens":{"access_token":expired,"account_id":"fixture-account"}}).to_string()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let cli = dir.path().join("login-owner");
        std::fs::write(&cli,format!("#!/usr/bin/python3\nimport sys,json,os\nfrom pathlib import Path\np=Path(os.environ['CODEX_HOME'])/'auth.json'\nfor line in sys.stdin:\n r=json.loads(line)\n if r.get('id')==1: print(json.dumps({{'id':1,'result':{{}}}}),flush=True)\n if r.get('id')==2:\n  assert r['method']=='account/read' and r['params']['refreshToken'] is True\n  d=json.loads(p.read_text()); d['tokens']['access_token']={fresh:?}; p.write_text(json.dumps(d))\n  print(json.dumps({{'id':2,'result':{{'account':{{'type':'chatgpt'}}}}}}),flush=True)\n")).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let model =
            json!({"provider":"openai-codex","api":"openai-codex-responses","baseUrl":BASE});
        assert!(credential(&path, &model).is_err());
        assert_eq!(
            credential_with_refresh(&path, Some(&cli), &model)
                .unwrap()
                .expose(),
            fresh.as_bytes()
        );
        // A later login is picked up without restarting AG1 or rewriting a vault.
        let next = token(9_999_999_998u64);
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["tokens"]["access_token"] = json!(next);
        std::fs::write(&path, value.to_string()).unwrap();
        assert_eq!(credential(&path, &model).unwrap().expose(), next.as_bytes());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            credential(&path, &model),
            Err(DomainError::PermissionDenied)
        ));
    }
}
