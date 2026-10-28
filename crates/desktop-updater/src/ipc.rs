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
    Foundation::{INVALID_HANDLE_VALUE, LocalFree, WAIT_TIMEOUT},
    Globalization::{CSTR_EQUAL, CompareStringOrdinal},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_READ_DATA, FILE_WRITE_DATA, OPEN_EXISTING,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, SYNCHRONIZE,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::{
            GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
};

/// Retain the kernel process object through authentication and handoff.
pub struct Process(OwnedHandle);
impl Process {
    pub fn open(pid: u32) -> anyhow::Result<Self> {
        ensure!(pid > 0, "进程标识无效");
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
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
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) == WAIT_TIMEOUT }
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
}
