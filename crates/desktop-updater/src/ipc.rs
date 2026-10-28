//! Local Windows pipe transport. Peer identity comes from the kernel, never JSON.
use anyhow::{Context, ensure};
use std::{
    ffi::OsString,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
};
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{FILETIME, INVALID_HANDLE_VALUE, LocalFree},
    Globalization::{CSTR_EQUAL, CompareStringOrdinal},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TOKEN_USER,
        TokenElevation, TokenSessionId, TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_READ_DATA, FILE_WRITE_DATA, OPEN_EXISTING,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, SYNCHRONIZE,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::{
            GetProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    },
};

/// Retain the kernel process object through authentication and handoff.
pub struct Process(OwnedHandle);
pub struct Identity {
    pub sid: String,
    pub session: u32,
    pub elevated: bool,
}
impl Process {
    pub fn identity(&self) -> anyhow::Result<Identity> {
        let mut token = std::ptr::null_mut();
        ensure!(
            unsafe { OpenProcessToken(self.0.as_raw_handle(), TOKEN_QUERY, &mut token) } != 0,
            "无法查询进程令牌：{}",
            std::io::Error::last_os_error()
        );
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let user = token_information(&token, TokenUser, std::mem::size_of::<TOKEN_USER>())?;
        let user = unsafe { &*user.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = std::ptr::null_mut();
        ensure!(
            unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } != 0,
            "无法读取进程用户标识：{}",
            std::io::Error::last_os_error()
        );
        let value = unsafe {
            let mut len = 0;
            while *sid.add(len) != 0 {
                len += 1;
            }
            let value = String::from_utf16(std::slice::from_raw_parts(sid, len));
            LocalFree(sid.cast());
            value
        }?;
        let session = token_information(&token, TokenSessionId, 4)?;
        let elevation = token_information(&token, TokenElevation, 4)?;
        Ok(Identity {
            sid: value,
            session: unsafe { *session.as_ptr().cast::<u32>() },
            elevated: unsafe { *elevation.as_ptr().cast::<u32>() } != 0,
        })
    }
    pub fn open(pid: u32) -> anyhow::Result<Self> {
        ensure!(pid > 0, "进程标识无效");
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        ensure!(
            !handle.is_null(),
            "无法查询进程：{}",
            std::io::Error::last_os_error()
        );
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }
    pub fn pid(&self) -> u32 {
        unsafe { GetProcessId(self.0.as_raw_handle()) }
    }
    pub fn is_running(&self) -> bool {
        // Ordinary users cannot request SYNCHRONIZE on a SYSTEM process.
        // Exit time is available with QUERY_LIMITED_INFORMATION and also avoids
        // treating a process that exited with code STILL_ACTIVE as still alive.
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                self.0.as_raw_handle(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            ) != 0
        }
        &&exit.dwLowDateTime == 0 && exit.dwHighDateTime == 0
    }
    pub fn matches_image(&self, path: &Path) -> anyhow::Result<bool> {
        let expected = match path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let mut image = vec![0u16; 32768];
        let mut size = image.len() as u32;
        ensure!(
            unsafe {
                QueryFullProcessImageNameW(self.0.as_raw_handle(), 0, image.as_mut_ptr(), &mut size)
            } != 0,
            "无法读取进程程序路径：{}",
            std::io::Error::last_os_error()
        );
        let actual = std::path::PathBuf::from(OsString::from_wide(&image[..size as usize]))
            .canonicalize()?;
        let expected: Vec<u16> = expected.as_os_str().encode_wide().collect();
        let actual: Vec<u16> = actual.as_os_str().encode_wide().collect();
        Ok(self.is_running()
            && unsafe {
                CompareStringOrdinal(
                    expected.as_ptr(),
                    expected.len() as i32,
                    actual.as_ptr(),
                    actual.len() as i32,
                    1,
                ) == CSTR_EQUAL
            })
    }
}

fn token_information(
    token: &OwnedHandle,
    class: TOKEN_INFORMATION_CLASS,
    minimum: usize,
) -> anyhow::Result<Vec<usize>> {
    let mut bytes = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            std::ptr::null_mut(),
            0,
            &mut bytes,
        );
    }
    ensure!(
        bytes as usize >= minimum && bytes <= 65536,
        "进程令牌信息大小无效"
    );
    // TOKEN_USER contains a native pointer, so the buffer must be pointer-aligned.
    let mut buffer = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } != 0,
        "无法读取进程令牌信息：{}",
        std::io::Error::last_os_error()
    );
    ensure!(bytes as usize >= minimum, "进程令牌信息不完整");
    Ok(buffer)
}

pub fn listen() -> anyhow::Result<NamedPipeServer> {
    create(&pipe_name())
}
pub fn pipe_name() -> String {
    format!(r"\\.\pipe\{}.desktop-update", env!("DESKTOP_IDENTIFIER"))
}
fn create(name: &str) -> anyhow::Result<NamedPipeServer> {
    // Generic write includes FILE_CREATE_PIPE_INSTANCE. Ordinary clients get
    // only data read/write + synchronization, never permission to host a pipe.
    let sddl: Vec<u16> = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00100003;;;AU)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = std::ptr::null_mut();
    ensure!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } != 0,
        "无法设置升级管道权限：{}",
        std::io::Error::last_os_error()
    );
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .max_instances(1)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    };
    unsafe {
        LocalFree(descriptor);
    }
    Ok(result.context("无法创建独占本机升级管道")?)
}
fn connect_peer(name: &str, expected: &Process) -> anyhow::Result<NamedPipeClient> {
    ensure!(expected.is_running(), "已注册升级服务未运行");
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // IDENTIFICATION prevents an impersonating pipe server from using our token
    // to open files or start processes as the desktop user.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_DATA | FILE_WRITE_DATA | SYNCHRONIZE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            std::ptr::null_mut(),
        )
    };
    ensure!(
        handle != INVALID_HANDLE_VALUE,
        "无法连接升级服务：{}",
        std::io::Error::last_os_error()
    );
    let client = unsafe { NamedPipeClient::from_raw_handle(handle) }?;
    let mut pid = 0;
    ensure!(
        unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) } != 0,
        "无法核验升级服务进程：{}",
        std::io::Error::last_os_error()
    );
    ensure!(
        pid == expected.pid() && expected.is_running(),
        "升级管道不是已注册服务所有"
    );
    Ok(client)
}
pub fn client_process(server: &NamedPipeServer) -> anyhow::Result<Process> {
    let mut pid = 0;
    ensure!(
        unsafe { GetNamedPipeClientProcessId(server.as_raw_handle(), &mut pid) } != 0,
        "无法核验升级客户端：{}",
        std::io::Error::last_os_error()
    );
    let process = Process::open(pid)?;
    ensure!(process.is_running(), "升级客户端已退出");
    Ok(process)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn exclusive_local_pipe_authenticates_both_peer_processes() {
        let name = format!("{}-test-{}", pipe_name(), std::process::id());
        let process = Process::open(std::process::id()).unwrap();
        assert!(process.identity().unwrap().sid.starts_with("S-1-"));
        assert!(
            process
                .matches_image(&std::env::current_exe().unwrap())
                .unwrap()
        );
        assert!(
            !process
                .matches_image(Path::new(r"C:\not-the-updater.exe"))
                .unwrap()
        );
        let mut server = create(&name).unwrap();
        assert!(
            create(&name).is_err(),
            "second server must not impersonate the first"
        );
        let mut client = connect_peer(&name, &process).unwrap();
        server.connect().await.unwrap();
        assert_eq!(client_process(&server).unwrap().pid(), std::process::id());
        client.write_all(b"request").await.unwrap();
        let mut bytes = [0; 7];
        server.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"request");
        server.write_all(b"response").await.unwrap();
        let mut bytes = [0; 8];
        client.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"response");
    }

    // Invoked only by the Windows harness under actual SYSTEM/standard tokens.
    #[tokio::test]
    async fn system_server_fixture() {
        let Some(root) = std::env::var_os("DESKTOP_IPC_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let identity = Process::open(std::process::id())
            .unwrap()
            .identity()
            .unwrap();
        assert_eq!(identity.sid, "S-1-5-18");
        let name = format!("{}-native-{}", pipe_name(), std::process::id());
        let mut server = create(&name).unwrap();
        std::fs::write(
            root.join("ready.json"),
            serde_json::to_vec(&serde_json::json!({
                "pid": std::process::id(), "pipe": name, "image": std::env::current_exe().unwrap()
            }))
            .unwrap(),
        )
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            server.connect().await.unwrap();
            let peer = client_process(&server).unwrap().identity().unwrap();
            assert_ne!(peer.sid, identity.sid);
            assert!(!peer.elevated);
            let mut bytes = [0; 7];
            server.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"request");
            server.write_all(b"response").await.unwrap();
            // Keep server identity alive until the client tests a spoofed pipe.
            let mut done = [0; 4];
            server.read_exact(&mut done).await.unwrap();
            assert_eq!(&done, b"done");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn standard_client_fixture() {
        let Some(root) = std::env::var_os("DESKTOP_IPC_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let identity = Process::open(std::process::id())
            .unwrap()
            .identity()
            .unwrap();
        assert_ne!(identity.sid, "S-1-5-18");
        assert!(
            !identity.elevated,
            "must exercise a genuinely unelevated token"
        );
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("ready.json")).unwrap()).unwrap();
        let server = Process::open(config["pid"].as_u64().unwrap() as u32).unwrap();
        assert_eq!(server.identity().unwrap().sid, "S-1-5-18");
        assert!(
            server
                .matches_image(Path::new(config["image"].as_str().unwrap()))
                .unwrap()
        );
        let name = config["pipe"].as_str().unwrap();
        let mut client = connect_peer(name, &server).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            client.write_all(b"request").await.unwrap();
            let mut response = [0; 8];
            client.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"response");
            assert!(ServerOptions::new().create(name).is_err());
            let spoof_name = format!("{name}-spoof");
            let _spoof = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&spoof_name)
                .unwrap();
            assert!(
                connect_peer(&spoof_name, &server).is_err(),
                "spoofed pipe must fail server authentication"
            );
            client.write_all(b"done").await.unwrap();
        })
        .await
        .unwrap();
    }
}
