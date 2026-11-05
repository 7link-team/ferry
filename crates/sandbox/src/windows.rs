//! No unrestricted fallback. The only tenant-code process is an AppContainer
//! with no capabilities, a read-only executable directory and a one-process job.
use anyhow::{Context, ensure};
use ferry_protocol::sandbox::{ErrorCode, MAX_RESPONSE_BYTES, Reply};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Write},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, Isolation::*, *},
    Storage::FileSystem::{FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ},
    System::{
        JobObjects::*, Memory::*, Pipes::CreatePipe, SystemInformation::GetWindowsDirectoryW,
        SystemServices::SECURITY_MANDATORY_LOW_RID, Threading::*,
    },
};

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
fn handle(raw: HANDLE) -> anyhow::Result<OwnedHandle> {
    ensure!(
        !raw.is_null() && raw != INVALID_HANDLE_VALUE,
        "Windows handle: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}
fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle()
}
fn checked(ok: i32, operation: &str) -> anyhow::Result<()> {
    ensure!(ok != 0, "{operation}: {}", std::io::Error::last_os_error());
    Ok(())
}
fn token() -> anyhow::Result<OwnedHandle> {
    let mut raw_token = null_mut();
    unsafe {
        checked(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token),
            "OpenProcessToken",
        )?;
    }
    handle(raw_token)
}
fn token_info(token: &OwnedHandle, class: TOKEN_INFORMATION_CLASS) -> anyhow::Result<Vec<usize>> {
    let mut needed = 0;
    unsafe {
        GetTokenInformation(raw(token), class, null_mut(), 0, &mut needed);
    }
    ensure!(needed > 0, "token info size");
    let mut buffer = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    unsafe {
        checked(
            GetTokenInformation(
                raw(token),
                class,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ),
            "GetTokenInformation",
        )?;
    }
    Ok(buffer)
}
pub(crate) fn confined() -> bool {
    (|| -> anyhow::Result<bool> {
        let token = token()?;
        let app = token_info(&token, TokenIsAppContainer)?;
        let integrity = token_info(&token, TokenIntegrityLevel)?;
        let mut in_job = 0;
        unsafe {
            checked(
                IsProcessInJob(GetCurrentProcess(), null_mut(), &mut in_job),
                "IsProcessInJob",
            )?;
            let label = &*(integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
            let count = *GetSidSubAuthorityCount(label.Label.Sid);
            let level = *GetSidSubAuthority(label.Label.Sid, (count - 1) as u32);
            Ok(*(app.as_ptr().cast::<u32>()) != 0
                && level <= SECURITY_MANDATORY_LOW_RID as u32
                && in_job != 0)
        }
    })()
    .unwrap_or(false)
}

struct Profile {
    name: Vec<u16>,
    sid: PSID,
}
impl Profile {
    fn new() -> anyhow::Result<Self> {
        let name = wide(format!("WorkflowSandbox.{}", uuid::Uuid::new_v4().simple()));
        let mut sid = null_mut();
        let result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        ensure!(
            result >= 0 && !sid.is_null(),
            "CreateAppContainerProfile: {result:#x}"
        );
        Ok(Self { name, sid })
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        unsafe {
            DeleteAppContainerProfile(self.name.as_ptr());
            FreeSid(self.sid);
        }
    }
}

/// Protect the dedicated copy, never alter permissions on the installation.
fn executable_acl(path: &Path, app: PSID) -> anyhow::Result<()> {
    let token = token()?;
    let user = token_info(&token, TokenUser)?;
    let user_sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut entries: [EXPLICIT_ACCESS_W; 2] = unsafe { zeroed() };
    for (entry, sid, rights) in [
        (&mut entries[0] as *mut _, user_sid, FILE_ALL_ACCESS),
        (
            &mut entries[1] as *mut _,
            app,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        ),
    ] {
        unsafe {
            let entry: &mut EXPLICIT_ACCESS_W = &mut *entry;
            entry.grfAccessPermissions = rights;
            entry.grfAccessMode = SET_ACCESS;
            entry.grfInheritance = SUB_CONTAINERS_AND_OBJECTS_INHERIT;
            BuildTrusteeWithSidW(&mut entry.Trustee, sid);
        }
    }
    let mut acl = null_mut();
    let result =
        unsafe { SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), null(), &mut acl) };
    ensure!(result == ERROR_SUCCESS, "SetEntriesInAclW: {result}");
    let path = wide(path);
    let result = unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    unsafe {
        LocalFree(acl.cast());
    }
    ensure!(result == ERROR_SUCCESS, "SetNamedSecurityInfoW: {result}");
    Ok(())
}

struct Attributes {
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new() -> anyhow::Result<Self> {
        let mut size = 0;
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size);
        }
        ensure!(size > 0, "attribute list size");
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        unsafe {
            checked(
                InitializeProcThreadAttributeList(pointer, 2, 0, &mut size),
                "InitializeProcThreadAttributeList",
            )?;
        }
        Ok(Self {
            _storage: storage,
            pointer,
        })
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

fn pipe() -> anyhow::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (null_mut(), null_mut());
    let attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    unsafe {
        checked(CreatePipe(&mut read, &mut write, &attrs, 0), "CreatePipe")?;
    }
    Ok((handle(read)?, handle(write)?))
}
fn job() -> anyhow::Result<OwnedHandle> {
    let job = handle(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = 256 * 1024 * 1024;
    unsafe {
        checked(
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                size_of_val(&limits) as u32,
            ),
            "job limits",
        )?;
    }
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_EXITWINDOWS,
    };
    unsafe {
        checked(
            SetInformationJobObject(
                raw(&job),
                JobObjectBasicUIRestrictions,
                &ui as *const _ as _,
                size_of_val(&ui) as u32,
            ),
            "job UI limits",
        )?;
    }
    Ok(job)
}

#[derive(Debug)]
struct Deadline;
impl std::fmt::Display for Deadline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sandbox deadline")
    }
}
impl std::error::Error for Deadline {}

fn launch(bytes: &[u8], timeout_ms: u64, inner_flag: &str) -> anyhow::Result<Vec<u8>> {
    let profile = Profile::new()?;
    let directory = tempfile::tempdir()?;
    let executable = directory.path().join("sandbox.exe");
    std::fs::copy(std::env::current_exe()?, &executable)?;
    executable_acl(directory.path(), profile.sid)?;
    executable_acl(&executable, profile.sid)?;
    let job = job()?;
    let (child_input, parent_input) = pipe()?;
    let (parent_output, child_output) = pipe()?;
    unsafe {
        checked(
            SetHandleInformation(raw(&parent_input), HANDLE_FLAG_INHERIT, 0),
            "input inheritance",
        )?;
        checked(
            SetHandleInformation(raw(&parent_output), HANDLE_FLAG_INHERIT, 0),
            "output inheritance",
        )?;
    }
    let attrs = Attributes::new()?;
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let handles = [raw(&child_input), raw(&child_output)];
    unsafe {
        checked(
            UpdateProcThreadAttribute(
                attrs.pointer,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                &capabilities as *const _ as _,
                size_of_val(&capabilities),
                null_mut(),
                null(),
            ),
            "appcontainer attribute",
        )?;
        checked(
            UpdateProcThreadAttribute(
                attrs.pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                size_of_val(&handles),
                null_mut(),
                null(),
            ),
            "handle allowlist",
        )?;
    }
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = raw(&child_input);
    startup.StartupInfo.hStdOutput = raw(&child_output);
    startup.StartupInfo.hStdError = raw(&child_output);
    startup.lpAttributeList = attrs.pointer;
    let exe = wide(&executable);
    let mut command = wide(format!("\"{}\" {inner_flag}", executable.display()));
    let cwd = wide(directory.path());
    let mut system_root = vec![0u16; 32768];
    let size = unsafe { GetWindowsDirectoryW(system_root.as_mut_ptr(), system_root.len() as u32) }
        as usize;
    ensure!(size > 0 && size < system_root.len(), "Windows directory");
    // Windows expands profile/system-directory variables while creating a
    // lowbox process. Only these locations are inherited, never PATH, business
    // configuration, database URLs, vault keys or arbitrary user variables.
    let allowed = [
        "windir",
        "systemdrive",
        "programdata",
        "allusersprofile",
        "programfiles",
        "programfiles(x86)",
        "commonprogramfiles",
        "userprofile",
        "localappdata",
        "appdata",
        "temp",
        "tmp",
    ];
    let mut entries: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| allowed.contains(&key.to_string_lossy().to_ascii_lowercase().as_str()))
        .collect();
    entries.push((
        "SystemRoot".into(),
        std::ffi::OsString::from_wide(&system_root[..size]),
    ));
    entries.sort_by_key(|(key, _)| key.to_string_lossy().to_ascii_lowercase());
    let mut environment = Vec::<u16>::new();
    for (key, value) in entries {
        environment.extend(key.encode_wide());
        environment.push('=' as u16);
        environment.extend(value.encode_wide());
        environment.push(0);
    }
    environment.push(0);
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    unsafe {
        checked(
            CreateProcessW(
                exe.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            ),
            "CreateProcessW",
        )?;
    }
    let process = handle(info.hProcess)?;
    let thread = handle(info.hThread)?;
    unsafe {
        if AssignProcessToJobObject(raw(&job), raw(&process)) == 0 {
            TerminateProcess(raw(&process), 1);
            anyhow::bail!(
                "AssignProcessToJobObject: {}",
                std::io::Error::last_os_error()
            );
        }
        if ResumeThread(raw(&thread)) == u32::MAX {
            TerminateProcess(raw(&process), 1);
            anyhow::bail!("ResumeThread: {}", std::io::Error::last_os_error());
        }
    }
    drop(child_input);
    drop(child_output);
    drop(thread);
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        let mut file = File::from(parent_input);
        file.write_all(&input)
    });
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        File::from(parent_output)
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let wait = unsafe { WaitForSingleObject(raw(&process), (timeout_ms + 3_000) as u32) };
    if wait != WAIT_OBJECT_0 {
        unsafe {
            TerminateJobObject(raw(&job), 1);
            WaitForSingleObject(raw(&process), 5_000);
        }
    }
    let write_result = writer.join().map_err(|_| anyhow::anyhow!("input thread"))?;
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("output thread"))??;
    if wait == WAIT_TIMEOUT {
        return Err(Deadline.into());
    }
    ensure!(wait == WAIT_OBJECT_0, "wait process");
    write_result.context("sandbox input")?;
    let mut exit = 0;
    unsafe {
        checked(
            GetExitCodeProcess(raw(&process), &mut exit),
            "GetExitCodeProcess",
        )?;
    }
    ensure!(exit == 0, "sandbox child exit: {exit:#x}");
    Ok(output)
}
pub(crate) fn evaluate(bytes: &[u8], timeout_ms: u64) -> Reply {
    match launch(bytes, timeout_ms, "--inner") {
        Ok(output) if output.len() > MAX_RESPONSE_BYTES => {
            Reply::error(ErrorCode::ResultLimit, None)
        }
        Ok(output) => serde_json::from_slice(&output)
            .unwrap_or_else(|_| Reply::error(ErrorCode::Internal, None)),
        Err(error) if error.is::<Deadline>() => Reply::error(ErrorCode::Timeout, None),
        Err(error) => {
            eprintln!("sandbox confinement refused: {error}");
            Reply::error(ErrorCode::Isolation, None)
        }
    }
}

/// Explicit local diagnostic, never a JS binding or an evaluation operation.
/// It executes native escape attempts inside exactly the same confinement.
pub(crate) fn diagnostic(args: &[String]) -> Option<serde_json::Value> {
    let inner = args == ["--inner-probe"];
    if !inner && args != ["--isolation-probe"] {
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
        if !inner {
            let output = launch(&bytes, 1000, "--inner-probe")?;
            return Ok(serde_json::from_slice(&output)?);
        }
        ensure!(confined(), "native probe requires confinement");
        let filesystem_denied = std::fs::read(&probe.path).is_err();
        let write_denied = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(format!("{}.denied", probe.path))
            .is_err();
        let network_denied = std::net::TcpStream::connect_timeout(
            &probe.address,
            std::time::Duration::from_millis(300),
        )
        .is_err();
        use std::os::windows::process::CommandExt;
        let child_denied = match std::process::Command::new(std::env::current_exe()?)
            .arg("--inner")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            Err(_) => true,
            Ok(mut child) => {
                let _ = child.kill();
                let _ = child.wait();
                false
            }
        };
        let memory_denied = unsafe {
            let memory = VirtualAlloc(
                null(),
                512 * 1024 * 1024,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            );
            if memory.is_null() {
                true
            } else {
                VirtualFree(memory, 0, MEM_RELEASE);
                false
            }
        };
        let environment_clean = std::env::vars_os().all(|(key, _)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            !key.starts_with("FERRY_") && key != "SANDBOX_CANARY_SECRET"
        });
        Ok(serde_json::json!({"status":"ok","version":1,"probe":{
            "appcontainer_low_integrity_job":true,"filesystem_denied":filesystem_denied,"write_denied":write_denied,
            "network_denied":network_denied,"child_denied":child_denied,"os_memory_denied":memory_denied,"environment_clean":environment_clean
        }}))
    })();
    Some(match result {
        Ok(result) => result,
        Err(error) => {
            eprintln!("sandbox diagnostic refused: {error}");
            serde_json::to_value(Reply::error(ErrorCode::Isolation, None)).unwrap()
        }
    })
}
