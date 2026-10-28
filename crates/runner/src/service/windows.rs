//! Native Windows SCM entry point and service registration.
use super::*;
use std::{ffi::OsString, sync::OnceLock, time::Duration};
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

impl Identity {
    fn manager(&self, create: bool) -> anyhow::Result<ServiceManager> {
        validate_name(&self.name)?;
        anyhow::ensure!(!self.user, "Windows 服务需要管理员权限，不支持 --user");
        let mut access = ServiceManagerAccess::CONNECT;
        if create {
            access |= ServiceManagerAccess::CREATE_SERVICE;
        }
        Ok(ServiceManager::local_computer(None::<&str>, access)?)
    }
    pub(super) fn register(&self, binary: &Path, config: &Path) -> anyhow::Result<()> {
        let manager = self.manager(true)?;
        let access = ServiceAccess::QUERY_CONFIG
            | ServiceAccess::QUERY_STATUS
            | ServiceAccess::CHANGE_CONFIG
            | ServiceAccess::START;
        let info = ServiceInfo {
            name: self.name.clone().into(),
            display_name: self.name.clone().into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: binary.to_owned(),
            launch_arguments: vec![
                "service".into(),
                "run".into(),
                "--config".into(),
                config.as_os_str().to_owned(),
                "--name".into(),
                self.name.clone().into(),
            ],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let service = match manager.create_service(&info, access) {
            Ok(service) => service,
            Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1073) => {
                let service = manager.open_service(&self.name, access)?;
                let installed = service.query_config()?;
                fn quote(path: &Path) -> anyhow::Result<String> {
                    let value = path.to_str().context("服务路径不是Unicode")?;
                    anyhow::ensure!(
                        !value.contains('"') && !value.chars().any(char::is_control),
                        "服务路径无效"
                    );
                    Ok(if value.chars().any(char::is_whitespace) {
                        format!("\"{value}\"")
                    } else {
                        value.to_owned()
                    })
                }
                let expected = format!(
                    "{} service run --config {} --name {}",
                    quote(binary)?,
                    quote(config)?,
                    self.name
                );
                anyhow::ensure!(
                    installed.executable_path.to_str() == Some(expected.as_str()),
                    "已有同名系统服务使用不同配置，拒绝覆盖"
                );
                if installed.start_type != ServiceStartType::AutoStart {
                    service.change_config(&info)?;
                }
                service
            }
            Err(error) => return Err(error.into()),
        };
        service.update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            }]),
        })?;
        service.set_failure_actions_on_non_crash_failures(true)?;
        Ok(())
    }
    pub(super) fn control(&self, action: &str) -> anyhow::Result<()> {
        let manager = self.manager(false)?;
        let service = manager.open_service(
            &self.name,
            ServiceAccess::QUERY_STATUS
                | match action {
                    "start" => ServiceAccess::START,
                    "stop" => ServiceAccess::STOP,
                    _ => ServiceAccess::empty(),
                },
        )?;
        let status = service.query_status()?;
        match action {
            "start" if status.current_state == ServiceState::Stopped => {
                service.start::<&str>(&[])?
            }
            "stop"
                if !matches!(
                    status.current_state,
                    ServiceState::Stopped | ServiceState::StopPending
                ) =>
            {
                service.stop()?;
            }
            "status" => println!(
                "{}: {:?}, pid={:?}",
                self.name, status.current_state, status.process_id
            ),
            _ => {}
        }
        Ok(())
    }
    pub(super) fn uninstall(&self) -> anyhow::Result<()> {
        let manager = self.manager(false)?;
        let service = match manager.open_service(
            &self.name,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS | ServiceAccess::DELETE,
        ) {
            Ok(service) => service,
            Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        if !matches!(
            service.query_status()?.current_state,
            ServiceState::Stopped | ServiceState::StopPending
        ) {
            service.stop()?;
        }
        // SCM deletes the registration after the running instance has stopped.
        service.delete()?;
        Ok(())
    }
}

static ENTRY: OnceLock<(PathBuf, String)> = OnceLock::new();
define_windows_service!(service_entry, service_main);

pub(super) fn run(config: PathBuf, name: String) -> anyhow::Result<()> {
    ENTRY
        .set((config, name.clone()))
        .map_err(|_| anyhow::anyhow!("服务入口已经初始化"))?;
    service_dispatcher::start(name, service_entry)?;
    Ok(())
}
fn service_main(_: Vec<OsString>) {
    if let Err(error) = serve() {
        if let Some((path, _)) = ENTRY.get() {
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.with_file_name("service.log"))
            {
                let _ = writeln!(log, "服务退出：{error:#}");
            }
        }
        // A nonzero process exit lets SCM's configured recovery restart us.
        std::process::exit(1);
    }
}
fn status(state: ServiceState, code: u32, checkpoint: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(code),
        checkpoint,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    }
}
fn serve() -> anyhow::Result<()> {
    let (path, name) = ENTRY.get().context("服务入口未初始化")?;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let handle = service_control_handler::register(name, move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    handle.set_service_status(status(ServiceState::StartPending, 0, 1))?;
    let settings = Settings::load(path)?;
    anyhow::ensure!(settings.service_name == *name, "服务名称与安装配置不一致");
    let cfg = crate::Config {
        server: settings.server,
        token: settings.token,
        state_dir: settings.state_dir,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    handle.set_service_status(status(ServiceState::Running, 0, 0))?;
    let result = runtime.block_on(async {
        let execution = crate::run_with_shutdown(cfg, stopped.clone());
        tokio::pin!(execution);
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut checkpoint = 0;
        loop {
            tokio::select! {
                result=&mut execution=>break result,
                _=tick.tick()=>{
                    if *stopped.borrow() {
                        checkpoint+=1;
                        handle.set_service_status(status(ServiceState::StopPending,0,checkpoint))?;
                    }
                }
            }
        }
    });
    handle.set_service_status(status(
        ServiceState::Stopped,
        if result.is_ok() { 0 } else { 1 },
        0,
    ))?;
    result
}

pub(super) fn secure_directory(root: &Path) -> anyhow::Result<()> {
    // Paths are data in the child environment, never interpolated PowerShell.
    command(
        Command::new("powershell.exe")
            // A caller running PowerShell 7 exports its own module search path.
            // Windows PowerShell 5 must resolve its matching Security module
            // (Get-Acl/Set-Acl), not attempt to load PowerShell 7 assemblies.
            .env_remove("PSModulePath")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("state-acl.ps1"),
            ])
            .env("RUNNER_SERVICE_STATE_DIRECTORY", root),
    )
}
