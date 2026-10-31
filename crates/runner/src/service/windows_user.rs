//! Windows current-user installation: a per-user logon task instead of an SCM
//! service. It needs no administrator rights or stored password, and runs with
//! the user's own token, profile and PATH while that user is logged on.
use super::*;
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LocalFree},
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    System::Threading::{
        CreateEventW, CreateMutexW, GetCurrentProcess, INFINITE, OpenEventW, OpenMutexW,
        OpenProcessToken, ReleaseMutex, SetEvent, WaitForSingleObject,
    },
};

const WAIT_OBJECT_0: u32 = 0;
const WAIT_ABANDONED: u32 = 0x80;
const SYNCHRONIZE: u32 = 0x0010_0000;
const EVENT_MODIFY_STATE: u32 = 0x0002;

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(Some(0)).collect()
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// The installing user's SID, which names the task and its control objects so
/// two users of one machine never share a registration.
fn current_sid() -> anyhow::Result<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        anyhow::ensure!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) != 0,
            "无法读取当前用户身份：{}",
            std::io::Error::last_os_error()
        );
        let token = Handle(token);
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        anyhow::ensure!(
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size
            ) != 0,
            "无法读取当前用户身份：{}",
            std::io::Error::last_os_error()
        );
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text: *mut u16 = std::ptr::null_mut();
        anyhow::ensure!(
            ConvertSidToStringSidW(user.User.Sid, &mut text) != 0,
            "无法读取当前用户身份：{}",
            std::io::Error::last_os_error()
        );
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        let sid = String::from_utf16(std::slice::from_raw_parts(text, len))?;
        LocalFree(text.cast());
        Ok(sid)
    }
}

fn task_name(name: &str, sid: &str) -> String {
    format!("{name}-{sid}")
}
// Session-local objects: the task, its shells and the user's own commands all
// run in the user's logon session.
fn running_object(name: &str, sid: &str) -> String {
    format!("Local\\{name}-{sid}-running")
}
fn stop_object(name: &str, sid: &str) -> String {
    format!("Local\\{name}-{sid}-stop")
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn quote(path: &Path) -> anyhow::Result<String> {
    let value = path.to_str().context("服务路径不是Unicode")?;
    anyhow::ensure!(
        !value.contains('"') && !value.chars().any(char::is_control),
        "服务路径无效"
    );
    Ok(format!("\"{value}\""))
}

/// Arguments of the task action. conhost's headless mode gives the console
/// program a console without ever creating a window on the user's desktop.
fn task_arguments(binary: &Path, config: &Path, name: &str) -> anyhow::Result<String> {
    Ok(format!(
        "--headless {} service run --config {} --name {name}",
        quote(binary)?,
        quote(config)?
    ))
}

fn task_xml(sid: &str, conhost: &Path, arguments: &str) -> anyhow::Result<String> {
    let user = xml(sid);
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\
<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\
<RegistrationInfo><Description>Host execution agent (current user)</Description></RegistrationInfo>\
<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>\
<Principals><Principal id=\"Author\"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>\
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\
<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\
<AllowHardTerminate>true</AllowHardTerminate><StartWhenAvailable>true</StartWhenAvailable>\
<RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\
<IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>\
<AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>false</Hidden>\
<RunOnlyIfIdle>false</RunOnlyIfIdle><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>7</Priority>\
<RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure></Settings>\
<Actions Context=\"Author\"><Exec><Command>{}</Command><Arguments>{}</Arguments></Exec></Actions>\
</Task>",
        xml(conhost.to_str().context("系统路径不是Unicode")?),
        xml(arguments)
    ))
}

fn conhost() -> anyhow::Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot未设置")?)
            .join("System32")
            .join("conhost.exe"),
    )
}

fn schtasks() -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("schtasks.exe");
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command
}

/// The registered task's action arguments, or None when it does not exist.
/// schtasks prints in the console code page, so read the parsed definition as
/// UTF-8 instead; the task name is data in the environment, never script text.
fn query(task: &str) -> anyhow::Result<Option<String>> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("powershell.exe")
        .env_remove("PSModulePath")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); \
             $task = Get-ScheduledTask -TaskPath '\\' -TaskName $env:RUNNER_TASK_NAME -ErrorAction SilentlyContinue; \
             if (-not $task) { exit 3 }; [Console]::Out.Write($task.Actions[0].Arguments)",
        ])
        .env("RUNNER_TASK_NAME", task)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .context("无法调用计划任务")?;
    match output.status.code() {
        Some(0) => Ok(Some(String::from_utf8(output.stdout)?)),
        Some(3) => Ok(None),
        _ => anyhow::bail!(
            "无法查询用户计划任务：{}",
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

pub(super) fn register(identity: &Identity, binary: &Path, config: &Path) -> anyhow::Result<()> {
    validate_name(&identity.name)?;
    let sid = current_sid()?;
    let task = task_name(&identity.name, &sid);
    let arguments = task_arguments(binary, config, &identity.name)?;
    if let Some(existing) = query(&task)? {
        anyhow::ensure!(
            existing == arguments,
            "已有同名用户计划任务使用不同配置，拒绝覆盖：{task}"
        );
        return Ok(());
    }
    let definition = task_xml(&sid, &conhost()?, &arguments)?;
    let root = config.parent().context("服务配置目录无效")?;
    let mut file = tempfile::Builder::new().suffix(".xml").tempfile_in(root)?;
    let bytes: Vec<u8> = std::iter::once(0xFEFF_u16)
        .chain(definition.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    // schtasks opens the definition exclusively; keep only the path, which is
    // still removed when dropped.
    let file = file.into_temp_path();
    command(
        schtasks()
            .args(["/Create", "/TN", &task, "/XML"])
            .arg(&file),
    )
}

pub(super) fn control(identity: &Identity, action: &str) -> anyhow::Result<()> {
    validate_name(&identity.name)?;
    let sid = current_sid()?;
    let task = task_name(&identity.name, &sid);
    match action {
        "start" => {
            anyhow::ensure!(query(&task)?.is_some(), "未找到用户计划任务：{task}");
            if open_running(&identity.name, &sid)?.is_none() {
                command(schtasks().args(["/Run", "/TN", &task]))?;
            }
            Ok(())
        }
        "stop" => stop(&identity.name, &sid),
        "status" => {
            let state = if query(&task)?.is_none() {
                "NotInstalled"
            } else if open_running(&identity.name, &sid)?.is_some() {
                "Running"
            } else {
                "Stopped"
            };
            println!("{}: {state}, task={task}", identity.name);
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn uninstall(identity: &Identity) -> anyhow::Result<()> {
    validate_name(&identity.name)?;
    let sid = current_sid()?;
    stop(&identity.name, &sid)?;
    let task = task_name(&identity.name, &sid);
    if query(&task)?.is_some() {
        command(schtasks().args(["/Delete", "/TN", &task, "/F"]))?;
    }
    Ok(())
}

fn open_running(name: &str, sid: &str) -> anyhow::Result<Option<Handle>> {
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, wide(&running_object(name, sid)).as_ptr()) };
    Ok((!handle.is_null()).then_some(Handle(handle)))
}

/// Ask the running agent to stop, then wait until its process has released the
/// running mutex: only then may an install replace the program files.
fn stop(name: &str, sid: &str) -> anyhow::Result<()> {
    let Some(running) = open_running(name, sid)? else {
        return Ok(());
    };
    let event = open_stop(name, sid, EVENT_MODIFY_STATE)?;
    anyhow::ensure!(unsafe { SetEvent(event.0) } != 0, "无法通知 Runner 停止");
    match unsafe { WaitForSingleObject(running.0, 30_000) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => {
            unsafe { ReleaseMutex(running.0) };
            Ok(())
        }
        _ => anyhow::bail!("等待 Runner 进程退出超时，未替换程序"),
    }
}

fn open_stop(name: &str, sid: &str, access: u32) -> anyhow::Result<Handle> {
    let event = unsafe { OpenEventW(access, 0, wide(&stop_object(name, sid)).as_ptr()) };
    anyhow::ensure!(
        !event.is_null(),
        "无法打开 Runner 停止信号：{}",
        std::io::Error::last_os_error()
    );
    Ok(Handle(event))
}

fn log(config: &Path, message: std::fmt::Arguments) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.with_file_name("service.log"))
    {
        let _ = writeln!(file, "{message}");
    }
}

/// Task action entry point and the user installation's service manager. Task
/// Scheduler neither restarts an exited action nor sees the agent's exit code
/// through conhost, so this small process restarts the supervisor itself, as
/// SCM recovery, systemd Restart= and launchd KeepAlive do elsewhere.
pub(super) fn run(config: PathBuf, name: String) -> anyhow::Result<()> {
    let sid = current_sid()?;
    let running = unsafe {
        CreateMutexW(
            std::ptr::null(),
            1,
            wide(&running_object(&name, &sid)).as_ptr(),
        )
    };
    anyhow::ensure!(!running.is_null(), "无法创建 Runner 运行标记");
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(running) };
        return Ok(()); // Logon trigger or manual start while already running.
    }
    // Held by this thread until the process exits; waiters then see it abandoned.
    let _running = Handle(running);
    let event = unsafe {
        CreateEventW(
            std::ptr::null(),
            1,
            0,
            wide(&stop_object(&name, &sid)).as_ptr(),
        )
    };
    anyhow::ensure!(!event.is_null(), "无法创建 Runner 停止信号");
    let event = Handle(event);
    let binary = std::env::current_exe()?;
    loop {
        use std::os::windows::{io::AsRawHandle, process::CommandExt};
        let child = Command::new(&binary)
            .args(["service", "supervise", "--config"])
            .arg(&config)
            .args(["--name", &name])
            .stdin(std::process::Stdio::null())
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                log(
                    &config,
                    format_args!("无法启动 Runner：{error:#}，5 秒后重试"),
                );
                if unsafe { WaitForSingleObject(event.0, 5_000) } == WAIT_OBJECT_0 {
                    return Ok(());
                }
                continue;
            }
        };
        let handles = [child.as_raw_handle() as HANDLE, event.0];
        let signalled = unsafe {
            windows_sys::Win32::System::Threading::WaitForMultipleObjects(
                2,
                handles.as_ptr(),
                0,
                INFINITE,
            )
        };
        if signalled == WAIT_OBJECT_0 + 1 {
            // The supervisor observes the same event and drains its worker.
            if unsafe { WaitForSingleObject(handles[0], 25_000) } != WAIT_OBJECT_0 {
                let _ = child.kill();
            }
            let _ = child.wait();
            return Ok(());
        }
        let status = child.wait()?;
        log(
            &config,
            format_args!("Runner 进程退出（{status}），5 秒后重新启动"),
        );
        if unsafe { WaitForSingleObject(event.0, 5_000) } == WAIT_OBJECT_0 {
            return Ok(());
        }
    }
}

/// The supervisor child of [`run`]: the stop event replaces SCM stop controls.
pub(super) fn supervise(config: PathBuf, name: String) -> anyhow::Result<()> {
    let sid = current_sid()?;
    let event = open_stop(&name, &sid, SYNCHRONIZE)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let (stop, stopped) = tokio::sync::watch::channel(false);
            let raw = event.0 as usize;
            std::thread::spawn(move || {
                unsafe { WaitForSingleObject(raw as HANDLE, INFINITE) };
                stop.send_replace(true);
            });
            let result = supervisor::run(config.clone(), stopped).await;
            if let Err(error) = &result {
                log(&config, format_args!("服务退出：{error:#}"));
            }
            result
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_definition_runs_headless_as_the_user_at_logon_without_time_limit() {
        let arguments = task_arguments(
            Path::new(r"C:\Users\A & B\AppData\Local\runner-agent\runner-service.exe"),
            Path::new(r"C:\Users\A & B\AppData\Local\runner-agent\service.json"),
            "runner-agent",
        )
        .unwrap();
        assert_eq!(
            arguments,
            r#"--headless "C:\Users\A & B\AppData\Local\runner-agent\runner-service.exe" service run --config "C:\Users\A & B\AppData\Local\runner-agent\service.json" --name runner-agent"#
        );
        let definition = task_xml(
            "S-1-5-21-1-2-3-1001",
            Path::new(r"C:\Windows\System32\conhost.exe"),
            &arguments,
        )
        .unwrap();
        for part in [
            "<LogonTrigger><Enabled>true</Enabled><UserId>S-1-5-21-1-2-3-1001</UserId>",
            "<LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel>",
            "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
            "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            r"<Command>C:\Windows\System32\conhost.exe</Command>",
            "A &amp; B",
            "&quot;C:",
        ] {
            assert!(definition.contains(part), "{part}\n{definition}");
        }
        assert!(task_arguments(Path::new("C:\\a\"b.exe"), Path::new("C:\\c.json"), "x").is_err());
    }

    #[test]
    fn task_and_control_objects_are_scoped_to_service_name_and_user() {
        assert_eq!(
            task_name("runner-agent", "S-1-5-21-9"),
            "runner-agent-S-1-5-21-9"
        );
        assert_eq!(running_object("a", "S-1"), "Local\\a-S-1-running");
        assert_ne!(stop_object("a", "S-1"), stop_object("a", "S-2"));
        assert!(current_sid().unwrap().starts_with("S-1-"));
    }
}
