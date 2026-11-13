//! Windows adapter for an owned llama.cpp process.
//!
//! This does not recreate the Linux process helper. The Windows supervisor owns
//! the server process itself through a Job Object, so its held process handle is
//! the custody proof and no later PID lookup is needed.

use super::{
    LaunchPlan, Material,
    lifecycle::{Observation, RuntimeHost},
};
use crate::{
    io_error,
    process::{OwnedProcess, ProcessIdentity, ProcessSpec, ProcessSupervisor},
};
use ag1_core::DomainError;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    os::windows::{fs::MetadataExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Signature {
    volume: u64,
    file: u64,
    size: u64,
    changed: u64,
    created: u64,
}

fn signature(file: &File) -> Result<Signature, DomainError> {
    let metadata = file.metadata().map_err(io_error)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(Signature {
        volume: info.dwVolumeSerialNumber as u64,
        file: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        size: metadata.file_size(),
        changed: metadata.last_write_time(),
        created: metadata.creation_time(),
    })
}

#[derive(Default)]
pub struct MaterialVerifier {
    cache: BTreeMap<PathBuf, (Signature, String)>,
    #[cfg(test)]
    pub(crate) bytes_hashed: u64,
}

impl MaterialVerifier {
    pub fn verify(&mut self, materials: &[Material]) -> Result<(), DomainError> {
        self.verify_progress(materials, &mut |_| {})
    }

    pub fn verify_progress(
        &mut self,
        materials: &[Material],
        progress: &mut dyn FnMut(u8),
    ) -> Result<(), DomainError> {
        if materials.is_empty() || materials.len() > 1024 {
            return Err(DomainError::InvalidInput(
                "runtime material manifest".into(),
            ));
        }

        let mut prepared = Vec::with_capacity(materials.len());
        let mut total = 0u128;
        for material in materials {
            if !Path::new(&material.path).is_absolute()
                || material.sha256.len() != 64
                || !material.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(DomainError::InvalidInput(
                    "runtime material identity".into(),
                ));
            }
            let path = Path::new(&material.path).canonicalize().map_err(io_error)?;
            let file = File::open(&path).map_err(io_error)?;
            let stamp = signature(&file)?;
            if !file.metadata().map_err(io_error)?.is_file() {
                return Err(DomainError::InvalidInput("runtime material file".into()));
            }
            total = total
                .checked_add(stamp.size as u128)
                .ok_or_else(|| DomainError::LimitExceeded("runtime material size".into()))?;
            prepared.push((material, path, file, stamp));
        }

        let mut unique = BTreeMap::new();
        let mut done = 0u128;
        let mut last = 0u8;
        progress(0);
        for (material, path, mut file, stamp) in prepared {
            if unique.insert(path.clone(), &material.sha256).is_some() {
                return Err(DomainError::InvalidInput(
                    "duplicate runtime material".into(),
                ));
            }
            if self
                .cache
                .get(&path)
                .is_some_and(|(saved, digest)| saved == &stamp && digest == &material.sha256)
            {
                done += stamp.size as u128;
                report_progress(done, total, &mut last, progress);
                continue;
            }

            // The handle pins the exact file object while hashing. Rechecking
            // both that handle and its path catches replacement or rename races.
            if signature(&file)? != stamp {
                return Err(DomainError::Conflict);
            }
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).map_err(io_error)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
                done += count as u128;
                report_progress(done, total, &mut last, progress);
                #[cfg(test)]
                {
                    self.bytes_hashed += count as u64;
                }
            }
            let current = File::open(&path).map_err(io_error)?;
            if signature(&file)? != stamp
                || signature(&current)? != stamp
                || Path::new(&material.path).canonicalize().map_err(io_error)? != path
            {
                return Err(DomainError::Conflict);
            }
            let digest = format!("{:x}", hash.finalize());
            if digest != material.sha256 {
                return Err(DomainError::Conflict);
            }
            if self.cache.len() >= 1024 && !self.cache.contains_key(&path) {
                self.cache.clear();
            }
            self.cache.insert(path, (stamp, digest));
        }
        progress(100);
        Ok(())
    }
}

fn report_progress(done: u128, total: u128, last: &mut u8, progress: &mut dyn FnMut(u8)) {
    let percent = (done.saturating_mul(100) / total.max(1)).min(99) as u8;
    if percent != *last {
        progress(percent);
        *last = percent;
    }
}

pub struct NativeOwnedRuntime {
    process: Mutex<OwnedProcess>,
    target: ProcessIdentity,
}

impl NativeOwnedRuntime {
    pub fn target_identity(&self) -> &ProcessIdentity {
        &self.target
    }
}

pub struct NativeRuntimeHost<O> {
    supervisor: ProcessSupervisor,
    observer: O,
    private_dir: PathBuf,
    verifier: MaterialVerifier,
    serial: u64,
    exclusive_gpu: bool,
}

impl<O> NativeRuntimeHost<O> {
    pub fn require_exclusive_gpu(mut self) -> Self {
        self.exclusive_gpu = true;
        self
    }

    /// `legacy_process_host` stays in this constructor while callers are moved.
    /// On Windows no helper process is started; the direct child joins its Job.
    pub fn new(
        legacy_process_host: &Path,
        private_dir: &Path,
        observer: O,
    ) -> Result<Self, DomainError> {
        Self::with_verifier(
            legacy_process_host,
            private_dir,
            observer,
            MaterialVerifier::default(),
        )
    }

    pub(crate) fn with_verifier(
        legacy_process_host: &Path,
        private_dir: &Path,
        observer: O,
        verifier: MaterialVerifier,
    ) -> Result<Self, DomainError> {
        let directory = private_dir.canonicalize().map_err(io_error)?;
        let metadata = fs::symlink_metadata(&directory).map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            supervisor: ProcessSupervisor::new(legacy_process_host)?,
            observer,
            private_dir: directory,
            verifier,
            serial: 0,
            exclusive_gpu: false,
        })
    }
}

impl<O: FnMut(&LaunchPlan) -> Result<Observation, DomainError>> RuntimeHost
    for NativeRuntimeHost<O>
{
    type Handle = NativeOwnedRuntime;

    fn verify_materials(&mut self, materials: &[Material]) -> Result<(), DomainError> {
        self.verifier.verify(materials)
    }

    fn verify_progress(
        &mut self,
        materials: &[Material],
        progress: &mut dyn FnMut(u8),
    ) -> Result<(), DomainError> {
        self.verifier.verify_progress(materials, progress)
    }

    fn loading_phase(&mut self, handle: &Self::Handle) -> Option<&'static str> {
        let process = handle.process.lock().ok()?;
        let log = process.tail(8192).ok()?;
        let text = String::from_utf8_lossy(&log.bytes);
        if text.contains("warming up") {
            Some("warming")
        } else if text.contains("load_model: initializing")
            || text.contains("loaded multimodal model")
        {
            Some("initializing")
        } else {
            None
        }
    }

    fn observe(&mut self, plan: &LaunchPlan) -> Result<Observation, DomainError> {
        if self.exclusive_gpu {
            return Err(DomainError::Unavailable(
                "exclusive GPU verification is not implemented for Windows yet".into(),
            ));
        }
        (self.observer)(plan)
    }

    fn spawn(&mut self, plan: &LaunchPlan) -> Result<Self::Handle, DomainError> {
        if self.exclusive_gpu {
            return Err(DomainError::Unavailable(
                "exclusive GPU verification is not implemented for Windows yet".into(),
            ));
        }
        self.verifier.verify(&plan.runtime_materials()?)?;
        self.serial = self.serial.checked_add(1).ok_or(DomainError::QueueFull)?;
        let spool = self.private_dir.join(format!(
            "runtime-{}-{}.log",
            std::process::id(),
            self.serial
        ));
        let mut environment = plan.environment.clone();
        // The captured profile intentionally does not inherit the user's full
        // environment. Windows still needs these system values for its loader,
        // CUDA driver, and temporary-file APIs; none exposes a user secret.
        for name in ["SystemRoot", "SystemDrive", "TEMP", "TMP"] {
            if !environment.contains_key(name)
                && let Some(value) = std::env::var_os(name)
            {
                environment.insert(name.to_owned(), value.to_string_lossy().into_owned());
            }
        }
        let spec = ProcessSpec {
            executable: PathBuf::from(&plan.command),
            argv: plan.args.clone(),
            cwd: self.private_dir.clone(),
            env: environment,
            timeout_ms: 86_400_000,
            grace_ms: 1000,
            resident: true,
        };
        let mut process = self.supervisor.spawn(spec, &spool)?;
        // Hash again after the child is attached to its Job. If a model material
        // changed between verification and launch, stop that exact owned job.
        if let Err(error) = self.verifier.verify(&plan.runtime_materials()?) {
            let _ = process.stop();
            return Err(error);
        }
        let target = process.identity().clone();
        if process.try_wait()?.is_some() {
            return Err(DomainError::Unavailable(
                "model process exited during launch".into(),
            ));
        }
        Ok(NativeOwnedRuntime {
            process: Mutex::new(process),
            target,
        })
    }

    fn owned_alive(&mut self, handle: &Self::Handle) -> Result<bool, DomainError> {
        // A live Windows process handle names one kernel process object. Unlike a
        // PID, it cannot become a handle to a later process after an exit.
        let mut process = handle
            .process
            .lock()
            .map_err(|_| DomainError::Unavailable("runtime handle poisoned".into()))?;
        let status = process.try_wait()?;
        if let Some(status) = status {
            // llama.cpp writes the useful explanation to its captured output.
            // Surface its tail instead of making a failed load look mysterious.
            let tail = String::from_utf8_lossy(&process.tail(8 * 1024)?.bytes)
                .trim()
                .to_owned();
            let detail = if tail.is_empty() {
                "no diagnostic was written".to_owned()
            } else {
                tail
            };
            return Err(DomainError::Unavailable(format!(
                "llama-server exited with {status:?}: {detail}"
            )));
        }
        Ok(true)
    }

    fn generation_idle(
        &mut self,
        _handle: &Self::Handle,
        plan: &LaunchPlan,
    ) -> Result<bool, DomainError> {
        super::observation::blocking_generation_idle(plan)
    }

    fn stop(&mut self, handle: &mut Self::Handle) -> Result<(), DomainError> {
        handle
            .process
            .lock()
            .map_err(|_| DomainError::Unavailable("runtime handle poisoned".into()))?
            .stop_with_timeout(Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama::lifecycle::RuntimeController;

    #[test]
    fn verifier_rejects_same_size_tamper_and_replacement() {
        let root = tempfile::Builder::new()
            .prefix("ag1-material-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let path = root.path().join("model.gguf");
        fs::write(&path, b"pinned").unwrap();
        let material = Material {
            path: path.display().to_string(),
            sha256: format!("{:x}", Sha256::digest(b"pinned")),
            kind: "weight".into(),
        };
        let mut verifier = MaterialVerifier::default();
        verifier.verify(std::slice::from_ref(&material)).unwrap();
        assert_eq!(verifier.bytes_hashed, 6);
        verifier.verify(std::slice::from_ref(&material)).unwrap();
        assert_eq!(verifier.bytes_hashed, 6);

        fs::write(&path, b"tamper").unwrap();
        assert_eq!(
            verifier.verify(std::slice::from_ref(&material)),
            Err(DomainError::Conflict)
        );
        let replacement = root.path().join("replacement.gguf");
        fs::write(&replacement, b"pinned").unwrap();
        fs::rename(&replacement, &path).unwrap();
        verifier.verify(std::slice::from_ref(&material)).unwrap();
        assert_eq!(verifier.bytes_hashed, 18);
    }

    #[test]
    fn runtime_lifecycle_owns_and_stops_the_direct_child() {
        let root = tempfile::Builder::new()
            .prefix("ag1-runtime-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let command =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32\\cmd.exe");
        let model = root.path().join("fixture.gguf");
        fs::write(&model, b"fixture model").unwrap();
        let mut plan = LaunchPlan {
            command: command.display().to_string(),
            // This is a short-lived test process, not llama.cpp or a model.
            args: vec![
                "/d".into(),
                "/c".into(),
                "ping -n 60 127.0.0.1 > nul".into(),
            ],
            environment: BTreeMap::new(),
            model_id: "fixture".into(),
            model_path: model.display().to_string(),
            projector: None,
            requested_context: 1024,
            max_tokens: 1,
            profile_sha256: "0".repeat(64),
            plan_sha256: String::new(),
            sources: BTreeMap::new(),
            materials: vec![
                Material {
                    path: command.display().to_string(),
                    sha256: format!("{:x}", Sha256::digest(fs::read(&command).unwrap())),
                    kind: "executable".into(),
                },
                Material {
                    path: model.display().to_string(),
                    sha256: format!("{:x}", Sha256::digest(b"fixture model")),
                    kind: "weight".into(),
                },
            ],
        };
        plan.plan_sha256 = plan.fingerprint().unwrap();
        let host = NativeRuntimeHost::new(
            Path::new("legacy-host-unused"),
            root.path(),
            |_: &LaunchPlan| -> Result<Observation, DomainError> { Ok(Observation::Vacant) },
        )
        .unwrap();
        let mut controller = RuntimeController::new(host);
        controller.start_owned_runtime(&plan).unwrap();
        assert!(controller.owned_alive().unwrap());
        controller.stop_owned_runtime().unwrap();
        assert!(!controller.owns_process());
    }
}
