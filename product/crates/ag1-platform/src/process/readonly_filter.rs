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

pub(super) fn file(git_startup: bool) -> Result<File, DomainError> {
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
    if git_startup {
        // Git's sanitize_stdfds opens /dev/null O_RDWR at startup. The
        // Git-only variant permits that flag combination, but never permits
        // data writes outside inherited stdout/stderr or descriptor substitution.
        // Read-only namespace mounts still prohibit changing ordinary files.
        for syscall in [
            libc::SYS_write,
            libc::SYS_writev,
            libc::SYS_pwrite64,
            libc::SYS_pwritev,
            libc::SYS_pwritev2,
        ] {
            instructions.extend([
                (EQ, 0, 5, syscall as u32),
                (LOAD, 0, 0, 16),
                (EQ, 2, 0, 1),
                (EQ, 1, 0, 2),
                (RET, 0, 0, DENY),
                (RET, 0, 0, ALLOW),
            ]);
        }
        for syscall in [
            libc::SYS_dup,
            libc::SYS_dup3,
            libc::SYS_sendfile,
            libc::SYS_splice,
            libc::SYS_vmsplice,
            libc::SYS_tee,
            libc::SYS_copy_file_range,
            libc::SYS_close_range,
            libc::SYS_io_setup,
            libc::SYS_io_submit,
            libc::SYS_pidfd_getfd,
        ] {
            instructions.extend([(EQ, 0, 1, syscall as u32), (RET, 0, 0, DENY)]);
        }
        #[cfg(target_arch = "x86_64")]
        instructions.extend([(EQ, 0, 1, libc::SYS_dup2 as u32), (RET, 0, 0, DENY)]);
        // Keep standard output descriptor identities reserved until process
        // exit. A successful no-op close cannot free 1/2 for a host FIFO.
        instructions.extend([
            (EQ, 0, 4, libc::SYS_close as u32),
            (LOAD, 0, 0, 16),
            (EQ, 1, 0, 1),
            (EQ, 0, 1, 2),
            (RET, 0, 0, 0x00050000),
            (LOAD, 0, 0, 0),
        ]);
        // Do not permit close-on-exec to replace those identities either.
        instructions.extend([
            (EQ, 0, 6, libc::SYS_fcntl as u32),
            (LOAD, 0, 0, 24),
            (EQ, 0, 4, libc::F_SETFD as u32),
            (LOAD, 0, 0, 16),
            (EQ, 1, 0, 1),
            (EQ, 0, 1, 2),
            (RET, 0, 0, DENY),
            (LOAD, 0, 0, 0),
        ]);
        instructions.extend([
            (EQ, 0, 4, libc::SYS_fcntl as u32),
            (LOAD, 0, 0, 24),
            (EQ, 1, 0, libc::F_DUPFD as u32),
            (EQ, 0, 1, libc::F_DUPFD_CLOEXEC as u32),
            (RET, 0, 0, DENY),
            (LOAD, 0, 0, 0),
        ]);
    }
    for (syscall, flags) in opens {
        if git_startup {
            instructions.extend([
                (EQ, 0, 3, syscall as u32),
                (LOAD, 0, 0, flags),
                (EQ, 0, 1, libc::O_RDWR as u32),
                (RET, 0, 0, ALLOW),
                (LOAD, 0, 0, 0),
            ]);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn git_startup_exception_cannot_write_a_fifo_or_replace_output_descriptors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.fifo");
        let cpath = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::readonly_filter::tests::git_io_filter_child",
                "--nocapture",
            ])
            .env_clear()
            .env("AG1_GIT_FILTER_FIXTURE_FIFO", &path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Owned filter worker exit: {:?}",
            result.status.code()
        );
        assert!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_fifo()
        );
    }
    #[test]
    fn git_io_filter_child() {
        use std::io::Read;
        let Some(path) = std::env::var_os("AG1_GIT_FILTER_FIXTURE_FIFO") else {
            return;
        };
        let path = std::ffi::CString::new(path.as_encoded_bytes()).unwrap();
        let mut bytes = Vec::new();
        file(true).unwrap().read_to_end(&mut bytes).unwrap();
        let mut filters = bytes
            .chunks_exact(8)
            .map(|b| libc::sock_filter {
                code: u16::from_ne_bytes([b[0], b[1]]),
                jt: b[2],
                jf: b[3],
                k: u32::from_ne_bytes([b[4], b[5], b[6], b[7]]),
            })
            .collect::<Vec<_>>();
        let program = libc::sock_fprog {
            len: filters.len() as u16,
            filter: filters.as_mut_ptr(),
        };
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                || libc::prctl(
                    libc::PR_SET_SECCOMP,
                    libc::SECCOMP_MODE_FILTER,
                    &program as *const libc::sock_fprog,
                ) != 0
            {
                libc::_exit(10);
            }
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
            if null < 0 {
                libc::_exit(11);
            }
            libc::close(null);
            let fifo = libc::open(path.as_ptr(), libc::O_RDWR);
            if fifo < 3 {
                libc::_exit(12);
            }
            if libc::write(fifo, c"x".as_ptr().cast(), 1) != -1
                || *libc::__errno_location() != libc::EPERM
            {
                libc::_exit(13);
            }
            if libc::dup3(fifo, 1, 0) != -1 || *libc::__errno_location() != libc::EPERM {
                libc::_exit(14);
            }
            if libc::fcntl(fifo, libc::F_DUPFD, 1) != -1 || *libc::__errno_location() != libc::EPERM
            {
                libc::_exit(15);
            }
            if libc::close(1) != 0 || libc::fcntl(1, libc::F_GETFD) < 0 {
                libc::_exit(16);
            }
            if libc::fcntl(1, libc::F_SETFD, libc::FD_CLOEXEC) != -1
                || *libc::__errno_location() != libc::EPERM
            {
                libc::_exit(17);
            }
            libc::close(fifo);
            let again = libc::open(path.as_ptr(), libc::O_RDWR);
            if again < 3 {
                libc::_exit(18);
            }
            if libc::write(again, c"x".as_ptr().cast(), 1) != -1 {
                libc::_exit(19);
            }
            libc::_exit(0);
        }
    }
    use std::os::unix::fs::FileTypeExt;
}
