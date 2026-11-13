//! Windows process ownership.
//!
//! A Job Object owns every process AG1W starts. A child is created suspended,
//! assigned to that job, then resumed. That ordering matters: starting first
//! and assigning later leaves a small but real orphan-process window.

use crate::io_error;
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{OsStr, c_void},
    fs::{File, OpenOptions},
    io::{Read, Write},
    mem::size_of,
    net::{SocketAddr, TcpListener},
    os::windows::{
        ffi::OsStrExt,
        fs::FileExt,
        io::{AsRawHandle, FromRawHandle, RawHandle},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::ExitStatus,
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, FILETIME, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            GetProcessTimes, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
            ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
            UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

pub const MAX_LOG_MEMORY: usize = 64 * 1024;
pub const MAX_LOG_DISK: usize = 8 * 1024 * 1024;
pub const STOP_DEADLINE_MS: u64 = 3000;

/// The Linux pinned-descriptor protocol has no Windows equivalent yet.
/// Keeping these names lets callers be moved one at a time, but Windows rejects
/// any path that asks for the Linux-only containment protocol.
pub const PINNED_CWD_FD: &str = "@AG1_PINNED_CWD_FD@";
pub(crate) const PINNED_DIRECTORY_PREFIX: &str = "@AG1_PINNED_DIRECTORY@";
pub(crate) const READONLY_FILTER_FD: &str = "@AG1_READONLY_FILTER_FD@";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Windows file times are 100-nanosecond ticks since 1601. They are only
    /// compared with another identity from this backend, never displayed.
    pub start_ticks: u64,
    pub executable: PathBuf,
    pub device: u64,
    pub inode: u64,
}

/// Windows observation is deliberately not a stop capability. The native
/// runtime retains its own process handle instead of later trusting a PID.
pub fn observe(_pid: u32) -> Result<ProcessIdentity, DomainError> {
    Err(DomainError::Unavailable(
        "Windows process observation is not exposed outside an owned handle".into(),
    ))
}

/// Host-only spec; environment is explicit and never copied into diagnostics.
#[derive(Clone, Serialize, Deserialize)]
pub struct ProcessSpec {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub grace_ms: u64,
    #[serde(default)]
    pub resident: bool,
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
            || !self.cwd.is_dir()
            || !self.executable.is_file()
        {
            return Err(DomainError::InvalidInput(
                "explicit process paths and bounded lifetime required".into(),
            ));
        }

        if self.argv.iter().any(|argument| argument.contains('\0'))
            || self.env.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
            })
        {
            return Err(DomainError::InvalidInput(
                "Windows process environment".into(),
            ));
        }

        if serde_json::to_vec(self)
            .map_err(|_| DomainError::InvalidInput("process spec".into()))?
            .len()
            > 64 * 1024
        {
            return Err(DomainError::LimitExceeded("process specification".into()));
        }
        Ok(())
    }
}

/// A held socket is stronger evidence of availability than a preceding probe.
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

struct Handle(HANDLE);

// Windows kernel handles may move between threads. This wrapper owns exactly
// one handle and only closes it, so sending the owner is safe.
unsafe impl Send for Handle {}

impl Handle {
    fn new(handle: HANDLE) -> Result<Self, DomainError> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io_error(std::io::Error::last_os_error()))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }

    fn into_file(self) -> File {
        let handle = self.0;
        std::mem::forget(self);
        // This transfers the one close responsibility from Handle to File.
        unsafe { File::from_raw_handle(handle as RawHandle) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct AttributeList {
    // The Win32 API treats this as an opaque C structure. `usize` storage gives
    // it the alignment C expects; a byte vector only promises byte alignment.
    words: Vec<usize>,
}

impl AttributeList {
    fn for_handles(handles: &[HANDLE]) -> Result<Self, DomainError> {
        let mut required = 0usize;
        // The first call is documented to fail while reporting the allocation size.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut required) };
        if required == 0 {
            return Err(win_error("InitializeProcThreadAttributeList"));
        }

        let word_count = required.div_ceil(size_of::<usize>());
        let mut words = vec![0usize; word_count];
        let list = words.as_mut_ptr().cast::<c_void>() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut required) } == 0 {
            return Err(win_error("UpdateProcThreadAttribute"));
        }

        if unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast::<c_void>(),
                std::mem::size_of_val(handles),
                null_mut(),
                null(),
            )
        } == 0
        {
            unsafe { DeleteProcThreadAttributeList(list) };
            return Err(win_error("SetInformationJobObject"));
        }
        Ok(Self { words })
    }

    fn raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.words.as_mut_ptr().cast::<c_void>() as LPPROC_THREAD_ATTRIBUTE_LIST
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.raw()) };
    }
}

struct Job {
    handle: Handle,
}

impl Job {
    fn new(memory_limit: Option<u64>) -> Result<Self, DomainError> {
        let handle = Handle::new(unsafe { CreateJobObjectW(null(), null()) })?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(bytes) = memory_limit {
            if !(64 * 1024 * 1024..=64 * 1024 * 1024 * 1024).contains(&bytes) {
                return Err(DomainError::InvalidInput("process memory limit".into()));
            }
            limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            limits.ProcessMemoryLimit = bytes as usize;
        }
        if unsafe {
            SetInformationJobObject(
                handle.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(win_error("AssignProcessToJobObject"));
        }
        Ok(Self { handle })
    }

    fn assign(&self, process: HANDLE) -> Result<(), DomainError> {
        if unsafe { AssignProcessToJobObject(self.handle.raw(), process) } == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn terminate(&self) -> Result<(), DomainError> {
        if unsafe { TerminateJobObject(self.handle.raw(), 1) } == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

struct Spool {
    file: File,
    tail: VecDeque<u8>,
    written: usize,
    seen: u64,
    truncated: bool,
    disk_limit: usize,
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
        let keep = bytes
            .len()
            .min(self.disk_limit.saturating_sub(self.written));
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

fn reader(mut pipe: File, spool: Arc<Mutex<Spool>>) -> Result<JoinHandle<()>, DomainError> {
    thread::Builder::new()
        .name("ag1-log-drain".into())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => spool.lock().unwrap().append(&buffer[..count]),
                    Err(error) => {
                        spool.lock().unwrap().error = Some(error.to_string());
                        break;
                    }
                }
            }
        })
        .map_err(io_error)
}

struct ActiveLease(Arc<AtomicUsize>);

impl Drop for ActiveLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Owns direct Windows children. The legacy helper argument is accepted so
/// Linux callers can be moved in small, reviewable steps; Windows never runs it.
#[derive(Clone, Default)]
pub struct ProcessSupervisor {
    active: Arc<AtomicUsize>,
}

impl ProcessSupervisor {
    pub fn new(_legacy_process_host: &Path) -> Result<Self, DomainError> {
        Ok(Self::default())
    }

    pub fn spawn(&self, spec: ProcessSpec, spool_path: &Path) -> Result<OwnedProcess, DomainError> {
        self.spawn_inner(spec, spool_path, None)
    }

    /// Llama servers use this bounded process path. Protocol transports need
    /// overlapped named pipes on Windows, so they remain explicitly unavailable.
    pub fn spawn_protocol(
        &self,
        _spec: ProcessSpec,
        _spool_path: &Path,
    ) -> Result<OwnedProcess, DomainError> {
        Err(DomainError::Unavailable(
            "Windows protocol pipes are not implemented yet".into(),
        ))
    }

    pub fn spawn_protocol_limited(
        &self,
        _spec: ProcessSpec,
        _spool_path: &Path,
        _address_space: u64,
    ) -> Result<OwnedProcess, DomainError> {
        Err(DomainError::Unavailable(
            "Windows protocol pipes are not implemented yet".into(),
        ))
    }

    fn spawn_inner(
        &self,
        spec: ProcessSpec,
        spool_path: &Path,
        memory_limit: Option<u64>,
    ) -> Result<OwnedProcess, DomainError> {
        spec.validate()?;
        if self
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 32).then_some(count + 1)
            })
            .is_err()
        {
            return Err(DomainError::LimitExceeded("owned processes".into()));
        }
        let lease = ActiveLease(self.active.clone());
        match OwnedProcess::launch(spec, spool_path, memory_limit, lease) {
            Ok(process) => Ok(process),
            Err(error) => Err(error),
        }
    }
}

pub enum ProcessCompletion {
    Exited(i32),
    Signaled(i32),
    TimedOut,
}

pub struct OwnedProcess {
    identity: ProcessIdentity,
    process: Handle,
    job: Job,
    stdin: Option<File>,
    readers: Vec<JoinHandle<()>>,
    spool: Arc<Mutex<Spool>>,
    status: Option<ExitStatus>,
    completion: Option<ProcessCompletion>,
    lease: Option<ActiveLease>,
}

impl OwnedProcess {
    fn launch(
        spec: ProcessSpec,
        spool_path: &Path,
        memory_limit: Option<u64>,
        lease: ActiveLease,
    ) -> Result<Self, DomainError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(spool_path)
            .map_err(io_error)?;
        let spool = Arc::new(Mutex::new(Spool {
            file,
            tail: VecDeque::new(),
            written: 0,
            seen: 0,
            truncated: false,
            disk_limit: MAX_LOG_DISK,
            error: None,
        }));

        let (stdin_read, stdin_write) = pipe()?;
        let (stdout_read, stdout_write) = pipe()?;
        let (stderr_read, stderr_write) = pipe()?;
        for parent_handle in [stdin_write.raw(), stdout_read.raw(), stderr_read.raw()] {
            if unsafe { SetHandleInformation(parent_handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
                return Err(win_error("SetHandleInformation"));
            }
        }

        // The handle list is the important part: inherit_handles=true otherwise
        // copies every inheritable handle in AG1W into a model or tool process.
        let inherited = [stdin_read.raw(), stdout_write.raw(), stderr_write.raw()];
        let mut attributes = AttributeList::for_handles(&inherited)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin_read.raw();
        startup.StartupInfo.hStdOutput = stdout_write.raw();
        startup.StartupInfo.hStdError = stderr_write.raw();
        startup.lpAttributeList = attributes.raw();

        let mut command = command_line(&spec.executable, &spec.argv);
        let environment = environment_block(&spec.env);
        let cwd = wide(&spec.cwd);
        let executable = wide(&spec.executable);
        // Create the empty owner before the child exists. If process creation
        // fails, closing this empty job has no effect; if it succeeds, it is
        // ready before the suspended child can execute a single instruction.
        let job = Job::new(memory_limit)?;
        let mut information = PROCESS_INFORMATION::default();
        let flags: PROCESS_CREATION_FLAGS = CREATE_SUSPENDED
            | CREATE_NO_WINDOW
            | CREATE_UNICODE_ENVIRONMENT
            | EXTENDED_STARTUPINFO_PRESENT;
        if unsafe {
            CreateProcessW(
                executable.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                flags,
                environment.as_ptr().cast::<c_void>(),
                cwd.as_ptr(),
                (&startup as *const STARTUPINFOEXW).cast(),
                &mut information,
            )
        } == 0
        {
            return Err(win_error("CreateProcessW"));
        }
        let process = Handle::new(information.hProcess)?;
        let thread = Handle::new(information.hThread)?;
        if let Err(error) = job.assign(process.raw()) {
            // A failed assignment leaves the process suspended, not owned by
            // the job. Terminate that exact handle before returning the error.
            unsafe { TerminateProcess(process.raw(), 1) };
            return Err(error);
        }
        if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
            let error = win_error("ResumeThread");
            let _ = job.terminate();
            return Err(error);
        }
        drop(thread);

        // Once CreateProcess returns, the parent must close child-only pipe ends
        // or a reader would never see EOF after the child exits.
        drop(stdin_read);
        drop(stdout_write);
        drop(stderr_write);

        let identity = identity(&process, information.dwProcessId, &spec.executable)?;
        let readers = vec![
            reader(stdout_read.into_file(), spool.clone())?,
            reader(stderr_read.into_file(), spool.clone())?,
        ];
        Ok(Self {
            identity,
            process,
            job,
            stdin: Some(stdin_write.into_file()),
            readers,
            spool,
            status: None,
            completion: None,
            lease: Some(lease),
        })
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
        let mut bytes = vec![0; limit.min(spool.written - offset as usize)];
        let count = spool.file.seek_read(&mut bytes, offset).map_err(io_error)?;
        bytes.truncate(count);
        Ok(bytes)
    }

    pub fn write_input(&mut self, _bytes: &[u8]) -> Result<usize, DomainError> {
        Err(DomainError::Unavailable(
            "Windows protocol stdin is not implemented yet".into(),
        ))
    }

    pub fn try_write_input(&mut self, _bytes: &[u8]) -> Result<Option<usize>, DomainError> {
        Err(DomainError::Unavailable(
            "Windows protocol stdin is not implemented yet".into(),
        ))
    }

    pub fn close_input(&mut self) {
        self.stdin.take();
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, DomainError> {
        if self.status.is_some() {
            return Ok(self.status);
        }
        match unsafe { WaitForSingleObject(self.process.raw(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0u32;
                if unsafe { GetExitCodeProcess(self.process.raw(), &mut code) } == 0 {
                    return Err(io_error(std::io::Error::last_os_error()));
                }
                self.status = Some(ExitStatus::from_raw(code));
                self.finish();
                Ok(self.status)
            }
            WAIT_FAILED => Err(io_error(std::io::Error::last_os_error())),
            _ => Err(DomainError::Io("unexpected Windows wait result".into())),
        }
    }

    pub fn completion(&self) -> Option<&ProcessCompletion> {
        self.completion.as_ref()
    }

    pub fn wait(&mut self, timeout: Duration) -> Result<ExitStatus, DomainError> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
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
        self.stop_with_timeout(Duration::from_millis(STOP_DEADLINE_MS))
    }

    pub fn stop(&mut self) -> Result<(), DomainError> {
        self.stop_if_identity(&self.identity.clone())
    }

    pub fn stop_with_timeout(&mut self, timeout: Duration) -> Result<(), DomainError> {
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err(DomainError::InvalidInput("owned stop wait budget".into()));
        }
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        self.job.terminate()?;
        let started = Instant::now();
        while self.try_wait()?.is_none() {
            if started.elapsed() >= timeout {
                return Err(DomainError::TimedOut);
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn finish(&mut self) {
        if self.completion.is_none() {
            let code = self.status.and_then(|status| status.code()).unwrap_or(-1);
            self.completion = Some(ProcessCompletion::Exited(code));
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        self.stdin.take();
        self.lease.take();
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if self.status.is_none() {
            let _ = self.job.terminate();
        }
    }
}

fn pipe() -> Result<(Handle, Handle), DomainError> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let mut read = null_mut();
    let mut write = null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
        return Err(win_error("CreatePipe"));
    }
    Ok((Handle::new(read)?, Handle::new(write)?))
}

fn identity(process: &Handle, pid: u32, executable: &Path) -> Result<ProcessIdentity, DomainError> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe {
        GetProcessTimes(
            process.raw(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(win_error("GetProcessTimes"));
    }
    let executable = executable.canonicalize().map_err(io_error)?;
    let file = File::open(&executable).map_err(io_error)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
        return Err(win_error("GetFileInformationByHandle"));
    }
    Ok(ProcessIdentity {
        pid,
        start_ticks: (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64,
        executable,
        device: info.dwVolumeSerialNumber as u64,
        inode: (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64,
    })
}

fn wide(value: &Path) -> Vec<u16> {
    value
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn command_line(executable: &Path, arguments: &[String]) -> Vec<u16> {
    let mut command = quote(executable.as_os_str());
    for argument in arguments {
        command.push(' ');
        command.push_str(&quote(OsStr::new(argument)));
    }
    OsStr::new(&command)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn quote(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if !text.is_empty() && !text.contains([' ', '\t', '"']) {
        return text.into_owned();
    }
    let mut quoted = String::from("\"");
    let mut slashes = 0usize;
    for character in text.chars() {
        match character {
            '\\' => slashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(slashes * 2 + 1));
                quoted.push('"');
                slashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(slashes));
                quoted.push(character);
                slashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(slashes * 2));
    quoted.push('"');
    quoted
}

fn environment_block(environment: &BTreeMap<String, String>) -> Vec<u16> {
    let mut block = String::new();
    for (key, value) in environment {
        block.push_str(key);
        block.push('=');
        block.push_str(value);
        block.push('\0');
    }
    // Windows requires a double-NUL terminator even for an intentionally empty
    // environment. A single NUL is only the end of one entry.
    if environment.is_empty() {
        block.push('\0');
    }
    block.push('\0');
    OsStr::new(&block).encode_wide().collect()
}

fn win_error(operation: &str) -> DomainError {
    DomainError::Io(format!("{operation}: {}", std::io::Error::last_os_error()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32\\cmd.exe")
    }

    fn spec(arguments: &[&str], root: &Path) -> ProcessSpec {
        ProcessSpec {
            executable: command(),
            argv: arguments
                .iter()
                .map(|argument| (*argument).into())
                .collect(),
            cwd: root.to_path_buf(),
            env: BTreeMap::new(),
            timeout_ms: 5_000,
            grace_ms: 100,
            resident: false,
        }
    }

    #[test]
    fn job_owned_child_streams_to_bounded_log() {
        let root = tempfile::Builder::new()
            .prefix("ag1-process-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut process = ProcessSupervisor::new(Path::new("unused"))
            .unwrap()
            .spawn(
                spec(&["/d", "/c", "echo owned-child"], root.path()),
                &root.path().join("log"),
            )
            .unwrap();
        assert!(process.wait(Duration::from_secs(2)).unwrap().success());
        let log = String::from_utf8(process.tail(1024).unwrap().bytes).unwrap();
        assert!(log.contains("owned-child"));
    }

    #[test]
    fn stopping_the_job_stops_its_child_tree() {
        let root = tempfile::Builder::new()
            .prefix("ag1-process-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut process = ProcessSupervisor::new(Path::new("unused"))
            .unwrap()
            .spawn(
                spec(&["/d", "/c", "ping -n 60 127.0.0.1 > nul"], root.path()),
                &root.path().join("log"),
            )
            .unwrap();
        process.stop_with_timeout(Duration::from_secs(2)).unwrap();
        assert!(process.try_wait().unwrap().is_some());
    }
}
