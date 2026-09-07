//! Linux adapter. Called on a bounded host blocking executor; the observer supplies
//! fresh HTTP health/model information. The supervisor control socket is ownership.
use super::{
    LaunchPlan, Material,
    lifecycle::{Observation, RuntimeHost},
};
use crate::{
    io_error,
    process::{OwnedProcess, ProcessIdentity, ProcessSpec, ProcessSupervisor, observe},
};
use ag1_core::DomainError;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
type Signature = (u64, u64, u64, i64, i64, i64, i64);
fn signature(m: &fs::Metadata) -> Signature {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
#[derive(Default)]
pub struct MaterialVerifier {
    cache: BTreeMap<PathBuf, (Signature, String)>,
}
impl MaterialVerifier {
    pub fn verify(&mut self, materials: &[Material]) -> Result<(), DomainError> {
        if materials.is_empty() || materials.len() > 1024 {
            return Err(DomainError::InvalidInput(
                "runtime material manifest".into(),
            ));
        }
        let mut unique = BTreeMap::new();
        for material in materials {
            if !Path::new(&material.path).is_absolute()
                || material.sha256.len() != 64
                || !material.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(DomainError::InvalidInput(
                    "runtime material identity".into(),
                ));
            }
            if unique.insert(&material.path, &material.sha256).is_some() {
                return Err(DomainError::InvalidInput(
                    "duplicate runtime material".into(),
                ));
            }
            let path = Path::new(&material.path).canonicalize().map_err(io_error)?;
            let before = fs::metadata(&path).map_err(io_error)?;
            if !before.is_file() {
                return Err(DomainError::InvalidInput("runtime material file".into()));
            }
            let stamp = signature(&before);
            if self
                .cache
                .get(&path)
                .is_some_and(|(saved, digest)| saved == &stamp && digest == &material.sha256)
            {
                continue;
            }
            let mut file = File::open(&path).map_err(io_error)?;
            if signature(&file.metadata().map_err(io_error)?) != stamp {
                return Err(DomainError::Conflict);
            }
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let size = file.read(&mut buffer).map_err(io_error)?;
                if size == 0 {
                    break;
                }
                hash.update(&buffer[..size]);
            }
            if signature(&file.metadata().map_err(io_error)?) != stamp
                || signature(&fs::metadata(&path).map_err(io_error)?) != stamp
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
        Ok(())
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
}
impl<O> NativeRuntimeHost<O> {
    pub fn new(helper: &Path, private_dir: &Path, observer: O) -> Result<Self, DomainError> {
        let directory = private_dir.canonicalize().map_err(io_error)?;
        let meta = fs::metadata(&directory).map_err(io_error)?;
        if !meta.is_dir() || meta.mode() & 0o077 != 0 {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            supervisor: ProcessSupervisor::new(helper)?,
            observer,
            private_dir: directory,
            verifier: MaterialVerifier::default(),
            serial: 0,
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
    fn observe(&mut self, plan: &LaunchPlan) -> Result<Observation, DomainError> {
        (self.observer)(plan)
    }
    fn spawn(&mut self, plan: &LaunchPlan) -> Result<Self::Handle, DomainError> {
        // Reverify after health observation, immediately before creating the child.
        self.verifier.verify(&plan.materials)?;
        self.serial = self.serial.checked_add(1).ok_or(DomainError::QueueFull)?;
        let spool = self.private_dir.join(format!(
            "runtime-{}-{}.log",
            std::process::id(),
            self.serial
        ));
        let spec = ProcessSpec {
            executable: PathBuf::from(&plan.command),
            argv: plan.args.clone(),
            cwd: self.private_dir.clone(),
            env: plan.environment.clone(),
            timeout_ms: 86_400_000,
            grace_ms: 1000,
            resident: true,
        };
        let mut process = self.supervisor.spawn(spec, &spool)?;
        let result = (|| {
            let helper = process.identity();
            if observe(helper.pid)? != *helper {
                return Err(DomainError::Conflict);
            }
            let children =
                fs::read_to_string(format!("/proc/{}/task/{}/children", helper.pid, helper.pid))
                    .map_err(io_error)?;
            let expected = fs::metadata(&plan.command).map_err(io_error)?;
            let mut candidates = vec![];
            for pid in children.split_whitespace().take(32) {
                let pid = pid
                    .parse()
                    .map_err(|_| DomainError::InvalidInput("supervised child identity".into()))?;
                if let Ok(identity) = observe(pid)
                    && identity.device == expected.dev()
                    && identity.inode == expected.ino()
                {
                    candidates.push(identity);
                }
            }
            if candidates.len() != 1 {
                return Err(DomainError::Unavailable(
                    "model process exited or executable identity differs".into(),
                ));
            }
            self.verifier.verify(&plan.materials)?;
            Ok(candidates.remove(0))
        })();
        match result {
            Ok(target) => Ok(NativeOwnedRuntime {
                process: Mutex::new(process),
                target,
            }),
            Err(error) => {
                process.stop()?;
                Err(error)
            }
        }
    }
    fn owned_alive(&mut self, handle: &Self::Handle) -> Result<bool, DomainError> {
        let mut process = handle
            .process
            .lock()
            .map_err(|_| DomainError::Unavailable("runtime handle poisoned".into()))?;
        if process.try_wait()?.is_some() {
            return Ok(false);
        }
        match observe(handle.target.pid) {
            Ok(current) if current == handle.target => Ok(true),
            Err(DomainError::NotFound) => Ok(false),
            _ => Err(DomainError::Conflict),
        }
    }
    fn stop(&mut self, handle: &mut Self::Handle) -> Result<(), DomainError> {
        // The helper owns/reaps the target tree even if its original PID exited.
        // No discovered target PID is ever used as a signal capability.
        handle
            .process
            .lock()
            .map_err(|_| DomainError::Unavailable("runtime handle poisoned".into()))?
            .stop()
    }
}
