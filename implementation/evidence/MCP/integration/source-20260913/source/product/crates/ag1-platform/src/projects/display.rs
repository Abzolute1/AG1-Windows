//! Optional private X11 display for untrusted native targets. Only this owned
//! display socket and its one-time cookie enter the generated application's view.
use crate::{
    io_error,
    process::{OwnedProcess, ProcessSpec, ProcessSupervisor},
    resource_fs::ScanControl,
};
use ag1_core::DomainError;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
pub struct NativeDisplay {
    pub(super) process: OwnedProcess,
    pub(super) number: u32,
    pub(super) auth: PathBuf,
}
impl NativeDisplay {
    pub(super) fn start(
        supervisor: &ProcessSupervisor,
        xvfb: &Path,
        state: &Path,
        control: &ScanControl<'_>,
    ) -> Result<Self, DomainError> {
        static ADMISSION: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _admission = ADMISSION.lock().map_err(|_| DomainError::Conflict)?;
        if !xvfb.is_file() {
            return Err(DomainError::Unavailable(
                "Install or explicitly configure Xvfb for an isolated native preview".into(),
            ));
        }
        let number = (100..500)
            .find(|n| {
                !Path::new(&format!("/tmp/.X{n}-lock")).exists()
                    && !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
            })
            .ok_or(DomainError::Conflict)?;
        let auth = state.join(format!("display-{number}.auth"));
        let mut cookie = [0; 16];
        fs::File::open("/dev/urandom")
            .map_err(io_error)?
            .read_exact(&mut cookie)
            .map_err(io_error)?;
        let mut bytes = vec![];
        bytes.extend(65535u16.to_be_bytes());
        for field in [
            b"".as_slice(),
            number.to_string().as_bytes(),
            b"MIT-MAGIC-COOKIE-1",
            &cookie,
        ] {
            bytes.extend((field.len() as u16).to_be_bytes());
            bytes.extend(field);
        }
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&auth)
            .map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        let mut process = supervisor.spawn_protocol(
            ProcessSpec {
                executable: xvfb.into(),
                argv: vec![
                    format!(":{number}"),
                    "-displayfd".into(),
                    "1".into(),
                    "-screen".into(),
                    "0".into(),
                    "1280x960x24".into(),
                    "-nolisten".into(),
                    "tcp".into(),
                    "-auth".into(),
                    auth.display().to_string(),
                ],
                cwd: state.into(),
                env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                timeout_ms: 300000,
                grace_ms: 100,
                resident: false,
            },
            &state.join(format!("display-{number}.log")),
        )?;
        let mut ready = vec![];
        loop {
            control.check()?;
            ready.extend(process.read_stdout(128)?);
            if ready.contains(&b'\n') {
                break;
            }
            if process.try_wait()?.is_some() {
                return Err(DomainError::Unavailable(format!(
                    "Private X11 display failed: {}",
                    String::from_utf8_lossy(&process.tail(8192)?.bytes)
                )));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if String::from_utf8_lossy(&ready).trim() != number.to_string() {
            return Err(DomainError::Conflict);
        }
        Ok(Self {
            process,
            number,
            auth,
        })
    }
    pub fn stop(&mut self) -> Result<(), DomainError> {
        self.process.stop()
    }
}
