//! Extra restrictions installed by bubblewrap *after* namespace setup. A
//! read-only bind alone still permits opening host FIFOs and UNIX sockets.
//! This filter admits read-only opens and inherited stdout/stderr pipe writes.
use crate::io_error;
use ag1_core::DomainError;
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::fd::FromRawFd,
};

pub(super) fn file() -> Result<File, DomainError> {
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e; // AUDIT_ARCH_X86_64
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7; // AUDIT_ARCH_AARCH64
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const ARCH: u32 = 0; // Unknown ABI must fail closed.
    if ARCH == 0 {
        return Err(DomainError::Unavailable(
            "readonly shell syscall ABI".into(),
        ));
    }
    const LOAD: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const EQ: u16 = 0x15;
    const SET: u16 = 0x45;
    const RET: u16 = 0x06;
    const DENY: u32 = 0x00050000 | libc::EPERM as u32;
    const ALLOW: u32 = 0x7fff0000;
    let mut instructions: Vec<(u16, u8, u8, u32)> = vec![
        (LOAD, 0, 0, 4), // seccomp_data.arch
        (EQ, 1, 0, ARCH),
        (RET, 0, 0, 0x80000000), // KILL_PROCESS for alternate syscall ABIs
        (LOAD, 0, 0, 0),         // seccomp_data.nr
    ];
    #[cfg(target_arch = "x86_64")]
    instructions.extend([(SET, 0, 1, 0x40000000), (RET, 0, 0, DENY)]); // x32
    for syscall in [
        libc::SYS_socket,
        libc::SYS_io_uring_setup,
        libc::SYS_openat2,
        libc::SYS_open_by_handle_at,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsmount,
        libc::SYS_ptrace,
        libc::SYS_process_vm_writev,
        libc::SYS_bpf,
    ] {
        instructions.extend([(EQ, 0, 1, syscall as u32), (RET, 0, 0, DENY)]);
    }
    #[cfg(target_arch = "x86_64")]
    instructions.extend([(EQ, 0, 1, libc::SYS_creat as u32), (RET, 0, 0, DENY)]);
    // Flags offsets in seccomp_data.args. Reject write access, truncation,
    // creation and append even on existing FIFOs; allow O_DIRECTORY reads.
    let writable = (libc::O_ACCMODE
        | libc::O_CREAT
        | libc::O_TRUNC
        | libc::O_APPEND
        | (libc::O_TMPFILE & !libc::O_DIRECTORY)) as u32;
    let mut opens = vec![(libc::SYS_openat, 32)];
    #[cfg(target_arch = "x86_64")]
    opens.push((libc::SYS_open, 24));
    for (syscall, flags) in opens {
        instructions.extend([
            (EQ, 0, 4, syscall as u32),
            (LOAD, 0, 0, flags),
            (SET, 0, 1, writable),
            (RET, 0, 0, DENY),
            (RET, 0, 0, ALLOW),
        ]);
    }
    instructions.push((RET, 0, 0, ALLOW));
    let fd = unsafe {
        libc::memfd_create(
            c"ag1-readonly-filter".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    for (code, jt, jf, k) in instructions {
        file.write_all(&code.to_ne_bytes())
            .and_then(|_| file.write_all(&[jt, jf]))
            .and_then(|_| file.write_all(&k.to_ne_bytes()))
            .map_err(io_error)?;
    }
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    if unsafe {
        libc::fcntl(
            fd,
            libc::F_ADD_SEALS,
            libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE,
        )
    } < 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(file)
}
