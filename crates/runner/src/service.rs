//! Native service installation and protected configuration for the runner.

pub(crate) mod supervisor;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    server: String,
    token: Option<String>,
    state_dir: PathBuf,
    service_name: String,
    user_service: bool,
}

fn regular_file(path: &Path) -> anyhow::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            anyhow::ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "服务配置必须是普通文件：{}",
                path.display()
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let uid = unsafe { libc::geteuid() };
                anyhow::ensure!(
                    (meta.uid() == uid || meta.uid() == 0) && meta.mode() & 0o022 == 0,
                    "服务文件所有者或写权限不安全：{}",
                    path.display()
                );
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

impl Settings {
    fn load(path: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(regular_file(path)?, "服务配置不存在：{}", path.display());
        let value: Self =
            serde_json::from_slice(&std::fs::read(path)?).context("无法读取服务配置")?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> anyhow::Result<()> {
        validate_name(&self.service_name)?;
        crate::ws_url(&self.server, "/api/runners/connect")?;
        anyhow::ensure!(
            !self.server.chars().any(char::is_control),
            "服务地址不能包含控制字符"
        );
        anyhow::ensure!(self.state_dir.is_absolute(), "服务状态目录必须是绝对路径");
        anyhow::ensure!(
            self.token
                .as_ref()
                .is_some_and(|token| !token.trim().is_empty())
                || crate::Enrolment::load(&self.state_dir).is_some(),
            "首次安装需要注册令牌"
        );
        Ok(())
    }

    fn store(&self, path: &Path) -> anyhow::Result<()> {
        self.validate()?;
        if regular_file(path)? {
            let old = Self::load(path)?;
            anyhow::ensure!(
                old.server.trim_end_matches('/') == self.server.trim_end_matches('/')
                    && old.state_dir == self.state_dir
                    && old.service_name == self.service_name
                    && old.user_service == self.user_service,
                "已有服务连接到其他地址或状态目录；为保护主机身份，不能通过重复安装覆盖配置"
            );
            return Ok(());
        }
        let parent = path.parent().context("服务配置目录无效")?;
        std::fs::create_dir_all(parent)?;
        // tempfile creates with 0600 on Unix; publish only a complete, synced file.
        let mut pending = tempfile::NamedTempFile::new_in(parent)?;
        pending.write_all(&serde_json::to_vec(self)?)?;
        pending.as_file().sync_all()?;
        match pending.persist_noclobber(path) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.store(path)
            }
            Err(error) => Err(error.error.into()),
        }
    }

    fn validate_rebind(&self, old: &Self) -> anyhow::Result<()> {
        self.validate()?;
        anyhow::ensure!(
            old.state_dir == self.state_dir
                && old.service_name == self.service_name
                && old.user_service == self.user_service,
            "已有服务使用其他状态目录、服务名称或服务范围，不能重新绑定"
        );
        anyhow::ensure!(self.token.is_some(), "重新绑定需要新令牌");
        Ok(())
    }

    fn rebind(&self, path: &Path) -> anyhow::Result<bool> {
        self.validate()?;
        if !regular_file(path)? {
            return Ok(false);
        }
        let old = Self::load(path)?;
        self.validate_rebind(&old)?;
        if old.token == self.token
            && old.server.trim_end_matches('/') == self.server.trim_end_matches('/')
        {
            return Ok(false);
        }
        // Explicit --rebind carries a fresh enrolment token. Never send the old
        // server's key to the new server; enrol into the host selected by that token.
        match std::fs::remove_file(crate::Enrolment::path(&self.state_dir)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut pending =
            tempfile::NamedTempFile::new_in(path.parent().context("服务配置目录无效")?)?;
        pending.write_all(&serde_json::to_vec(self)?)?;
        pending.as_file().sync_all()?;
        pending.persist(path)?;
        Ok(true)
    }
}

fn validate_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.')),
        "服务名称只允许字母、数字、横线、下划线和点，必须以字母或数字开头（最多80位）"
    );
    Ok(())
}

use clap::{Args, Parser, Subcommand};
use std::process::Command;

#[derive(Parser)]
#[command(about = "安装和管理主机代理系统服务")]
struct Cli {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Install the current runner as an automatically started native service.
    Install {
        #[command(flatten)]
        service: Identity,
        #[arg(long)]
        server: String,
        #[arg(long, env = "FERRY_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long)]
        rebind: bool,
        #[arg(long)]
        state_dir: Option<PathBuf>,
    },
    /// Verify a signed update for this runner without modifying the service.
    VerifyUpdate {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        signature: PathBuf,
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        version: String,
    },
    Start(Identity),
    Stop(Identity),
    Status(Identity),
    /// Remove the service registration, preserving host identity and data.
    Uninstall(Identity),
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        failed_version: Option<String>,
    },
    #[command(hide = true)]
    Run {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// Windows current-user installation: the supervisor under its keeper.
    #[cfg(windows)]
    #[command(hide = true)]
    Supervise {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        name: String,
    },
}
#[derive(Args)]
struct Identity {
    #[arg(long, default_value = "runner-agent")]
    name: String,
    /// Install for the current user instead of the machine (Windows logon task,
    /// systemd --user or LaunchAgent); no administrator rights are needed.
    #[arg(long)]
    user: bool,
}

pub fn dispatch() -> anyhow::Result<bool> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new("service")) {
        return Ok(false);
    }
    match Cli::parse_from(std::env::args_os().skip(1)).action {
        Action::Install {
            service,
            server,
            token,
            rebind,
            state_dir,
        } => {
            validate_name(&service.name)?;
            let state_dir = state_dir.unwrap_or(service.default_state_dir()?);
            let settings = Settings {
                server,
                token,
                state_dir,
                service_name: service.name.clone(),
                user_service: service.user,
            };
            service.install(&settings, rebind)?;
        }
        Action::VerifyUpdate {
            manifest,
            signature,
            artifact,
            version,
        } => {
            crate::update::verify_files(&manifest, &signature, &artifact, &version)?;
            println!("更新签名、版本、平台及文件完整性验证通过");
        }
        Action::Start(service) => service.control("start")?,
        Action::Stop(service) => service.control("stop")?,
        Action::Status(service) => service.control("status")?,
        Action::Uninstall(service) => service.uninstall()?,
        Action::Worker {
            config,
            name,
            failed_version,
        } => {
            let settings = Settings::load(&config)?;
            anyhow::ensure!(settings.service_name == name, "服务名称与安装配置不一致");
            let (stop, stopped) = tokio::sync::watch::channel(false);
            let (confirm, confirmation) = tokio::sync::watch::channel(false);
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::stdin().lock().lines() {
                    if matches!(line.as_deref(), Ok("confirm")) {
                        confirm.send_replace(true);
                    } else {
                        break;
                    }
                }
                stop.send_replace(true);
            });
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(crate::run_with_shutdown(
                    crate::Config {
                        server: settings.server,
                        token: settings.token,
                        state_dir: settings.state_dir,
                        managed: true,
                        confirmation: Some(confirmation),
                        failed_version,
                    },
                    stopped,
                ))?;
        }
        #[cfg(windows)]
        Action::Supervise { config, name } => {
            validate_name(&name)?;
            let settings = Settings::load(&config)?;
            anyhow::ensure!(
                settings.service_name == name && settings.user_service,
                "服务名称与安装配置不一致"
            );
            windows_user::supervise(config, name)?;
        }
        Action::Run { config, name } => {
            validate_name(&name)?;
            #[cfg(windows)]
            if Settings::load(&config)?.user_service {
                windows_user::run(config, name)?;
            } else {
                windows::run(config, name)?;
            }
            #[cfg(not(windows))]
            {
                let settings = Settings::load(&config)?;
                anyhow::ensure!(settings.service_name == name, "服务名称与安装配置不一致");
                supervisor::foreground(config)?;
            }
        }
    }
    Ok(true)
}

fn command(command: &mut Command) -> anyhow::Result<()> {
    // Arguments can include paths but never enrolment tokens or private keys.
    let status = command.status().context("无法调用系统服务管理器")?;
    anyhow::ensure!(status.success(), "系统服务命令执行失败：{status}");
    Ok(())
}

#[cfg(unix)]
fn secure_directory(root: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = unsafe { libc::geteuid() };
    let real = std::fs::canonicalize(root)?;
    // Check both lexical ancestors and symlink destinations: chmod on the leaf
    // alone cannot stop an untrusted parent owner from replacing the directory.
    for path in root.ancestors().chain(real.ancestors()) {
        let metadata = std::fs::symlink_metadata(path)?;
        anyhow::ensure!(
            metadata.uid() == uid || metadata.uid() == 0,
            "服务目录或父目录属于其他用户：{}",
            path.display()
        );
        if !metadata.file_type().is_symlink() {
            let shared_sticky_parent =
                path != root && path != real && metadata.mode() & 0o1000 != 0;
            anyhow::ensure!(
                metadata.mode() & 0o022 == 0 || shared_sticky_parent,
                "服务目录或父目录允许其他用户写入：{}",
                path.display()
            );
        }
    }
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
fn write_registration(path: &Path, content: &str) -> anyhow::Result<()> {
    if regular_file(path)? {
        anyhow::ensure!(
            std::fs::read(path)? == content.as_bytes(),
            "已有同名系统服务使用不同配置，拒绝覆盖：{}",
            path.display()
        );
        return Ok(());
    }
    let parent = path.parent().context("服务注册路径无效")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644))?;
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(())
}

impl Identity {
    fn default_state_dir(&self) -> anyhow::Result<PathBuf> {
        if self.user {
            #[cfg(windows)]
            return Ok(PathBuf::from(
                std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA未设置")?,
            )
            .join(&self.name));
            #[cfg(not(windows))]
            {
                let home = std::env::var_os("HOME").context("无法确定用户目录")?;
                return Ok(PathBuf::from(home)
                    .join(if cfg!(target_os = "macos") {
                        "Library/Application Support"
                    } else {
                        ".local/state"
                    })
                    .join(&self.name));
            }
        }
        #[cfg(windows)]
        return Ok(
            PathBuf::from(std::env::var_os("ProgramData").context("ProgramData未设置")?)
                .join(&self.name),
        );
        #[cfg(not(windows))]
        Ok(PathBuf::from("/var/lib").join(&self.name))
    }

    fn install(&self, settings: &Settings, rebind: bool) -> anyhow::Result<()> {
        validate_name(&self.name)?;
        settings.validate()?;
        let root = &settings.state_dir;
        #[cfg(unix)]
        {
            std::fs::create_dir_all(root)?;
            anyhow::ensure!(
                !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
                "服务状态目录不能是符号链接"
            );
            secure_directory(root)?;
        }
        #[cfg(windows)]
        windows::secure_directory(root, self.user)?;
        let config = root.join("service.json");
        let mut rebound = false;
        if rebind && regular_file(&config)? {
            let old = Settings::load(&config)?;
            settings.validate_rebind(&old)?;
            if old.token != settings.token
                || old.server.trim_end_matches('/') != settings.server.trim_end_matches('/')
            {
                println!("正在停止旧 Runner 服务，等待进程退出…");
                self.control("stop")?;
                rebound = settings.rebind(&config)?;
            }
        }
        settings.store(&config)?;
        let binary = root.join(if cfg!(windows) {
            "runner-service.exe"
        } else {
            "runner-service"
        });
        if rebound || !regular_file(&binary)? {
            let mut temporary = tempfile::NamedTempFile::new_in(root)?;
            std::io::copy(
                &mut std::fs::File::open(std::env::current_exe()?)?,
                temporary.as_file_mut(),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                temporary
                    .as_file()
                    .set_permissions(std::fs::Permissions::from_mode(0o700))?;
            }
            temporary.as_file().sync_all()?;
            if rebound {
                temporary.persist(&binary).map_err(|e| e.error)?;
            } else {
                temporary.persist_noclobber(&binary).map_err(|e| e.error)?;
            }
        }
        self.register(&binary, &config)?;
        println!("正在启动 Runner 服务…");
        self.control("start")
    }
}

#[cfg(target_os = "linux")]
impl Identity {
    fn unit_path(&self) -> anyhow::Result<PathBuf> {
        Ok(if self.user {
            PathBuf::from(std::env::var_os("HOME").context("无法确定用户目录")?)
                .join(".config/systemd/user")
        } else {
            PathBuf::from("/etc/systemd/system")
        }
        .join(format!("{}.service", self.name)))
    }
    fn systemctl(&self) -> Command {
        let mut cmd = Command::new("systemctl");
        if self.user {
            cmd.arg("--user");
        }
        cmd
    }
    fn register(&self, binary: &Path, config: &Path) -> anyhow::Result<()> {
        fn quote(path: &Path) -> anyhow::Result<String> {
            let text = path.to_str().context("服务路径不是UTF-8")?;
            anyhow::ensure!(!text.chars().any(char::is_control), "服务路径包含控制字符");
            Ok(format!(
                "\"{}\"",
                text.replace('\\', "\\\\")
                    .replace('\"', "\\\"")
                    .replace('%', "%%")
                    .replace('$', "$$")
            ))
        }
        let unit = format!(
            "[Unit]\nDescription=Host execution agent\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=0\n[Service]\nType=simple\nExecStart={} service run --config {} --name {}\nRestart=on-failure\nRestartSec=5\nKillMode=mixed\nTimeoutStopSec=infinity\nUMask=0077\n[Install]\nWantedBy={}\n",
            quote(binary)?,
            quote(config)?,
            self.name,
            if self.user {
                "default.target"
            } else {
                "multi-user.target"
            }
        );
        let path = self.unit_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        write_registration(&path, &unit)?;
        command(self.systemctl().arg("daemon-reload"))?;
        command(
            self.systemctl()
                .args(["enable", &format!("{}.service", self.name)]),
        )
    }
    fn control(&self, action: &str) -> anyhow::Result<()> {
        validate_name(&self.name)?;
        command(
            self.systemctl()
                .args([action, &format!("{}.service", self.name)]),
        )
    }
    fn uninstall(&self) -> anyhow::Result<()> {
        validate_name(&self.name)?;
        command(
            self.systemctl()
                .args(["disable", "--now", &format!("{}.service", self.name)]),
        )?;
        std::fs::remove_file(self.unit_path()?)?;
        command(self.systemctl().arg("daemon-reload"))
    }
}

#[cfg(target_os = "macos")]
impl Identity {
    fn domain(&self) -> String {
        if self.user {
            format!("gui/{}", unsafe { libc::geteuid() })
        } else {
            "system".into()
        }
    }
    fn plist_path(&self) -> anyhow::Result<PathBuf> {
        Ok(if self.user {
            PathBuf::from(std::env::var_os("HOME").context("无法确定用户目录")?)
                .join("Library/LaunchAgents")
        } else {
            PathBuf::from("/Library/LaunchDaemons")
        }
        .join(format!("{}.plist", self.name)))
    }
    fn register(&self, binary: &Path, config: &Path) -> anyhow::Result<()> {
        fn xml(text: &str) -> String {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\"', "&quot;")
                .replace('\'', "&apos;")
        }
        let args = [
            binary.to_str().context("服务路径不是UTF-8")?,
            "service",
            "run",
            "--config",
            config.to_str().context("配置路径不是UTF-8")?,
            "--name",
            &self.name,
        ];
        let log = config.parent().unwrap().join("service.log");
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array>{}</array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>ThrottleInterval</key><integer>5</integer><key>ExitTimeOut</key><integer>0</integer><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>",
            xml(&self.name),
            args.iter()
                .map(|arg| format!("<string>{}</string>", xml(arg)))
                .collect::<String>(),
            xml(&log.to_string_lossy()),
            xml(&log.to_string_lossy())
        );
        let path = self.plist_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        write_registration(&path, &plist)?;
        command(Command::new("plutil").arg("-lint").arg(&path))?;
        let target = format!("{}/{}", self.domain(), self.name);
        if !Command::new("launchctl")
            .args(["print", &target])
            .output()?
            .status
            .success()
        {
            command(
                Command::new("launchctl")
                    .arg("bootstrap")
                    .arg(self.domain())
                    .arg(path),
            )?;
        }
        Ok(())
    }
    fn control(&self, action: &str) -> anyhow::Result<()> {
        validate_name(&self.name)?;
        let target = format!("{}/{}", self.domain(), self.name);
        match action {
            "start" => {
                if !Command::new("launchctl")
                    .args(["print", &target])
                    .output()?
                    .status
                    .success()
                {
                    command(
                        Command::new("launchctl")
                            .arg("bootstrap")
                            .arg(self.domain())
                            .arg(self.plist_path()?),
                    )?;
                }
                command(Command::new("launchctl").args(["kickstart", &target]))
            }
            "stop" => {
                if Command::new("launchctl")
                    .args(["print", &target])
                    .output()?
                    .status
                    .success()
                {
                    command(Command::new("launchctl").arg("bootout").arg(&target))?;
                }
                Ok(())
            }
            _ => command(Command::new("launchctl").args(["print", &target])),
        }
    }
    fn uninstall(&self) -> anyhow::Result<()> {
        self.control("stop")?;
        match std::fs::remove_file(self.plist_path()?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_user;

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        tempfile::Builder::new()
            .prefix("runner-service-test-")
            .tempdir()
            .unwrap()
            .keep()
    }

    #[test]
    fn reinstall_preserves_enrolment_and_does_not_replace_pending_token() {
        let dir = scratch();
        let settings = Settings {
            server: "https://ops.example.invalid".into(),
            token: Some("first-token".into()),
            state_dir: dir.clone(),
            service_name: "fixture-agent".into(),
            user_service: true,
        };
        settings.store(&dir.join("service.json")).unwrap();
        std::fs::write(
            dir.join("runner.json"),
            br#"{"host_id":"123","key":"existing-key"}"#,
        )
        .unwrap();
        let replacement = Settings {
            token: Some("must-not-replace".into()),
            ..settings.clone()
        };
        replacement.store(&dir.join("service.json")).unwrap();
        let saved = Settings::load(&dir.join("service.json")).unwrap();
        assert_eq!(saved.token.as_deref(), Some("first-token"));
        assert_eq!(
            std::fs::read_to_string(dir.join("runner.json")).unwrap(),
            r#"{"host_id":"123","key":"existing-key"}"#
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_rebind_replaces_token_and_removes_stale_key() {
        let dir = scratch();
        let settings = Settings {
            server: "https://ops.example.invalid".into(),
            token: Some("old".into()),
            state_dir: dir.clone(),
            service_name: "fixture-agent".into(),
            user_service: true,
        };
        let path = dir.join("service.json");
        settings.store(&path).unwrap();
        std::fs::write(
            dir.join("runner.json"),
            br#"{"host_id":"123","key":"old-key"}"#,
        )
        .unwrap();
        let replacement = Settings {
            token: Some("new".into()),
            ..settings
        };
        assert!(replacement.rebind(&path).unwrap());
        assert_eq!(Settings::load(&path).unwrap().token.as_deref(), Some("new"));
        assert!(!dir.join("runner.json").exists());
        assert!(!replacement.rebind(&path).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_rebind_allows_server_move_without_reusing_old_identity() {
        let dir = scratch();
        let original = Settings {
            server: "http://127.0.0.1:5173".into(),
            token: Some("old".into()),
            state_dir: dir.clone(),
            service_name: "fixture-agent".into(),
            user_service: true,
        };
        let path = dir.join("service.json");
        original.store(&path).unwrap();
        std::fs::write(
            dir.join("runner.json"),
            br#"{"host_id":"123","key":"old-key"}"#,
        )
        .unwrap();
        let replacement = Settings {
            server: "http://127.0.0.1:4291".into(),
            token: Some("fresh-host-token".into()),
            ..original.clone()
        };
        // An ordinary reinstall remains unable to redirect an existing identity.
        assert!(replacement.store(&path).is_err());
        assert!(replacement.rebind(&path).unwrap());
        let saved = Settings::load(&path).unwrap();
        assert_eq!(saved.server, replacement.server);
        assert_eq!(saved.token, replacement.token);
        assert!(!dir.join("runner.json").exists());
        assert!(!replacement.rebind(&path).unwrap());
        for invalid in [
            Settings {
                state_dir: dir.join("other"),
                ..replacement.clone()
            },
            Settings {
                service_name: "other".into(),
                ..replacement.clone()
            },
            Settings {
                user_service: false,
                ..replacement.clone()
            },
            Settings {
                token: None,
                ..replacement.clone()
            },
        ] {
            assert!(invalid.rebind(&path).is_err());
            assert_eq!(Settings::load(&path).unwrap().server, replacement.server);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reinstall_cannot_redirect_registered_identity_to_another_server() {
        let dir = scratch();
        let settings = Settings {
            server: "https://ops.example.invalid".into(),
            token: Some("token".into()),
            state_dir: dir.clone(),
            service_name: "fixture-agent".into(),
            user_service: true,
        };
        settings.store(&dir.join("service.json")).unwrap();
        let other = Settings {
            server: "https://different.example.invalid".into(),
            ..settings
        };
        assert!(other.store(&dir.join("service.json")).is_err());
        assert_eq!(
            Settings::load(&dir.join("service.json")).unwrap().server,
            "https://ops.example.invalid"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn service_credentials_are_private_and_symlink_targets_are_not_overwritten() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = scratch();
        let settings = Settings {
            server: "https://ops.example.invalid".into(),
            token: Some("token".into()),
            state_dir: dir.clone(),
            service_name: "fixture-agent".into(),
            user_service: true,
        };
        let path = dir.join("service.json");
        settings.store(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.join("link.json");
        symlink(&path, &link).unwrap();
        assert!(settings.store(&link).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn one_state_directory_cannot_be_claimed_by_two_services() {
        let dir = scratch();
        let first = Settings {
            server: "https://ops.example.invalid".into(),
            token: Some("first".into()),
            state_dir: dir.clone(),
            service_name: "first-agent".into(),
            user_service: true,
        };
        first.store(&dir.join("service.json")).unwrap();
        let other = Settings {
            service_name: "second-agent".into(),
            ..first.clone()
        };
        assert!(other.store(&dir.join("service.json")).is_err());
        let system = Settings {
            user_service: false,
            ..first
        };
        assert!(system.store(&dir.join("service.json")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn writable_parent_cannot_replace_privileged_agent_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let child = dir.join("state");
        std::fs::create_dir(&child).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(secure_directory(&child).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        secure_directory(&child).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn service_names_cannot_escape_native_service_paths() {
        for name in ["../other", "a/b", "a b", "a\nExecStart=bad", "", "-option"] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert!(validate_name("runner-test.123").is_ok());
    }
}
