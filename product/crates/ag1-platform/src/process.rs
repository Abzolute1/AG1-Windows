//! Linux process capabilities. An observation is never a kill handle.
use crate::io_error;
use ag1_core::{DomainError, PtyPort, PtySize};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::MetadataExt, net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const MAX_LOG_MEMORY: usize = 64 * 1024;
pub const MAX_LOG_DISK: usize = 8 * 1024 * 1024;
pub const STOP_DEADLINE_MS: u64 = 3000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_ticks: u64,
    pub executable: PathBuf,
    pub device: u64,
    pub inode: u64,
}
pub fn observe(pid: u32) -> Result<ProcessIdentity, DomainError> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(io_error)?;
    let end = stat
        .rfind(')')
        .ok_or_else(|| DomainError::InvalidInput("process stat".into()))?;
    let start_ticks = stat[end + 2..]
        .split_whitespace()
        .nth(19)
        .ok_or(DomainError::NotFound)?
        .parse()
        .map_err(|_| DomainError::NotFound)?;
    let executable = std::fs::read_link(format!("/proc/{pid}/exe")).map_err(io_error)?;
    let metadata = std::fs::metadata(format!("/proc/{pid}/exe")).map_err(io_error)?;
    Ok(ProcessIdentity {
        pid,
        start_ticks,
        executable,
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}
fn pidfd(pid: u32) -> Result<OwnedFd, DomainError> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
fn signal(fd: &OwnedFd, sig: i32) -> Result<(), DomainError> {
    if unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            sig,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    } < 0
    {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(io_error(err));
        }
    }
    Ok(())
}

/// Host-only spec; environment is explicit and is never included in diagnostics.
#[derive(Clone, Serialize, Deserialize)]
pub struct ProcessSpec {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub grace_ms: u64,
}
impl ProcessSpec {
    fn validate(&self) -> Result<(), DomainError> {
        if !self.executable.is_absolute()
            || !self.cwd.is_absolute()
            || self.argv.len() > 256
            || self.env.len() > 128
            || self.timeout_ms == 0
            || self.timeout_ms > 86_400_000
            || self.grace_ms > 1000
        {
            return Err(DomainError::InvalidInput(
                "explicit process paths and bounded lifetime required".into(),
            ));
        }
        if serde_json::to_vec(self)
            .map_err(|_| DomainError::InvalidInput("process spec".into()))?
            .len()
            > 64 * 1024
        {
            return Err(DomainError::LimitExceeded("process specification".into()));
        }
        if !self.cwd.is_dir() || !self.executable.is_file() {
            return Err(DomainError::NotFound);
        }
        Ok(())
    }
}

/// No Stop/Quit method exists on observed services. A held reservation is stronger than a port scan.
pub struct PortReservation {
    listener: TcpListener,
}
impl PortReservation {
    pub fn bind(address: SocketAddr) -> Result<Self, DomainError> {
        TcpListener::bind(address)
            .map(|listener| Self { listener })
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AddrInUse {
                    DomainError::Conflict
                } else {
                    io_error(error)
                }
            })
    }
    pub fn address(&self) -> Result<SocketAddr, DomainError> {
        self.listener.local_addr().map_err(io_error)
    }
    pub fn into_listener(self) -> TcpListener {
        self.listener
    }
}

struct Spool {
    file: File,
    tail: std::collections::VecDeque<u8>,
    written: usize,
    seen: u64,
    truncated: bool,
    error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct LogView {
    pub bytes: Vec<u8>,
    pub total_seen: u64,
    pub disk_bytes: usize,
    pub memory_bytes: usize,
    pub truncated: bool,
}
impl Spool {
    fn append(&mut self, bytes: &[u8]) {
        self.seen = self.seen.saturating_add(bytes.len() as u64);
        let keep = bytes.len().min(MAX_LOG_DISK.saturating_sub(self.written));
        if self.error.is_none() && keep > 0 {
            if let Err(error) = self.file.write_all(&bytes[..keep]) {
                self.error = Some(error.to_string());
            } else {
                self.written += keep;
            }
        }
        self.truncated |= keep != bytes.len();
        for byte in bytes {
            if self.tail.len() == MAX_LOG_MEMORY {
                self.tail.pop_front();
            }
            self.tail.push_back(*byte);
        }
    }
    fn view(&self, limit: usize) -> Result<LogView, DomainError> {
        if limit > MAX_LOG_MEMORY {
            return Err(DomainError::LimitExceeded("log tail".into()));
        }
        if let Some(error) = &self.error {
            return Err(DomainError::Io(error.clone()));
        }
        Ok(LogView {
            bytes: self
                .tail
                .iter()
                .skip(self.tail.len().saturating_sub(limit))
                .copied()
                .collect(),
            total_seen: self.seen,
            disk_bytes: self.written,
            memory_bytes: self.tail.len(),
            truncated: self.truncated,
        })
    }
}
fn nonblocking(fd: i32) -> Result<(), DomainError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn reader(
    mut file: File,
    spool: Arc<Mutex<Spool>>,
    done: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, DomainError> {
    nonblocking(file.as_raw_fd())?;
    thread::Builder::new()
        .name("ag1-log-drain".into())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => spool.lock().unwrap().append(&buffer[..count]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if done.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break, // PTY slave exit
                    Err(error) => {
                        spool.lock().unwrap().error = Some(error.to_string());
                        break;
                    }
                }
            }
        })
        .map_err(io_error)
}

/// Owns a compiled AG1 helper, never a discovered PID. At most 32 concurrent children per supervisor.
pub struct ProcessSupervisor {
    helper: PathBuf,
    helper_identity: (u64, u64),
    active: Arc<std::sync::atomic::AtomicUsize>,
}
struct ActiveLease(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for ActiveLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl ProcessSupervisor {
    pub fn new(helper: &Path) -> Result<Self, DomainError> {
        let helper = helper.canonicalize().map_err(io_error)?;
        let metadata = std::fs::metadata(&helper).map_err(io_error)?;
        Ok(Self {
            helper,
            helper_identity: (metadata.dev(), metadata.ino()),
            active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }
    pub fn spawn(&self, spec: ProcessSpec, spool_path: &Path) -> Result<OwnedProcess, DomainError> {
        self.spawn_inner(spec, spool_path, None)
    }
    /// P04 worker transport: stdout stays a separate bounded kernel pipe; stderr is spooled.
    pub fn spawn_protocol(
        &self,
        spec: ProcessSpec,
        spool_path: &Path,
    ) -> Result<OwnedProcess, DomainError> {
        self.spawn_configured(spec, spool_path, None, false)
    }
    pub fn spawn_pty(
        &self,
        spec: ProcessSpec,
        spool_path: &Path,
        size: PtySize,
    ) -> Result<OwnedPty, DomainError> {
        validate_size(&size)?;
        let mut master = -1;
        let mut slave = -1;
        let size = libc::winsize {
            ws_row: size.rows,
            ws_col: size.columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        } != 0
        {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
        }
        let process = self.spawn_inner(spec, spool_path, Some(slave))?;
        nonblocking(master.as_raw_fd())?;
        Ok(OwnedPty {
            process,
            master,
            disconnected: false,
        })
    }
    fn spawn_inner(
        &self,
        spec: ProcessSpec,
        spool_path: &Path,
        pty: Option<File>,
    ) -> Result<OwnedProcess, DomainError> {
        self.spawn_configured(spec, spool_path, pty, true)
    }
    fn spawn_configured(
        &self,
        spec: ProcessSpec,
        spool_path: &Path,
        pty: Option<File>,
        capture_stdout: bool,
    ) -> Result<OwnedProcess, DomainError> {
        spec.validate()?;
        if self
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 32).then_some(n + 1)
            })
            .is_err()
        {
            return Err(DomainError::LimitExceeded("owned processes".into()));
        }
        self.launch(
            spec,
            spool_path,
            pty,
            ActiveLease(self.active.clone()),
            capture_stdout,
        )
    }
    fn launch(
        &self,
        spec: ProcessSpec,
        spool_path: &Path,
        pty: Option<File>,
        lease: ActiveLease,
        capture_stdout: bool,
    ) -> Result<OwnedProcess, DomainError> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .open(spool_path)
            .map_err(io_error)?;
        let spool = Arc::new(Mutex::new(Spool {
            file,
            tail: Default::default(),
            written: 0,
            seen: 0,
            truncated: false,
            error: None,
        }));
        let (mut control, child_control) = UnixStream::pair().map_err(io_error)?;
        control
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(io_error)?;
        control
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(io_error)?;
        let fd = child_control.as_raw_fd();
        let cwd_handle = File::open(&spec.cwd).map_err(io_error)?;
        let cwd_fd = cwd_handle.as_raw_fd();
        let mut command = Command::new(&self.helper);
        command
            .arg(fd.to_string())
            .current_dir(&spec.cwd)
            .env_clear();
        if let Some(slave) = &pty {
            command
                .stdin(slave.try_clone().map_err(io_error)?)
                .stdout(slave.try_clone().map_err(io_error)?)
                .stderr(slave.try_clone().map_err(io_error)?);
        } else {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0
                    || libc::fcntl(fd, libc::F_SETFD, 0) < 0
                    || libc::fchdir(cwd_fd) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(io_error)?;
        drop(child_control);
        drop(pty);
        // Retain a direct child and private control capability before exposing ownership to callers.
        let setup = (|| {
            let identity = observe(child.id())?;
            if (identity.device, identity.inode) != self.helper_identity {
                return Err(DomainError::Conflict);
            }
            let handle = pidfd(child.id())?;
            let bytes = serde_json::to_vec(&spec)
                .map_err(|_| DomainError::InvalidInput("process spec".into()))?;
            control
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .and_then(|_| control.write_all(&bytes))
                .map_err(io_error)?;
            let mut ready = [0u8; 1];
            control.read_exact(&mut ready).map_err(io_error)?;
            if ready != [1] {
                return Err(DomainError::Io("owned child failed to launch".into()));
            }
            Ok((identity, handle))
        })();
        let (identity, handle) = match setup {
            Ok(pair) => pair,
            Err(error) => {
                drop(control);
                let _ = child.wait();
                return Err(error);
            }
        };
        let done = Arc::new(AtomicBool::new(false));
        let mut process = OwnedProcess {
            identity,
            _pidfd: handle,
            stdin: child.stdin.take(),
            child,
            control: Some(control),
            readers: vec![],
            spool,
            done,
            status: None,
            lease: Some(lease),
            stdout: None,
        };
        if !capture_stdout {
            process.stdout = process
                .child
                .stdout
                .take()
                .map(OwnedFd::from)
                .map(File::from);
            if let Some(stdout) = &process.stdout {
                nonblocking(stdout.as_raw_fd())?;
            }
        }
        for pipe in [
            process.child.stdout.take().map(OwnedFd::from),
            process.child.stderr.take().map(OwnedFd::from),
        ]
        .into_iter()
        .flatten()
        {
            process.readers.push(reader(
                File::from(pipe),
                process.spool.clone(),
                process.done.clone(),
            )?);
        }
        Ok(process)
    }
}

pub struct OwnedProcess {
    identity: ProcessIdentity,
    _pidfd: OwnedFd,
    child: Child,
    stdin: Option<ChildStdin>,
    control: Option<UnixStream>,
    readers: Vec<JoinHandle<()>>,
    spool: Arc<Mutex<Spool>>,
    done: Arc<AtomicBool>,
    status: Option<ExitStatus>,
    lease: Option<ActiveLease>,
    stdout: Option<File>,
}
impl OwnedProcess {
    pub fn read_stdout(&mut self, max_bytes: usize) -> Result<Vec<u8>, DomainError> {
        if max_bytes == 0 || max_bytes > 32 * 1024 {
            return Err(DomainError::LimitExceeded("protocol stdout".into()));
        }
        let stdout = self.stdout.as_mut().ok_or(DomainError::NotFound)?;
        let mut bytes = vec![0; max_bytes];
        match stdout.read(&mut bytes) {
            Ok(count) => {
                bytes.truncate(count);
                Ok(bytes)
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(vec![]),
            Err(error) => Err(io_error(error)),
        }
    }
    pub fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }
    pub fn tail(&self, limit: usize) -> Result<LogView, DomainError> {
        self.spool.lock().unwrap().view(limit)
    }
    pub fn log_page(&self, offset: u64, limit: usize) -> Result<Vec<u8>, DomainError> {
        if limit > 32 * 1024 {
            return Err(DomainError::LimitExceeded("log page".into()));
        }
        let spool = self.spool.lock().unwrap();
        if offset > spool.written as u64 {
            return Err(DomainError::NotFound);
        }
        let file = spool.file.try_clone().map_err(io_error)?;
        // pread avoids sharing the write cursor with a log reader.
        use std::os::unix::fs::FileExt;
        let mut bytes = vec![0; limit.min(spool.written - offset as usize)];
        let count = file.read_at(&mut bytes, offset).map_err(io_error)?;
        bytes.truncate(count);
        Ok(bytes)
    }
    pub fn write_input(&mut self, bytes: &[u8]) -> Result<usize, DomainError> {
        if bytes.len() > 32 * 1024 {
            return Err(DomainError::LimitExceeded("process input".into()));
        }
        let stdin = self.stdin.as_mut().ok_or(DomainError::NotFound)?;
        nonblocking(stdin.as_raw_fd())?;
        stdin.write(bytes).map_err(io_error)
    }
    pub fn close_input(&mut self) {
        self.stdin.take();
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, DomainError> {
        if self.status.is_none() {
            self.status = self.child.try_wait().map_err(io_error)?;
        }
        if self.status.is_some() {
            self.finish();
        }
        Ok(self.status)
    }
    fn finish(&mut self) {
        self.done.store(true, Ordering::Release);
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        self.stdin.take();
        self.control.take();
        self.lease.take();
    }
    pub fn wait(&mut self, timeout: Duration) -> Result<ExitStatus, DomainError> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if start.elapsed() >= timeout {
                self.stop()?;
                return Err(DomainError::TimedOut);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    pub fn stop_if_identity(&mut self, expected: &ProcessIdentity) -> Result<(), DomainError> {
        if expected != &self.identity {
            return Err(DomainError::Conflict);
        }
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        match observe(self.identity.pid) {
            Ok(current) if current == self.identity => {}
            // /proc/exe can disappear just before waitpid observes exit. The private socket still
            // names our child; closing it cannot signal a replacement PID.
            Err(DomainError::NotFound) => return self.request_stop(),
            _ => return Err(DomainError::Conflict),
        }
        self.request_stop()
    }
    pub fn stop(&mut self) -> Result<(), DomainError> {
        self.stop_if_identity(&self.identity.clone())
    }
    fn request_stop(&mut self) -> Result<(), DomainError> {
        if let Some(control) = self.control.take() {
            let _ = control.shutdown(std::net::Shutdown::Both);
        }
        let start = Instant::now();
        while self.try_wait()?.is_none() {
            if start.elapsed() >= Duration::from_millis(STOP_DEADLINE_MS) {
                return Err(DomainError::TimedOut);
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.request_stop();
    }
}

fn validate_size(size: &PtySize) -> Result<(), DomainError> {
    if size.rows == 0 || size.columns == 0 || size.rows > 1000 || size.columns > 1000 {
        Err(DomainError::InvalidInput("PTY size".into()))
    } else {
        Ok(())
    }
}
pub struct OwnedPty {
    pub process: OwnedProcess,
    master: File,
    disconnected: bool,
}
impl PtyPort for OwnedPty {
    fn write_input(&mut self, bytes: &[u8]) -> Result<usize, DomainError> {
        if self.disconnected {
            return Err(DomainError::Cancelled);
        }
        if bytes.len() > 32 * 1024 {
            return Err(DomainError::LimitExceeded("PTY input".into()));
        }
        self.master.write(bytes).map_err(io_error)
    }
    fn resize(&mut self, size: PtySize) -> Result<(), DomainError> {
        if self.disconnected {
            return Err(DomainError::Cancelled);
        }
        validate_size(&size)?;
        let size = libc::winsize {
            ws_row: size.rows,
            ws_col: size.columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    fn read_output(&mut self, max_bytes: usize) -> Result<Vec<u8>, DomainError> {
        if max_bytes == 0 || max_bytes > 32 * 1024 {
            return Err(DomainError::LimitExceeded("PTY output".into()));
        }
        let mut bytes = vec![0; max_bytes];
        match self.master.read(&mut bytes) {
            Ok(count) => {
                bytes.truncate(count);
                self.process.spool.lock().unwrap().append(&bytes);
                Ok(bytes)
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(libc::EIO) =>
            {
                Ok(vec![])
            }
            Err(error) => Err(io_error(error)),
        }
    }
    fn disconnect(&mut self) -> Result<(), DomainError> {
        self.process.stop()?;
        self.disconnected = true;
        Ok(())
    }
}

// The helper is an isolated subreaper. It enumerates only its own direct children after adoption.
// pidfds pin each selected child identity; no public API accepts a PID to terminate.
fn direct_children() -> Result<Vec<u32>, DomainError> {
    let pid = std::process::id();
    let bytes =
        std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).map_err(io_error)?;
    if bytes.len() > 64 * 1024 {
        return Err(DomainError::LimitExceeded("process descendants".into()));
    }
    bytes
        .split_whitespace()
        .map(|v| v.parse().map_err(|_| DomainError::NotFound))
        .collect()
}
fn kill_direct_children(sig: i32) -> Result<(), DomainError> {
    for pid in direct_children()? {
        // These are unreaped children of this private helper; the PID cannot be recycled before waitpid.
        match pidfd(pid) {
            Ok(fd) => signal(&fd, sig)?,
            Err(DomainError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

struct HelperCleanup;
impl Drop for HelperCleanup {
    fn drop(&mut self) {
        // Any error after spawning still owns cleanup, including a disconnected parent during setup.
        let start = Instant::now();
        while direct_children().is_ok_and(|children| !children.is_empty()) {
            let _ = kill_direct_children(libc::SIGKILL);
            let mut status = 0;
            while unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) } > 0 {}
            if start.elapsed() >= Duration::from_millis(STOP_DEADLINE_MS) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn helper_main() -> Result<i32, DomainError> {
    let fd: i32 = std::env::args()
        .nth(1)
        .ok_or(DomainError::PermissionDenied)?
        .parse()
        .map_err(|_| DomainError::PermissionDenied)?;
    if fd < 3 {
        return Err(DomainError::PermissionDenied);
    }
    let mut control = unsafe { UnixStream::from_raw_fd(fd) };
    control
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(io_error)?;
    let mut length = [0; 4];
    control.read_exact(&mut length).map_err(io_error)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 64 * 1024 {
        return Err(DomainError::LimitExceeded("helper specification".into()));
    }
    let mut bytes = vec![0; length];
    control.read_exact(&mut bytes).map_err(io_error)?;
    let spec: ProcessSpec = serde_json::from_slice(&bytes)
        .map_err(|_| DomainError::InvalidInput("helper specification".into()))?;
    spec.validate()?;
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let mut command = Command::new(&spec.executable);
    command
        .args(&spec.argv)
        .current_dir(".") // Inherit the directory pinned by the parent; do not resolve it again.
        .env_clear()
        .envs(&spec.env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    unsafe {
        command.pre_exec(move || {
            libc::close(fd);
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::isatty(0) == 1 && libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = control.write_all(&[0]);
            return Err(io_error(error));
        }
    };
    let _cleanup = HelperCleanup;
    let root_pid = child.id() as i32;
    // waitpid below reaps this owned child and adopted descendants; Child::drop does not wait.
    drop(child);
    control.write_all(&[1]).map_err(io_error)?;
    control.set_nonblocking(true).map_err(io_error)?;
    let started = Instant::now();
    let mut ending: Option<Instant> = None;
    let mut exit_code = None;
    loop {
        let mut status = 0;
        loop {
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                break;
            }
            if pid == root_pid {
                exit_code = Some(if libc::WIFEXITED(status) {
                    libc::WEXITSTATUS(status)
                } else {
                    128 + libc::WTERMSIG(status)
                });
                ending.get_or_insert_with(Instant::now);
            }
        }
        let mut marker = [0; 1];
        match control.read(&mut marker) {
            Ok(_) => {
                ending.get_or_insert_with(Instant::now);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                ending.get_or_insert_with(Instant::now);
            }
        }
        if started.elapsed() >= Duration::from_millis(spec.timeout_ms) {
            ending.get_or_insert_with(Instant::now);
            exit_code.get_or_insert(124);
        }
        if let Some(since) = ending {
            let sig = if since.elapsed() >= Duration::from_millis(spec.grace_ms) {
                libc::SIGKILL
            } else {
                libc::SIGTERM
            };
            kill_direct_children(sig)?;
            if direct_children()?.is_empty() {
                return Ok(exit_code.unwrap_or(130));
            }
            if since.elapsed() > Duration::from_millis(STOP_DEADLINE_MS - 500) {
                // Do not claim cleanup on an uninterruptible kernel task. Keep ownership and continue reaping.
                kill_direct_children(libc::SIGKILL)?;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}
