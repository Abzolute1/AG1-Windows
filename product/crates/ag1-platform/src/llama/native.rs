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
    #[cfg(test)]
    pub(crate) bytes_hashed: u64,
}
impl MaterialVerifier {
    pub fn verify(&mut self, materials: &[Material]) -> Result<(), DomainError> {
        self.verify_progress(materials, &mut |_| {})
    }
    pub fn verify_progress(&mut self, materials: &[Material], progress: &mut dyn FnMut(u8)) -> Result<(), DomainError> {
        if materials.is_empty() || materials.len() > 1024 {
            return Err(DomainError::InvalidInput(
                "runtime material manifest".into(),
            ));
        }
        let total: u128 = materials.iter().map(|m| {
            if !Path::new(&m.path).is_absolute() || m.sha256.len() != 64 || !m.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(DomainError::InvalidInput("runtime material identity".into()));
            }
            fs::metadata(&m.path).map(|m| m.len() as u128).map_err(io_error)
        }).collect::<Result<Vec<_>, _>>()?.into_iter().sum();
        let mut done = 0u128;
        let mut last = 0u8;
        progress(0);
        let mut report = |done: u128| {
            let percent = (done.saturating_mul(100) / total.max(1)).min(99) as u8;
            if percent != last { progress(percent); last = percent; }
        };
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
                done += before.len() as u128;
                report(done);
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
                done += size as u128;
                report(done);
                #[cfg(test)]
                {
                    self.bytes_hashed += size as u64;
                }
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
        progress(100);
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
    exclusive_gpu: bool,
    owned_pid: Option<u32>,
}
impl<O> NativeRuntimeHost<O> {
    pub fn require_exclusive_gpu(mut self) -> Self {
        self.exclusive_gpu = true;
        self
    }
    pub fn new(helper: &Path, private_dir: &Path, observer: O) -> Result<Self, DomainError> {
        Self::with_verifier(helper, private_dir, observer, MaterialVerifier::default())
    }
    pub(crate) fn with_verifier(
        helper: &Path,
        private_dir: &Path,
        observer: O,
        verifier: MaterialVerifier,
    ) -> Result<Self, DomainError> {
        let directory = private_dir.canonicalize().map_err(io_error)?;
        let meta = fs::metadata(&directory).map_err(io_error)?;
        if !meta.is_dir() || meta.mode() & 0o077 != 0 {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            supervisor: ProcessSupervisor::new(helper)?,
            observer,
            private_dir: directory,
            verifier,
            serial: 0,
            exclusive_gpu: false,
            owned_pid: None,
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
    fn verify_progress(&mut self, materials: &[Material], progress: &mut dyn FnMut(u8)) -> Result<(), DomainError> {
        self.verifier.verify_progress(materials, progress)
    }
    fn loading_phase(&mut self, handle: &Self::Handle) -> Option<&'static str> {
        let process = handle.process.lock().ok()?;
        let log = process.tail(8192).ok()?;
        let text = String::from_utf8_lossy(&log.bytes);
        if text.contains("warming up") { Some("warming") }
        else if text.contains("load_model: initializing") || text.contains("loaded multimodal model") { Some("initializing") }
        else { None }
    }
    fn observe(&mut self, plan: &LaunchPlan) -> Result<Observation, DomainError> {
        if self.exclusive_gpu && self.owned_pid.is_none() {
            super::window::verify(None)?;
        }
        (self.observer)(plan)
    }
    fn spawn(&mut self, plan: &LaunchPlan) -> Result<Self::Handle, DomainError> {
        if self.exclusive_gpu {
            super::window::verify(None)?;
        }
        // Reverify after health observation, immediately before creating the child.
        self.verifier.verify(&plan.runtime_materials()?)?;
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
            self.verifier.verify(&plan.runtime_materials()?)?;
            Ok(candidates.remove(0))
        })();
        match result {
            Ok(target) => {
                self.owned_pid = Some(target.pid);
                Ok(NativeOwnedRuntime {
                    process: Mutex::new(process),
                    target,
                })
            }
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
            self.owned_pid = None;
            return Ok(false);
        }
        match observe(handle.target.pid) {
            Ok(current) if current == handle.target => {
                if self.exclusive_gpu {
                    super::window::verify(Some(handle.target.pid))?;
                }
                Ok(true)
            }
            Err(DomainError::NotFound) => {
                self.owned_pid = None;
                Ok(false)
            }
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
            .stop_with_timeout(std::time::Duration::from_secs(30))?;
        self.owned_pid = None;
        Ok(())
    }
    fn resources(
        &mut self,
        handle: &Self::Handle,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        if observe(handle.target.pid)? != handle.target {
            return Err(DomainError::Conflict);
        }
        let status =
            fs::read_to_string(format!("/proc/{}/status", handle.target.pid)).map_err(io_error)?;
        let rss = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(DomainError::Conflict)?;
        let gpu = super::window::gpu_memory()?;
        if observe(handle.target.pid)? != handle.target {
            return Err(DomainError::Conflict);
        }
        Ok(Some(
            serde_json::json!({"owned_pid":handle.target.pid,"resident_kib":rss,"gpu":gpu}),
        ))
    }
}

#[cfg(test)]
mod verifier_tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn transferred_cache_avoids_rehash_but_invalidates_changed_identity_and_digest() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("model");
        fs::write(&path, b"pinned").unwrap();
        let original_times = fs::metadata(&path).unwrap();
        let material = Material {
            path: path.display().to_string(),
            sha256: format!("{:x}", Sha256::digest(b"pinned")),
            kind: "weight".into(),
        };
        let mut verifier = MaterialVerifier::default();
        verifier.verify(std::slice::from_ref(&material)).unwrap();
        assert_eq!(verifier.bytes_hashed, 6);
        let mut host = NativeRuntimeHost::with_verifier(
            &std::env::current_exe().unwrap(),
            dir.path(),
            |_: &LaunchPlan| Ok(Observation::Vacant),
            verifier,
        )
        .unwrap();
        host.verify_materials(std::slice::from_ref(&material))
            .unwrap();
        assert_eq!(host.verifier.bytes_hashed, 6);

        let mut wrong_digest = material.clone();
        wrong_digest.sha256 = "0".repeat(64);
        assert_eq!(
            host.verify_materials(&[wrong_digest]),
            Err(DomainError::Conflict)
        );
        assert_eq!(host.verifier.bytes_hashed, 12);

        // Same bytes with a different inode must be hashed again.
        let replacement = dir.path().join("replacement");
        fs::write(&replacement, b"pinned").unwrap();
        fs::rename(&replacement, &path).unwrap();
        host.verify_materials(std::slice::from_ref(&material))
            .unwrap();
        assert_eq!(host.verifier.bytes_hashed, 18);

        // Restoring mtime after a same-size write cannot restore ctime/inode.
        fs::write(&path, b"tamper").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_times.modified().unwrap()))
            .unwrap();
        assert_eq!(
            host.verify_materials(std::slice::from_ref(&material)),
            Err(DomainError::Conflict)
        );
        assert_eq!(host.verifier.bytes_hashed, 24);

        fs::remove_file(&path).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, b"pinned").unwrap();
        symlink(&target, &path).unwrap();
        host.verify_materials(std::slice::from_ref(&material))
            .unwrap();
        assert_eq!(host.verifier.bytes_hashed, 30);
        let bad_target = dir.path().join("bad-target");
        fs::write(&bad_target, b"tamper").unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&bad_target, &path).unwrap();
        assert_eq!(
            host.verify_materials(std::slice::from_ref(&material)),
            Err(DomainError::Conflict)
        );
        assert_eq!(host.verifier.bytes_hashed, 36);

        fs::remove_file(&path).unwrap();
        assert_eq!(
            host.verify_materials(std::slice::from_ref(&material)),
            Err(DomainError::NotFound)
        );
        fs::write(&path, b"pinned").unwrap();
        let mut fresh = NativeRuntimeHost::new(
            &std::env::current_exe().unwrap(),
            dir.path(),
            |_: &LaunchPlan| Ok(Observation::Vacant),
        )
        .unwrap();
        assert_eq!(fresh.verifier.bytes_hashed, 0);
        fresh.verify_materials(&[material]).unwrap();
        assert_eq!(fresh.verifier.bytes_hashed, 6);
    }
}
