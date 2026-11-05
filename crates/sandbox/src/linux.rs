//! The re-executed worker receives no environment or inherited descriptors.
//! Tenant code starts only after identity, rlimits and seccomp are installed.
use anyhow::{Context, ensure};
use ferry_protocol::sandbox::{Batch, ErrorCode, MAX_RESPONSE_BYTES, Reply};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn checked(result: libc::c_int, operation: &str) -> anyhow::Result<()> {
    ensure!(
        result == 0,
        "{operation}: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

fn limit(resource: u32, value: libc::rlim_t) -> anyhow::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    checked(
        unsafe { libc::setrlimit(resource as _, &limit) },
        "setrlimit",
    )
}

fn identity() -> anyhow::Result<()> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    if uid == 0 {
        checked(
            unsafe { libc::setgroups(0, std::ptr::null()) },
            "clear groups",
        )?;
    } else {
        // No host account or privileged setuid helper is needed. If the kernel
        // disables unprivileged user namespaces, refuse rather than run as server.
        checked(
            unsafe { libc::unshare(libc::CLONE_NEWUSER) },
            "user namespace",
        )?;
        std::fs::write("/proc/self/setgroups", "deny")?;
        std::fs::write("/proc/self/uid_map", format!("65534 {uid} 1\n"))?;
        std::fs::write("/proc/self/gid_map", format!("65534 {gid} 1\n"))?;
    }
    checked(
        unsafe { libc::setresgid(65534, 65534, 65534) },
        "sandbox gid",
    )?;
    checked(
        unsafe { libc::setresuid(65534, 65534, 65534) },
        "sandbox uid",
    )?;
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Capabilities {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x2008_0522,
        pid: 0,
    };
    let caps = [Capabilities {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    checked(
        unsafe { libc::syscall(libc::SYS_capset, &header, &caps) } as i32,
        "clear capabilities",
    )?;
    checked(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) },
        "disable dumps",
    )?;
    checked(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        "no new privileges",
    )?;
    Ok(())
}

fn filter() -> anyhow::Result<()> {
    // Reject other ABIs (including x32) before considering syscall numbers.
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc000_00b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    anyhow::bail!("unsupported Linux sandbox architecture");
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
        let mut code = vec![
            instruction(0x20, 0, 0, 4), // load seccomp_data.arch
            instruction(0x15, 1, 0, arch),
            instruction(0x06, 0, 0, 0x8000_0000), // KILL_PROCESS
            instruction(0x20, 0, 0, 0),           // load seccomp_data.nr
        ];
        // No open/openat, socket, connect, exec, clone, ptrace, kill, ioctl,
        // process_vm_*, io_uring or descriptor-transfer interfaces are allowed.
        for syscall in [
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_close,
            libc::SYS_fstat,
            libc::SYS_lseek,
            libc::SYS_mmap,
            libc::SYS_mprotect,
            libc::SYS_munmap,
            libc::SYS_mremap,
            libc::SYS_madvise,
            libc::SYS_brk,
            libc::SYS_rt_sigaction,
            libc::SYS_rt_sigprocmask,
            libc::SYS_rt_sigreturn,
            libc::SYS_sigaltstack,
            libc::SYS_clock_gettime,
            libc::SYS_gettimeofday,
            libc::SYS_futex,
            libc::SYS_sched_yield,
            libc::SYS_getrandom,
            libc::SYS_getpid,
            libc::SYS_getppid,
            libc::SYS_gettid,
            libc::SYS_getuid,
            libc::SYS_geteuid,
            libc::SYS_getgid,
            libc::SYS_getegid,
            libc::SYS_exit,
            libc::SYS_exit_group,
        ] {
            code.push(instruction(0x15, 0, 1, syscall as u32));
            code.push(instruction(0x06, 0, 0, 0x7fff_0000)); // ALLOW
        }
        code.push(instruction(0x06, 0, 0, 0x0005_0000 | libc::EPERM as u32));
        let program = libc::sock_fprog {
            len: code.len() as u16,
            filter: code.as_mut_ptr(),
        };
        checked(
            unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program, 0, 0) },
            "seccomp",
        )
    }
}

fn confine(parent: libc::pid_t, timeout_ms: u64) -> anyhow::Result<()> {
    identity()?;
    // setuid resets PDEATHSIG: restore it after changing identity, then close
    // the race by checking the exact launcher pid captured before the spawn.
    checked(
        unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) },
        "parent death",
    )?;
    ensure!(unsafe { libc::getppid() } == parent, "launcher exited");
    limit(libc::RLIMIT_AS as _, 256 * 1024 * 1024)?;
    limit(libc::RLIMIT_CORE as _, 0)?;
    limit(libc::RLIMIT_NPROC as _, 0)?;
    limit(libc::RLIMIT_CPU as _, timeout_ms.div_ceil(1000) + 2)?;
    // Descriptors 0/1/2 are private pipes or /dev/null. Never let native code
    // see an inherited database, file, socket, pidfd or directory descriptor.
    checked(
        unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } as i32,
        "close descriptors",
    )?;
    limit(libc::RLIMIT_NOFILE as _, 3)?;
    filter()
}

pub(crate) fn worker(batch: Batch, parent: libc::pid_t) -> Reply {
    if confine(parent, batch.timeout_ms).is_err() {
        return Reply::error(ErrorCode::Isolation, None);
    }
    crate::engine::evaluate(batch)
}

pub(crate) fn evaluate(bytes: &[u8], timeout_ms: u64) -> Reply {
    match launch(bytes, timeout_ms, "--linux-worker") {
        Ok(output) => serde_json::from_slice(&output)
            .unwrap_or_else(|_| Reply::error(ErrorCode::Internal, None)),
        Err(_) => Reply::error(ErrorCode::Isolation, None),
    }
}

fn launch(bytes: &[u8], timeout_ms: u64, flag: &str) -> anyhow::Result<Vec<u8>> {
    let parent = unsafe { libc::getpid() };
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([flag, &parent.to_string()])
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                || libc::getppid() != parent
                || libc::setpgid(0, 0) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("spawn Linux sandbox")?;
    let input = bytes.to_vec();
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms + 3_000);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let write = writer
        .join()
        .map_err(|_| anyhow::anyhow!("sandbox input thread"))?;
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("sandbox output thread"))??;
    let Some(status) = status else {
        return Ok(serde_json::to_vec(&Reply::error(ErrorCode::Timeout, None))?);
    };
    ensure!(status.success(), "sandbox worker failed");
    write?;
    if output.len() > MAX_RESPONSE_BYTES {
        return Ok(serde_json::to_vec(&Reply::error(
            ErrorCode::ResultLimit,
            None,
        ))?);
    }
    Ok(output)
}

/// Native escape attempts behind exactly the same boundary as QuickJS.
/// A local diagnostic command, never a JS binding or graph operation.
pub(crate) fn diagnostic(args: &[String]) -> Option<serde_json::Value> {
    let worker = args.len() == 2 && args[0] == "--linux-probe";
    if !worker && args != ["--isolation-probe"] {
        return None;
    }
    let result = (|| -> anyhow::Result<serde_json::Value> {
        let bytes = crate::read_request()?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Probe {
            path: String,
            address: std::net::SocketAddr,
        }
        let probe: Probe = serde_json::from_slice(&bytes)?;
        if !worker {
            return Ok(serde_json::from_slice(&launch(
                &bytes,
                1000,
                "--linux-probe",
            )?)?);
        }
        let parent: i32 = args[1].parse()?;
        ensure!(parent > 1, "invalid parent");
        confine(parent, 1000)?;
        let identity = unsafe { libc::geteuid() == 65534 && libc::getegid() == 65534 };
        let filesystem_denied = std::fs::read(&probe.path).is_err();
        let write_denied = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(format!("{}.denied", probe.path))
            .is_err();
        let network_denied =
            std::net::TcpStream::connect_timeout(&probe.address, Duration::from_millis(300))
                .is_err();
        let child_denied = unsafe {
            match libc::fork() {
                -1 => true,
                0 => libc::_exit(99),
                pid => {
                    libc::waitpid(pid, std::ptr::null_mut(), 0);
                    false
                }
            }
        };
        let memory_denied = unsafe {
            let memory = libc::mmap(
                std::ptr::null_mut(),
                512 * 1024 * 1024,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            );
            if memory == libc::MAP_FAILED {
                true
            } else {
                libc::munmap(memory, 512 * 1024 * 1024);
                false
            }
        };
        Ok(serde_json::json!({"status":"ok","version":1,"probe":{
            "low_privilege_identity":identity,"filesystem_denied":filesystem_denied,
            "write_denied":write_denied,"network_denied":network_denied,
            "child_denied":child_denied,"os_memory_denied":memory_denied,
            "environment_clean":std::env::vars_os().next().is_none()
        }}))
    })();
    Some(result.unwrap_or_else(|_| {
        serde_json::to_value(Reply::error(ErrorCode::Isolation, None)).unwrap()
    }))
}
