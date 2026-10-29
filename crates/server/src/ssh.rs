//! SSH 接入：用 russh / russh-sftp 实现 `HostChannel`。
//!
//! 一次 HTTP 请求对应一条连接——`remote.rs` 的每个处理函数都自己 `open` 一次
//! 通道——所以这里不做连接池，只负责在一条已认证的连接上按需开 SSH channel：
//! 执行命令一条、终端一条、每次文件操作一条 SFTP 子系统。
//!
//! 需要小心的只有 `read_file`：`remote::download` 拿到字节流之后就把通道丢了，
//! 而 `client::Handle` 一 drop 整条 SSH 连接就断开，所以返回的流必须自己持有
//! 连接，否则下载会在第一个数据块之后断掉。

use crate::snowflake::Id;

use crate::{
    channel::{ByteStream, DirEntry, FileMeta, HostChannel, Output, Pty, PtySize},
    entities::hosts,
    error::{ApiError, Result},
    remote::Host,
    state::AppState,
};
use axum::http::StatusCode;
use bytes::Bytes;
use ferry_protocol::{DiskUsage, HostMonitor, NetworkUsage};
use futures::StreamExt;
use russh::{
    Channel, ChannelMsg,
    client::{self, Handle},
    keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpError},
    protocol::{FileAttributes, FileType, OpenFlags, StatusCode as SftpStatus},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// TCP 握不上手就不必等 SSH 协议超时——操作员填错地址或端口是最常见的情况。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 连接空着不动时的探活间隔。默认 `keepalive_max` 是 3，所以大约 90 秒内能
/// 发现一条已经死掉的连接，终端不会一直挂在那里。
const KEEPALIVE: Duration = Duration::from_secs(30);
/// 一次性命令的输出要留在内存里，所以必须有上限：一条跑飞的命令
/// （`cat /dev/urandom`）不该把服务撑爆。
const MAX_CAPTURE: usize = 1 << 20;
/// 文件流每块的大小。SFTP 默认单包 256 KiB，取同一个量级，块太小会被
/// 往返次数拖慢。
const CHUNK: usize = 256 * 1024;

// ------------------------------------------------------------------ 凭据

/// 登录主机用的秘密。用户名、地址、端口来自 `hosts` 行，只有秘密单独传。
pub enum Credential {
    Password(String),
    Key {
        /// OpenSSH 格式的私钥内容。
        pem: String,
        passphrase: Option<String>,
    },
}

impl Credential {
    /// Loads the credential a host references, decrypting it at the moment of
    /// use. The plaintext lives only for the length of this connection
    /// attempt: it is never logged, never returned over HTTP, and never
    /// written anywhere.
    pub async fn load(s: &AppState, workspace_id: &Id, credential_id: &Id) -> Result<Self> {
        if *credential_id == Id::from(0) {
            return Err(ApiError::bad(
                "这台主机还没有选择登录凭据，请在「授权认证」里新建一条，再到主机上选用。",
            ));
        }
        let (kind, _username, secret) =
            crate::credentials::unseal(s, workspace_id, credential_id).await?;
        match kind.as_str() {
            "password" => Ok(Self::Password(secret)),
            "ssh-key" => Ok(Self::Key {
                pem: secret,
                passphrase: None,
            }),
            other => Err(ApiError::bad(format!(
                "凭据类型「{other}」不能用于 SSH 登录，请改用密码或 SSH 私钥。"
            ))),
        }
    }
}

// ------------------------------------------------------------- 主机密钥 TOFU

/// 首次信任（TOFU）的判断，单独拎出来是因为这是这个文件里唯一的安全属性：
/// 没有记录就记下，和记录一致就放行，变了就拒。没有「永远接受」这一档。
fn tofu(expected: &str, actual: &str) -> std::result::Result<(), Handshake> {
    if expected.is_empty() || expected == actual {
        return Ok(());
    }
    Err(Handshake::HostKeyChanged {
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    })
}

/// 握手期间的失败。单独一个类型，是为了把「主机密钥变了」和普通的协议错误
/// 分开——前者要让操作员看到两个指纹并人工确认，后者只是连不上。
#[derive(Debug)]
enum Handshake {
    Ssh(russh::Error),
    HostKeyChanged { expected: String, actual: String },
}

impl From<russh::Error> for Handshake {
    fn from(error: russh::Error) -> Self {
        Self::Ssh(error)
    }
}

/// 校验主机密钥，并把看到的指纹留给调用方。
struct Tofu {
    expected: String,
    /// 握手在后台任务里跑，`Handle` 不把 handler 交回来，所以指纹只能靠
    /// 共享状态带出去。
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Tofu {
    type Error = Handshake;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let actual = server_public_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        tofu(&self.expected, &actual)?;
        *self.seen.lock().expect("fingerprint lock") = Some(actual);
        Ok(true)
    }
}

// ------------------------------------------------------------------ 通道

/// 工厂入口，和 `runner::open` 同形，所以 `channel_open` 里是一行。
///
/// 首次连接到的指纹在这里落库：若等调用方去写，忘了写就等于把 TOFU 降级成
/// 「永远接受」，所以这一步和建立连接绑在一起，写不成功就不给连接。
pub async fn open(s: &AppState, host: &Host) -> Result<Box<dyn HostChannel>> {
    let credential = Credential::load(s, &host.workspace_id, &host.credential_id).await?;
    let target = hosts::Entity::find_by_id(i64::from(host.id))
        .filter(hosts::Column::WorkspaceId.eq(host.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let channel = if target.jump_host_id == 0 {
        SshChannel::open(host, credential).await?
    } else {
        let jump = hosts::Entity::find_by_id(target.jump_host_id)
            .filter(hosts::Column::WorkspaceId.eq(host.workspace_id))
            .filter(hosts::Column::Connection.eq("ssh"))
            .filter(hosts::Column::JumpHostId.eq(0))
            .one(&s.db)
            .await?
            .ok_or_else(|| ApiError::bad("跳板主机不存在或不再可直连。"))?;
        let jump_host = Host {
            id: Id::from(jump.id),
            workspace_id: host.workspace_id,
            credential_id: Id::from(jump.credential_id),
            name: jump.name,
            address: jump.address,
            port: jump.port as u16,
            username: jump.username,
            connection: jump.connection,
            host_key: jump.host_key,
        };
        let jump_credential =
            Credential::load(s, &host.workspace_id, &jump_host.credential_id).await?;
        let jump_channel = SshChannel::open(&jump_host, jump_credential).await?;
        record_fingerprint(s, &jump_host, &jump_channel).await?;
        let stream = jump_channel
            .session
            .channel_open_direct_tcpip(host.address.clone(), u32::from(host.port), "127.0.0.1", 0)
            .await
            .map_err(|error| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!(
                        "跳板主机无法连接目标 {}:{}：{error}",
                        host.address, host.port
                    ),
                )
            })?
            .into_stream();
        SshChannel::connect_stream(host, credential, stream, Some(jump_channel.session.clone()))
            .await?
    };
    record_fingerprint(s, host, &channel).await?;
    Ok(Box::new(channel))
}

async fn record_fingerprint(s: &AppState, host: &Host, channel: &SshChannel) -> Result<()> {
    if let Some(fingerprint) = &channel.learned_host_key {
        // 只在仍然为空时写：两个并发的首次连接不会互相覆盖。也不动 version，
        // 这不是操作员的编辑，不该让别人正在填的表单失效。
        hosts::Entity::update_many()
            .col_expr(hosts::Column::HostKey, Expr::value(fingerprint.clone()))
            .filter(hosts::Column::Id.eq(host.id))
            .filter(hosts::Column::HostKey.eq(""))
            .exec(&s.db)
            .await?;
    }
    Ok(())
}

pub struct SshChannel {
    /// `Arc` 是为了让 `read_file` 返回的流能把连接一起带走。
    session: Arc<Handle<Tofu>>,
    jump: Option<Arc<Handle<Tofu>>>,
    /// 只有首次连接（`hosts.host_key` 为空）才有值：调用方要把它写回
    /// `hosts.host_key`，之后的连接就靠它防中间人。这里不碰数据库。
    pub learned_host_key: Option<String>,
}

impl SshChannel {
    pub async fn open(host: &Host, credential: Credential) -> Result<Self> {
        let address = format!("{}:{}", host.address, host.port);
        // 自己做 TCP 连接而不是用 `client::connect`：这样「连不上」和
        // 「认证不过」从一开始就是两条路径，操作员拿到的提示才有区别。
        let stream =
            tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(&address))
                .await
                .map_err(|_| unreachable(&address, "连接超时"))?
                .map_err(|error| unreachable(&address, &error.to_string()))?;

        Self::connect_stream(host, credential, stream, None).await
    }

    async fn connect_stream<S>(
        host: &Host,
        credential: Credential,
        stream: S,
        jump: Option<Arc<Handle<Tofu>>>,
    ) -> Result<Self>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let address = format!("{}:{}", host.address, host.port);
        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE),
            ..Default::default()
        });
        let seen = Arc::new(Mutex::new(None));
        let mut session = client::connect_stream(
            config,
            stream,
            Tofu {
                expected: host.host_key.clone(),
                seen: seen.clone(),
            },
        )
        .await
        .map_err(|error| match error {
            Handshake::HostKeyChanged { expected, actual } => ApiError(
                StatusCode::CONFLICT,
                format!(
                    "主机 {address} 的密钥指纹已改变（记录为 {expected}，实际为 {actual}），\
                     为防止中间人攻击已拒绝连接。若确认该主机重装过系统或更换过机器，\
                     请先清空主机记录里的指纹再重新连接。"
                ),
            ),
            Handshake::Ssh(error) => ApiError(
                StatusCode::BAD_GATEWAY,
                format!("与 {address} 的 SSH 握手失败：{error}。请确认该端口上运行的是 SSH 服务。"),
            ),
        })?;

        let result = match &credential {
            Credential::Password(password) => {
                session
                    .authenticate_password(&host.username, password)
                    .await
            }
            Credential::Key { pem, passphrase } => {
                let key = match passphrase {
                    Some(phrase) => PrivateKey::from_openssh(pem).and_then(|k| k.decrypt(phrase)),
                    None => PrivateKey::from_openssh(pem),
                }
                .map_err(|error| {
                    ApiError::bad(format!(
                        "私钥无法解析（{error}），请确认是 OpenSSH 格式且口令正确。"
                    ))
                })?;
                // RSA 私钥要挑签名哈希，不挑就落回 SHA-1，而现在多数 sshd
                // 已经不接受 ssh-rsa 了。非 RSA 的密钥这里会被忽略。
                let hash = session
                    .best_supported_rsa_hash()
                    .await
                    .ok()
                    .flatten()
                    .flatten();
                session
                    .authenticate_publickey(
                        &host.username,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
            }
        }
        .map_err(|error| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("向 {address} 提交登录凭据时连接中断：{error}。"),
            )
        })?;
        if !result.success() {
            return Err(ApiError(
                StatusCode::BAD_GATEWAY,
                format!(
                    "主机 {address} 拒绝了账号 {} 的登录凭据，请检查用户名、密码或私钥。",
                    host.username
                ),
            ));
        }

        // 走到这里 `check_server_key` 一定跑过了，所以 `seen` 必有值。
        let fingerprint = seen.lock().expect("fingerprint lock").clone();
        Ok(Self {
            session: Arc::new(session),
            jump,
            learned_host_key: host.host_key.is_empty().then_some(fingerprint).flatten(),
        })
    }

    async fn session_channel(&self) -> Result<Channel<client::Msg>> {
        self.session.channel_open_session().await.map_err(|error| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("无法在该连接上打开会话：{error}。"),
            )
        })
    }

    /// 每次文件操作开一条新的 SFTP 子系统。一次请求只做一件事，省下的那点
    /// 往返不值得为此维护一个会话缓存。
    async fn sftp(&self) -> Result<SftpSession> {
        let channel = self.session_channel().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!("无法请求 SFTP 子系统：{error}。"),
                )
            })?;
        SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!("主机未能启动 SFTP 子系统（{error}），请确认 sshd 已启用 sftp。"),
                )
            })
    }
}

fn parse_linux_monitor(output: &str) -> Result<HostMonitor> {
    let mut cpu = Vec::new();
    let mut memory = None;
    let mut load1 = 0.0;
    let mut disks = Vec::new();
    let mut networks = Vec::new();
    for line in output.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["cpu", values @ ..] if cpu.len() < 2 && values.len() >= 5 => {
                let values: Vec<u64> = values
                    .iter()
                    .map(|v| v.parse())
                    .collect::<std::result::Result<_, _>>()
                    .map_err(|_| ApiError::bad("无法读取主机 CPU 数据。"))?;
                cpu.push((
                    values.iter().sum::<u64>(),
                    values[3] + values.get(4).copied().unwrap_or(0),
                ));
            }
            ["mem", used, total] => memory = Some((used.parse::<u64>(), total.parse::<u64>())),
            ["load", value] => load1 = value.parse().unwrap_or(0.0),
            ["disk", name, mount, total, used] => {
                if let (Ok(total), Ok(used)) = (total.parse(), used.parse()) {
                    disks.push(DiskUsage {
                        name: (*name).into(),
                        mount: (*mount).into(),
                        total,
                        used,
                    });
                }
            }
            ["net", name, received, transmitted] => {
                if let (Ok(received), Ok(transmitted)) = (received.parse(), transmitted.parse()) {
                    networks.push(NetworkUsage {
                        name: (*name).into(),
                        received,
                        transmitted,
                    });
                }
            }
            _ => {}
        }
    }
    let (Some((Ok(memory_used), Ok(memory_total))), [first, second]) = (memory, cpu.as_slice())
    else {
        return Err(ApiError::bad("无法读取主机 CPU 或内存数据。"));
    };
    let elapsed = second.0.saturating_sub(first.0);
    let idle = second.1.saturating_sub(first.1);
    Ok(HostMonitor {
        cpu_percent: if elapsed == 0 {
            0.0
        } else {
            100.0 * (elapsed.saturating_sub(idle)) as f64 / elapsed as f64
        },
        load1,
        memory_used,
        memory_total,
        disks,
        networks,
    })
}

#[async_trait::async_trait]
impl HostChannel for SshChannel {
    async fn monitor(&self) -> Result<HostMonitor> {
        const COMMAND: &str = "head -n 1 /proc/stat; sleep 1; head -n 1 /proc/stat; awk '{print \"load\", $1}' /proc/loadavg; awk '/^MemTotal:/ {total=$2} /^MemAvailable:/ {available=$2} END {printf \"mem %.0f %.0f\\n\", (total-available)*1024, total*1024}' /proc/meminfo; df -P -B1 | awk 'NR>1 && $1 ~ /^\\/dev\\// {printf \"disk %s %s %s %s\\n\", $1, $6, $2, $3}'; for item in /sys/class/net/*; do test -e \"$item/device\" || continue; name=${item##*/}; printf 'net %s %s %s\\n' \"$name\" \"$(cat \"$item/statistics/rx_bytes\")\" \"$(cat \"$item/statistics/tx_bytes\")\"; done";
        let output = self.exec(COMMAND, Duration::from_secs(10)).await?;
        if output.code == 0 {
            if let Ok(metrics) = parse_linux_monitor(&output.stdout) {
                return Ok(metrics);
            }
        }
        const WINDOWS: &str = "powershell.exe -NoProfile -Command \"$cpu=(Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average; $os=Get-CimInstance Win32_OperatingSystem; $disks=@(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object { @{name=$_.DeviceID;mount=$_.DeviceID;total=[uint64]$_.Size;used=[uint64]($_.Size-$_.FreeSpace)} }); $networks=@(Get-NetAdapter -Physical | Get-NetAdapterStatistics | ForEach-Object { @{name=$_.Name;received=[uint64]$_.ReceivedBytes;transmitted=[uint64]$_.SentBytes} }); @{cpu_percent=[double]$cpu;load1=0.0;memory_used=[uint64](($os.TotalVisibleMemorySize-$os.FreePhysicalMemory)*1024);memory_total=[uint64]($os.TotalVisibleMemorySize*1024);disks=$disks;networks=$networks} | ConvertTo-Json -Compress -Depth 4\"";
        let output = self.exec(WINDOWS, Duration::from_secs(10)).await?;
        if output.code != 0 {
            return Err(ApiError::bad(
                "主机监控采集失败，请检查 Linux /proc 或 Windows PowerShell 是否可用。",
            ));
        }
        serde_json::from_str(&output.stdout)
            .map_err(|_| ApiError::bad("无法解析 Windows 主机监控数据。"))
    }

    async fn exec(&self, command: &str, timeout: Duration) -> Result<Output> {
        let mut channel = self.session_channel().await?;
        channel.exec(true, command).await.map_err(|error| {
            ApiError(StatusCode::BAD_GATEWAY, format!("下发命令失败：{error}。"))
        })?;
        match tokio::time::timeout(timeout, capture(&mut channel)).await {
            Ok(output) => output,
            Err(_) => {
                // 远端进程还在跑，关掉 channel 让 sshd 回收它。
                let _ = channel.close().await;
                Err(ApiError(
                    StatusCode::GATEWAY_TIMEOUT,
                    format!("命令在 {} 秒内没有结束，已中断。", timeout.as_secs()),
                ))
            }
        }
    }

    async fn exec_stream(
        &self,
        command: &str,
        timeout: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        let mut channel = self.session_channel().await?;
        channel
            .exec(true, command)
            .await
            .map_err(|_| ApiError::bad("下发流水线命令失败。"))?;
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        let mut stopping = None;
        let mut code = None;
        loop {
            if *cancel.borrow() && stopping.is_none() {
                let _ = channel.signal(russh::Sig::TERM).await;
                stopping = Some(-130);
                deadline
                    .as_mut()
                    .reset(tokio::time::Instant::now() + Duration::from_secs(70));
            }
            tokio::select! {
                _ = cancel.changed(), if stopping.is_none() => {
                    let _ = channel.signal(russh::Sig::TERM).await;
                    stopping = Some(-130);
                    deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(70));
                }
                _ = &mut deadline => {
                    if let Some(reason) = stopping {
                        let _ = channel.signal(russh::Sig::KILL).await;
                        let _ = channel.close().await;
                        return Ok(reason);
                    }
                    let _ = channel.signal(russh::Sig::TERM).await;
                    stopping = Some(-124);
                    deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(70));
                }
                message = channel.wait() => {
                    let chunk = match message {
                        Some(ChannelMsg::Data {data}) => Some(("stdout".into(), data.to_vec())),
                        Some(ChannelMsg::ExtendedData {data, ..}) => Some(("stderr".into(), data.to_vec())),
                        Some(ChannelMsg::ExitStatus {exit_status}) => { code = Some(exit_status as i32); None }
                        Some(ChannelMsg::ExitSignal {..}) => { code = Some(-1); None }
                        None => return Ok(stopping.or(code).unwrap_or(-1)),
                        _ => None,
                    };
                    if let Some(chunk) = chunk && stopping.is_none() {
                        let reason = tokio::select! {
                            sent = output.send(chunk) => if sent.is_err() {Some(-130)} else {None},
                            _ = cancel.changed() => Some(-130),
                            _ = &mut deadline => Some(-124),
                        };
                        if let Some(reason) = reason {
                            let _ = channel.signal(russh::Sig::TERM).await;
                            stopping = Some(reason);
                            deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(70));
                        }
                    }
                }
            }
        }
    }

    async fn open_pty(&self, size: PtySize) -> Result<Box<dyn Pty>> {
        let size = size.sane();
        let channel = self.session_channel().await?;
        channel
            .request_pty(
                true,
                "xterm-256color",
                size.cols as u32,
                size.rows as u32,
                0,
                0,
                &[],
            )
            .await
            .map_err(|error| {
                ApiError(StatusCode::BAD_GATEWAY, format!("申请终端失败：{error}。"))
            })?;
        channel.request_shell(true).await.map_err(|error| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("启动远端 shell 失败：{error}。"),
            )
        })?;
        Ok(Box::new(SshPty {
            channel,
            _session: self.session.clone(),
            _jump: self.jump.clone(),
        }))
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let sftp = self.sftp().await?;
        let mut entries: Vec<DirEntry> = sftp
            .read_dir(path)
            .await
            .map_err(|error| sftp_failed(path, error))?
            .map(|entry| {
                let meta = entry.metadata();
                DirEntry {
                    name: entry.file_name(),
                    kind: kind_of(entry.file_type()),
                    size: meta.size.unwrap_or(0),
                    // 只留权限位：类型位已经在 kind 里，操作员看的是 644 这种数字。
                    mode: meta.permissions.unwrap_or(0) & 0o7777,
                    modified_at: meta.mtime.map_or(0, |time| time as i64 * 1000),
                }
            })
            .collect();
        // SFTP 不保证顺序，而界面是照着数组渲染的，所以在这里排：目录在前，
        // 同类按名字，和本地文件管理器的习惯一致。
        entries.sort_by(|a, b| {
            (a.kind != "directory", a.name.to_lowercase())
                .cmp(&(b.kind != "directory", b.name.to_lowercase()))
        });
        Ok(entries)
    }

    async fn create_file(&self, path: &str) -> Result<()> {
        let sftp = self.sftp().await?;
        let file = sftp
            .open_with_flags_and_attributes(
                path,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                FileAttributes {
                    permissions: Some(0o644),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| sftp_failed(path, error))?;
        file.close().await.map_err(ApiError::from)
    }

    async fn create_dir(&self, path: &str) -> Result<()> {
        self.sftp()
            .await?
            .create_dir(path)
            .await
            .map_err(|error| sftp_failed(path, error))
    }

    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)> {
        let sftp = self.sftp().await?;
        let meta = sftp
            .metadata(path)
            .await
            .map_err(|error| sftp_failed(path, error))?;
        if meta.file_type().is_dir() {
            return Err(ApiError::bad("这是一个目录，不能直接下载。"));
        }
        let file = sftp
            .open(path)
            .await
            .map_err(|error| sftp_failed(path, error))?;

        // 流自带连接：`remote::download` 一拿到流就把通道丢了，而 `Handle`
        // 一 drop 整条 SSH 连接就断，所以 session 和 sftp 会话都搬进状态里。
        // 每次只持有一块，所以 2 GB 的文件也只占一块的内存。
        let stream = futures::stream::try_unfold(
            (file, sftp, self.session.clone(), self.jump.clone()),
            |(mut file, sftp, session, jump)| async move {
                let mut buffer = vec![0u8; CHUNK];
                let filled = file.read(&mut buffer).await?;
                if filled == 0 {
                    return Ok(None);
                }
                buffer.truncate(filled);
                Ok(Some((Bytes::from(buffer), (file, sftp, session, jump))))
            },
        );
        Ok((
            FileMeta {
                size: meta.size.unwrap_or(0),
                mode: meta.permissions.unwrap_or(0) & 0o7777,
                modified_at: meta.mtime.map_or(0, |time| time as i64 * 1000),
            },
            Box::pin(stream),
        ))
    }

    async fn write_file(&self, path: &str, mode: u32, mut body: ByteStream) -> Result<()> {
        let sftp = self.sftp().await?;
        let mut file = sftp
            .open_with_flags_and_attributes(
                path,
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                FileAttributes {
                    permissions: Some(mode & 0o7777),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| sftp_failed(path, error))?;
        while let Some(chunk) = body.next().await {
            file.write_all(&chunk?).await?;
        }
        // `close` 会等所有在途的写回执，少了它就无法区分「写完了」和
        // 「前面某一块其实失败了」。
        file.close().await?;
        Ok(())
    }

    async fn delete_file(&self, path: &str) -> Result<()> {
        let sftp = self.sftp().await?;
        let meta = sftp
            .metadata(path)
            .await
            .map_err(|error| sftp_failed(path, error))?;
        // 删目录要走 RMDIR，否则 sshd 只回一个「失败」，操作员无从判断。
        if meta.file_type().is_dir() {
            sftp.remove_dir(path).await
        } else {
            sftp.remove_file(path).await
        }
        .map_err(|error| sftp_failed(path, error))
    }
}

struct SshPty {
    channel: Channel<client::Msg>,
    /// 终端活着的时候连接不能断。
    _session: Arc<Handle<Tofu>>,
    _jump: Option<Arc<Handle<Tofu>>>,
}

#[async_trait::async_trait]
impl Pty for SshPty {
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.channel
            .data_bytes(Bytes::copy_from_slice(data))
            .await
            .map_err(|error| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!("向终端写入失败：{error}。"),
                )
            })
    }

    async fn read(&mut self) -> Result<Option<Bytes>> {
        while let Some(message) = self.channel.wait().await {
            match message {
                // 带 pty 时 stderr 本来就并进 Data，但 sshd 在 pty 之外也可能
                // 发 ExtendedData，终端上两者都该显示。
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    return Ok(Some(data));
                }
                ChannelMsg::Eof | ChannelMsg::Close => return Ok(None),
                _ => {}
            }
        }
        Ok(None)
    }

    async fn resize(&mut self, size: PtySize) -> Result<()> {
        let size = size.sane();
        self.channel
            .window_change(size.cols as u32, size.rows as u32, 0, 0)
            .await
            .map_err(|error| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!("调整终端大小失败：{error}。"),
                )
            })
    }
}

// ------------------------------------------------------------------ 辅助

/// 收完一条命令的输出。读到 channel 关闭为止，而不是读到 ExitStatus 就停：
/// 退出状态可能先到，后面还跟着最后一批输出。
async fn capture(channel: &mut Channel<client::Msg>) -> Result<Output> {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let mut code = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => append(&mut stdout, &data),
            ChannelMsg::ExtendedData { data, .. } => append(&mut stderr, &data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status as i32),
            ChannelMsg::ExitSignal {
                signal_name,
                error_message,
                ..
            } => append(
                &mut stderr,
                format!("\n进程被信号 {signal_name:?} 终止。{error_message}").as_bytes(),
            ),
            _ => {}
        }
    }
    Ok(Output {
        // 没有退出状态说明进程是被信号打断或连接断了，不能当成成功。
        code: code.unwrap_or(-1),
        stdout: text(stdout),
        stderr: text(stderr),
    })
}

fn append(buffer: &mut Vec<u8>, data: &[u8]) {
    let room = MAX_CAPTURE.saturating_sub(buffer.len());
    buffer.extend_from_slice(&data[..room.min(data.len())]);
}

fn text(buffer: Vec<u8>) -> String {
    let truncated = buffer.len() >= MAX_CAPTURE;
    let mut out = String::from_utf8_lossy(&buffer).into_owned();
    if truncated {
        out.push_str("\n……输出超过 1 MiB，其余已丢弃。");
    }
    out
}

fn kind_of(file_type: FileType) -> String {
    match file_type {
        FileType::Dir => "directory",
        FileType::Symlink => "symlink",
        _ => "file",
    }
    .to_owned()
}

fn unreachable(address: &str, detail: &str) -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        format!("无法连接到 {address}（{detail}），请检查网络、地址和端口。"),
    )
}

/// SFTP 的失败绝大多数是「路径不存在」和「权限不够」，这两条操作员自己能处理，
/// 所以单独报出来，其余才落到通用提示。
fn sftp_failed(path: &str, error: SftpError) -> ApiError {
    match &error {
        SftpError::Status(status) if status.status_code == SftpStatus::NoSuchFile => {
            ApiError(StatusCode::NOT_FOUND, format!("路径 {path} 不存在。"))
        }
        SftpError::Status(status) if status.status_code == SftpStatus::PermissionDenied => {
            ApiError(
                StatusCode::FORBIDDEN,
                format!("没有权限访问 {path}，请检查登录账号在该路径上的权限。"),
            )
        }
        _ => ApiError(
            StatusCode::BAD_GATEWAY,
            format!("对 {path} 的文件操作失败：{error}。"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_monitor_reads_cpu_memory_disks_and_nics() {
        let output = "cpu 100 0 40 860 0\ncpu 130 0 50 920 0\nmem 1073741824 2147483648\ndisk /dev/sda1 / 100000 25000\nnet eth0 1234 5678\n";
        let sample = parse_linux_monitor(output).unwrap();
        assert!((sample.cpu_percent - 40.0).abs() < 0.01);
        assert_eq!(sample.memory_used, 1_073_741_824);
        assert_eq!(sample.disks[0].mount, "/");
        assert_eq!(sample.networks[0].received, 1234);
        assert!(parse_linux_monitor("cpu 1 2 3 4\n").is_err());
    }

    const FINGERPRINT: &str = "SHA256:7xEJ/p1S2KLvQHGC3kqKtJHQ1t6Dh6Y9DVKxwKk0mXo";

    #[test]
    fn a_host_without_a_recorded_key_is_trusted_once() {
        assert!(tofu("", FINGERPRINT).is_ok());
    }

    #[test]
    fn the_same_key_keeps_connecting() {
        assert!(tofu(FINGERPRINT, FINGERPRINT).is_ok());
    }

    #[test]
    fn a_changed_key_is_refused_and_says_both_fingerprints() {
        let other = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        match tofu(FINGERPRINT, other) {
            Err(Handshake::HostKeyChanged { expected, actual }) => {
                assert_eq!(expected, FINGERPRINT);
                assert_eq!(actual, other);
            }
            other => panic!("期望拒绝，实际 {other:?}"),
        }
    }

    #[test]
    fn a_key_that_differs_only_in_case_is_still_a_different_key() {
        // 指纹是 base64，大小写有意义；不能图省事做不区分大小写的比较。
        assert!(tofu(FINGERPRINT, &FINGERPRINT.to_lowercase()).is_err());
    }

    #[test]
    fn captured_output_is_capped() {
        let mut buffer = Vec::new();
        for _ in 0..40 {
            append(&mut buffer, &vec![b'x'; 64 * 1024]);
        }
        assert_eq!(buffer.len(), MAX_CAPTURE);
        assert!(text(buffer).ends_with("其余已丢弃。"));
    }

    // ---------------------------------------------------------- 真机测试
    //
    // 下面的用例要一台真的 sshd，由 `scripts/test-ssh.sh` 起一个容器并带上
    // 环境变量跑；没有环境变量时整组跳过，所以 `cargo test` 不会因为没装
    // Docker 而失败。
    //
    //   scripts/test-ssh.sh

    fn live() -> Option<(Host, String)> {
        let address = std::env::var("FERRY_TEST_SSH_ADDR").ok()?;
        let (address, port) = address.rsplit_once(':')?;
        Some((
            Host {
                id: 1.into(),
                workspace_id: 1.into(),
                credential_id: Id::default(),
                name: "live".into(),
                address: address.into(),
                port: port.parse().ok()?,
                username: std::env::var("FERRY_TEST_SSH_USER").unwrap_or_else(|_| "ferry".into()),
                connection: "ssh".into(),
                host_key: String::new(),
            },
            std::env::var("FERRY_TEST_SSH_PASSWORD").unwrap_or_else(|_| "ferry-test".into()),
        ))
    }

    /// 本进程的常驻内存（KiB）。用来证明大文件是流过去的而不是攒在内存里。
    fn rss_kib() -> u64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .expect("rss")
    }

    /// A missing sshd is a skip on a laptop and a failure in CI. Without this,
    /// `cargo test` reports these six as passing when they did nothing at all,
    /// which is how the whole file silently stopped being covered once.
    fn live_or_fail() -> Option<(Host, String)> {
        match live() {
            Some(found) => Some(found),
            None if std::env::var("FERRY_REQUIRE_LIVE").is_ok() => {
                panic!(
                    "FERRY_REQUIRE_LIVE is set but FERRY_TEST_SSH_ADDR is not; no sshd to test against"
                )
            }
            None => None,
        }
    }

    async fn connect() -> SshChannel {
        let (host, password) = live().expect("FERRY_TEST_SSH_ADDR");
        SshChannel::open(&host, Credential::Password(password))
            .await
            .expect("连接真机")
    }

    #[tokio::test]
    async fn live_jump_host_tunnels_ssh_and_keeps_download_alive() {
        let Some((jump_host, password)) = live_or_fail() else {
            return;
        };
        let jump = SshChannel::open(&jump_host, Credential::Password(password.clone()))
            .await
            .unwrap();
        let target = Host {
            address: "127.0.0.1".into(),
            port: 22,
            ..jump_host
        };
        let stream = jump
            .session
            .channel_open_direct_tcpip("127.0.0.1", 22, "127.0.0.1", 0)
            .await
            .unwrap()
            .into_stream();
        let target = SshChannel::connect_stream(
            &target,
            Credential::Password(password),
            stream,
            Some(jump.session.clone()),
        )
        .await
        .unwrap();
        let output = target
            .exec("printf jump-ok", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(output.stdout, "jump-ok");
        target
            .exec(
                "printf jump-file > /tmp/ferry-jump-test.txt",
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let (_, mut download) = target.read_file("/tmp/ferry-jump-test.txt").await.unwrap();
        drop(target);
        drop(jump);
        let mut bytes = Vec::new();
        while let Some(chunk) = download.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(bytes, b"jump-file");
    }

    #[tokio::test]
    async fn live_exec_reports_output_and_failure_codes() {
        if live_or_fail().is_none() {
            return;
        }
        let ssh = connect().await;
        let learned = ssh.learned_host_key.as_deref().unwrap();
        assert!(learned.starts_with("SHA256:"));
        // 脚本用 ssh-keygen 独立算了一遍同一把密钥的指纹，两边必须一致，
        // 否则 TOFU 比的就不是 OpenSSH 认的那个指纹。
        if let Ok(expected) = std::env::var("FERRY_TEST_SSH_EXPECT_KEY") {
            assert_eq!(learned, expected);
        }

        let ok = ssh
            .exec("echo ferry-ok", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(ok.code, 0);
        assert_eq!(ok.stdout.trim(), "ferry-ok");

        let bad = ssh
            .exec("echo to-stderr >&2; exit 3", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(bad.code, 3);
        assert_eq!(bad.stderr.trim(), "to-stderr");

        let slow = ssh.exec("sleep 5", Duration::from_secs(1)).await;
        assert_eq!(slow.unwrap_err().0, StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn live_streaming_logs_cancel_descendants_and_enforce_timeout() {
        if live_or_fail().is_none() {
            return;
        }
        let ssh = connect().await;
        let (output, mut chunks) = tokio::sync::mpsc::channel(8);
        let (stop, cancelled) = tokio::sync::watch::channel(false);
        let execution = ssh.exec_stream(
            "sleep 120 & printf '%s\\n' \"$!\"; wait",
            Duration::from_secs(30),
            output,
            cancelled,
        );
        tokio::pin!(execution);
        let (stream, data) = tokio::select! {
            result = &mut execution => panic!("command finished before live log: {result:?}"),
            chunk = chunks.recv() => chunk.unwrap(),
        };
        assert_eq!(stream, "stdout");
        let pid = String::from_utf8(data).unwrap().trim().to_owned();
        stop.send(true).unwrap();
        assert_ne!(
            tokio::time::timeout(Duration::from_secs(12), execution)
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let alive = ssh
            .exec(
                &format!("kill -0 {pid} 2>/dev/null"),
                Duration::from_secs(3),
            )
            .await
            .unwrap();
        assert_ne!(alive.code, 0, "cancel must stop the descendant");
        let (output, _chunks) = tokio::sync::mpsc::channel(8);
        let (_stop, cancelled) = tokio::sync::watch::channel(false);
        assert_eq!(
            ssh.exec_stream("sleep 120", Duration::from_secs(1), output, cancelled)
                .await
                .unwrap(),
            -124
        );
    }

    #[tokio::test]
    async fn live_streaming_cancel_with_full_output_buffer() {
        if live_or_fail().is_none() {
            return;
        }
        let ssh = connect().await;
        let (output, _unread) = tokio::sync::mpsc::channel(1);
        let (stop, cancelled) = tokio::sync::watch::channel(false);
        let execution = ssh.exec_stream(
            "while :; do echo backpressure; done",
            Duration::from_secs(30),
            output,
            cancelled,
        );
        let cancellation = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stop.send(true).unwrap();
        };
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(execution, cancellation).0
        })
        .await;
        assert!(
            result.is_ok(),
            "full log buffer must not prevent cancellation"
        );
        assert_ne!(result.unwrap().unwrap(), 0);
    }

    #[tokio::test]
    async fn live_pty_echoes_and_follows_resize() {
        if live_or_fail().is_none() {
            return;
        }
        let ssh = connect().await;
        let mut pty = ssh.open_pty(PtySize { cols: 80, rows: 24 }).await.unwrap();

        // 读到想要的那行为止：登录提示、回显和 shell 提示符会混在一起。
        async fn until(pty: &mut Box<dyn Pty>, needle: &str) -> String {
            let mut seen = String::new();
            for _ in 0..200 {
                match tokio::time::timeout(Duration::from_secs(10), pty.read()).await {
                    Ok(Ok(Some(data))) => seen.push_str(&String::from_utf8_lossy(&data)),
                    _ => break,
                }
                if seen.contains(needle) {
                    return seen;
                }
            }
            panic!("终端里没有出现 {needle}；收到的是 {seen:?}");
        }

        pty.write(b"echo hello-from-pty\n").await.unwrap();
        until(&mut pty, "hello-from-pty").await;

        pty.resize(PtySize {
            cols: 132,
            rows: 43,
        })
        .await
        .unwrap();
        pty.write(b"stty size\n").await.unwrap();
        until(&mut pty, "43 132").await;
    }

    #[tokio::test]
    async fn live_list_dir_separates_files_from_directories() {
        if live_or_fail().is_none() {
            return;
        }
        let ssh = connect().await;
        ssh.exec(
            "rm -rf /tmp/ferry-list && mkdir -p /tmp/ferry-list/subdir \
             && printf 12345 > /tmp/ferry-list/plain.txt \
             && chmod 600 /tmp/ferry-list/plain.txt \
             && ln -s plain.txt /tmp/ferry-list/link",
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        let entries = ssh.list_dir("/tmp/ferry-list").await.unwrap();
        let shape: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("subdir", "directory"),
                ("link", "symlink"),
                ("plain.txt", "file"),
            ]
        );
        let plain = entries.iter().find(|e| e.name == "plain.txt").unwrap();
        assert_eq!(plain.size, 5);
        assert_eq!(plain.mode, 0o600);
        assert!(plain.modified_at > 1_700_000_000_000);

        ssh.create_dir("/tmp/ferry-list/created").await.unwrap();
        ssh.create_file("/tmp/ferry-list/created/empty.txt")
            .await
            .unwrap();
        assert!(
            ssh.create_file("/tmp/ferry-list/created/empty.txt")
                .await
                .is_err()
        );
        let created = ssh.list_dir("/tmp/ferry-list/created").await.unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].name, "empty.txt");
        assert_eq!(created[0].size, 0);
        ssh.delete_file("/tmp/ferry-list/created/empty.txt")
            .await
            .unwrap();
        ssh.delete_file("/tmp/ferry-list/created").await.unwrap();

        assert_eq!(
            ssh.list_dir("/tmp/ferry-nope").await.unwrap_err().0,
            StatusCode::NOT_FOUND
        );
    }

    /// 上传再下载 128 MiB，比对 sha256，并盯住常驻内存。缓冲式实现会在这里
    /// 涨掉上百兆，流式实现不会。
    #[tokio::test]
    async fn live_large_file_round_trips_without_buffering() {
        if live_or_fail().is_none() {
            return;
        }
        use sha2::{Digest, Sha256};
        const BLOCK: usize = 1 << 20;
        const BLOCKS: usize = 128;

        let ssh = connect().await;
        let baseline = rss_kib();

        // 本地也不攒：按块生成、按块算摘要。
        let mut sent = Sha256::new();
        let body = futures::stream::iter((0..BLOCKS).map(move |index| {
            let block = Bytes::from(vec![(index % 251) as u8; BLOCK]);
            Ok(block)
        }));
        for index in 0..BLOCKS {
            sent.update(vec![(index % 251) as u8; BLOCK]);
        }
        let sent = hex::encode(sent.finalize());

        ssh.write_file("/tmp/ferry-big.bin", 0o640, Box::pin(body))
            .await
            .unwrap();
        let after_upload = rss_kib();

        let listed = ssh.list_dir("/tmp").await.unwrap();
        let listed = listed.iter().find(|e| e.name == "ferry-big.bin").unwrap();
        assert_eq!(listed.size as usize, BLOCK * BLOCKS);
        assert_eq!(listed.mode, 0o640);

        let (meta, mut stream) = ssh.read_file("/tmp/ferry-big.bin").await.unwrap();
        assert_eq!(meta.size as usize, BLOCK * BLOCKS);
        let mut received = Sha256::new();
        let mut total = 0usize;
        let mut peak = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            total += chunk.len();
            received.update(&chunk);
            if total % (16 * BLOCK) < CHUNK {
                peak = peak.max(rss_kib());
            }
        }
        assert_eq!(total, BLOCK * BLOCKS);
        assert_eq!(hex::encode(received.finalize()), sent);

        let growth = peak.max(after_upload).saturating_sub(baseline);
        assert!(
            growth < 64 * 1024,
            "常驻内存涨了 {growth} KiB，说明文件被攒在内存里而不是流过去的"
        );

        ssh.delete_file("/tmp/ferry-big.bin").await.unwrap();
        assert_eq!(
            ssh.read_file("/tmp/ferry-big.bin").await.err().unwrap().0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn live_bad_credentials_and_bad_address_read_differently() {
        let Some((host, _)) = live_or_fail() else {
            return;
        };

        let rejected = SshChannel::open(&host, Credential::Password("wrong-password".into()))
            .await
            .err()
            .unwrap();
        assert!(
            rejected.1.contains("拒绝了账号"),
            "认证失败的提示应说明是凭据问题，实际：{}",
            rejected.1
        );

        // 198.51.100.0/24 是 RFC 5737 留给文档的地址，不会有人应答。
        let unreachable = SshChannel::open(
            &Host {
                address: "198.51.100.7".into(),
                port: 22,
                ..host.clone()
            },
            Credential::Password("whatever".into()),
        )
        .await
        .err()
        .unwrap();
        assert!(
            unreachable.1.contains("无法连接到"),
            "连不上的提示应说明是网络问题，实际：{}",
            unreachable.1
        );
        assert_ne!(rejected.1, unreachable.1);
    }

    #[tokio::test]
    async fn live_a_changed_host_key_is_refused() {
        let Some((host, password)) = live_or_fail() else {
            return;
        };

        let learned = SshChannel::open(&host, Credential::Password(password.clone()))
            .await
            .unwrap()
            .learned_host_key
            .unwrap();

        // 同一个指纹再连一次要通过……
        let again = Host {
            host_key: learned.clone(),
            ..host.clone()
        };
        SshChannel::open(&again, Credential::Password(password.clone()))
            .await
            .expect("指纹一致应当放行");

        // ……换一个指纹就必须拒绝，而且要在认证之前拒绝。脚本第二阶段会把
        // 上一台容器（另一把真实主机密钥）的指纹放进这个环境变量，那才是
        // 真正的「主机换了密钥」；没有它时退回一个构造的指纹。
        let changed = Host {
            host_key: std::env::var("FERRY_TEST_SSH_PRIOR_KEY")
                .unwrap_or_else(|_| "SHA256:0000000000000000000000000000000000000000000".into()),
            ..host.clone()
        };
        let error = SshChannel::open(&changed, Credential::Password(password))
            .await
            .err()
            .unwrap();
        assert_eq!(error.0, StatusCode::CONFLICT);
        assert!(error.1.contains("密钥指纹已改变"), "{}", error.1);
        assert!(
            error.1.contains(&learned),
            "提示里要带上实际指纹：{}",
            error.1
        );
    }
}
