//! Preview grants name one owned run and one exact loopback origin.
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grant {
    pub operation: String,
    pub origin: String,
    pub session: String,
    pub open: bool,
}
impl Grant {
    pub fn new(operation: &str, session: &str, url: &str) -> Result<Self, DomainError> {
        let parsed = reqwest::Url::parse(url).map_err(|_| DomainError::PermissionDenied)?;
        if parsed.scheme() != "http"
            || parsed.host_str() != Some("127.0.0.1")
            || parsed.port().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            operation: operation.into(),
            origin: parsed.origin().ascii_serialization(),
            session: session.into(),
            open: true,
        })
    }
    pub fn allows(&self, operation: &str, url: &str) -> bool {
        self.open
            && self.operation == operation
            && reqwest::Url::parse(url).is_ok_and(|u| {
                u.username().is_empty()
                    && u.password().is_none()
                    && u.origin().ascii_serialization() == self.origin
            })
    }
    pub fn close(&mut self) {
        self.open = false;
    }
}

use crate::{
    io_error,
    process::{ProcessSpec, ProcessSupervisor},
    resource_fs::ScanControl,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Browser {
    pub node: PathBuf,
    pub driver: PathBuf,
    pub playwright: PathBuf,
    pub executable: PathBuf,
    pub helper: PathBuf,
    pub state: PathBuf,
}
pub struct Observation {
    pub result: Value,
    pub png: Vec<u8>,
    pub elapsed_ms: u64,
    pub rss_bytes: u64,
}
impl Browser {
    pub fn available(&self) -> ag1_core::projects::Availability {
        for (name, path) in [
            ("Node", &self.node),
            ("browser driver", &self.driver),
            ("Playwright module", &self.playwright),
            ("automation browser", &self.executable),
            ("process helper", &self.helper),
        ] {
            if !path.is_file() {
                return ag1_core::projects::Availability::no(format!(
                    "Configure an installed {name}: {}",
                    path.display()
                ));
            }
        }
        ag1_core::projects::Availability::yes()
    }
    pub fn observe(
        &self,
        grant: &Grant,
        arguments: &Value,
        control: &ScanControl<'_>,
    ) -> Result<Observation, DomainError> {
        control.check()?;
        if !self.available().available {
            return Err(DomainError::Unavailable(self.available().reason));
        }
        let url = arguments["url"]
            .as_str()
            .ok_or(DomainError::PermissionDenied)?;
        if !grant.allows(&grant.operation, url) {
            return Err(DomainError::PermissionDenied);
        }
        for action in arguments["actions"].as_array().into_iter().flatten() {
            if action["type"] == "navigate"
                && !action["url"]
                    .as_str()
                    .is_some_and(|url| grant.allows(&grant.operation, url))
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        if arguments["viewport"].as_object().is_some_and(|v| {
            ["width", "height"]
                .iter()
                .any(|k| v[*k].as_u64().is_none_or(|n| n == 0 || n > 2048))
        }) {
            return Err(DomainError::LimitExceeded(
                "visual artifact dimensions max 2048".into(),
            ));
        }
        let mut input = arguments.clone();
        input["origin"] = json!(grant.origin);
        input["playwright"] = json!(self.playwright);
        input["executable"] = json!(self.executable);
        let mut nonce = [0; 16];
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .map_err(io_error)?
            .read_exact(&mut nonce)
            .map_err(io_error)?;
        let name = format!("browser-{:x}", Sha256::digest(nonce));
        let private = self.state.join(&name);
        std::fs::create_dir(&private).map_err(io_error)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))
            .map_err(io_error)?;
        let start = Instant::now();
        let mut process = ProcessSupervisor::new(&self.helper)?.spawn_protocol(
            ProcessSpec {
                executable: self.node.clone(),
                argv: vec![self.driver.display().to_string()],
                cwd: private.clone(),
                env: BTreeMap::from([
                    ("HOME".into(), private.display().to_string()),
                    ("PATH".into(), "/usr/bin:/bin".into()),
                ]),
                timeout_ms: 60000,
                grace_ms: 100,
                resident: false,
            },
            &private.join("stderr.log"),
        )?;
        let bytes = serde_json::to_vec(&input)
            .map_err(|_| DomainError::InvalidInput("browser request".into()))?;
        if bytes.len() > 32768 {
            return Err(DomainError::LimitExceeded("browser request".into()));
        }
        process.write_input(&bytes)?;
        process.close_input();
        let mut output = vec![];
        let mut rss = 0;
        let exit = loop {
            control.check()?;
            let bytes = process.read_stdout(32768)?;
            let empty = bytes.is_empty();
            output.extend(bytes);
            if output.len() > 12 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("browser response".into()));
            }
            rss = rss.max(super::runtime::owned_rss(process.identity().pid));
            if empty && let Some(exit) = process.try_wait()? {
                break exit;
            }
            if empty {
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        if !exit.success() {
            return Err(DomainError::Unavailable(format!(
                "Automation browser failed: {}",
                String::from_utf8_lossy(&process.tail(8192)?.bytes)
            )));
        }
        let mut result: Value = serde_json::from_slice(&output)
            .map_err(|_| DomainError::InvalidInput("browser response JSON".into()))?;
        let png = STANDARD
            .decode(result["png"].as_str().ok_or(DomainError::NotFound)?)
            .map_err(|_| DomainError::InvalidInput("browser PNG encoding".into()))?;
        result.as_object_mut().unwrap().remove("png");
        if png.len() > 8 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("browser screenshot".into()));
        }
        if result["privileged"]["node"] != false || result["privileged"]["tauri"] != false {
            return Err(DomainError::PermissionDenied);
        }
        std::fs::write(
            private.join("result.json"),
            serde_json::to_vec(&result).unwrap(),
        )
        .map_err(io_error)?;
        std::fs::write(private.join("capture.png"), &png).map_err(io_error)?;
        Ok(Observation {
            result,
            png,
            elapsed_ms: start.elapsed().as_millis() as u64,
            rss_bytes: rss,
        })
    }
}
/// Same durable binary record and branch authorization used by preserved Godot
/// consumers. No base64 is persisted in session history or public projections.
pub fn retain(
    store: &ag1_storage::Store,
    session: &str,
    origin: &str,
    png: &[u8],
    kind: &str,
    purpose: &str,
    source: &str,
) -> Result<Value, DomainError> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|_| DomainError::InvalidInput("visual PNG".into()))?;
    let sha = format!("{:x}", Sha256::digest(png));
    let id = format!("art_{}", &sha[..32]);
    let record = json!({"version":1,"id":id,"sessionId":session,"kind":kind,"producer":{"type":"tool","name":"visual-observe"},"sha256":sha,"bytes":png.len(),"encoding":"binary","mimeType":"image/png","binaryMetadata":{"width":image.width(),"height":image.height(),"purpose":purpose,"source":source,"retentionState":"retained"}});
    store
        .put_artifact(&ag1_storage::ArtifactInput {
            origin: origin.into(),
            session: session.into(),
            record: serde_json::to_vec(&record).unwrap(),
            bytes: png.to_vec(),
        })
        .map_err(|e| DomainError::Io(format!("visual artifact custody: {e}")))?;
    Ok(record)
}
