//! Preview ingress snapshots and resources owned by this server process.
//! Public port/Quick Tunnel modes are explicitly acknowledged, never token-authenticated URLs.
use super::*;
use crate::accounts::addresses::Configuration;
use axum::{extract::Request, routing::any};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    process::Stdio,
    sync::{LazyLock, Mutex, Weak},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::{Child, Command},
    task::JoinHandle,
};

pub const MODES: [&str; 3] = ["domain", "ip_port", "cloudflare"];
const LEASE_MS: i64 = 45_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub preview_default_mode: String,
    pub preview_ip_enabled: bool,
    pub preview_ip_address: String,
    pub preview_ip_bind: String,
    pub preview_port_start: u16,
    pub preview_port_end: u16,
    pub preview_quick_enabled: bool,
    pub preview_tunnel_binary: String,
    pub preview_public_acknowledged: bool,
    pub preview_public_automatic: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            preview_default_mode: "domain".into(),
            preview_ip_enabled: false,
            preview_ip_address: String::new(),
            preview_ip_bind: "0.0.0.0".into(),
            preview_port_start: 43000,
            preview_port_end: 43100,
            preview_quick_enabled: false,
            preview_tunnel_binary: "cloudflared".into(),
            preview_public_acknowledged: false,
            preview_public_automatic: false,
        }
    }
}
impl Settings {
    pub fn update(&mut self, patch: &serde_json::Map<String, Value>) -> Result<()> {
        let mut raw = serde_json::to_value(&*self).map_err(std::io::Error::other)?;
        let values = raw.as_object_mut().unwrap();
        for (key, value) in patch {
            if values.contains_key(key) {
                values.insert(key.clone(), value.clone());
            }
        }
        let next: Self = serde_json::from_value(raw).map_err(|_| error("holes.entryInvalid"))?;
        if !MODES.contains(&next.preview_default_mode.as_str())
            || next.preview_tunnel_binary.is_empty()
            || next.preview_tunnel_binary.len() > 2048
            || next.preview_tunnel_binary.contains('\0')
            || next.preview_port_start == 0
            || next.preview_port_start > next.preview_port_end
        {
            return Err(error("holes.entryInvalid"));
        }
        let binary = std::path::Path::new(&next.preview_tunnel_binary);
        if binary.components().count() > 1 && !binary.is_absolute() {
            return Err(error("holes.entryInvalid"));
        }
        if next.preview_ip_enabled {
            let address = next
                .preview_ip_address
                .parse::<IpAddr>()
                .map_err(|_| error("holes.entryInvalid"))?;
            let bind = next
                .preview_ip_bind
                .parse::<IpAddr>()
                .map_err(|_| error("holes.entryInvalid"))?;
            if address.is_unspecified()
                || address.is_multicast()
                || address.is_ipv4() != bind.is_ipv4()
            {
                return Err(error("holes.entryInvalid"));
            }
        }
        if (next.preview_ip_enabled || next.preview_quick_enabled || next.preview_public_automatic)
            && !next.preview_public_acknowledged
        {
            return Err(error("holes.entryRisk"));
        }
        if (next.preview_default_mode == "ip_port" && !next.preview_ip_enabled)
            || (next.preview_default_mode == "cloudflare" && !next.preview_quick_enabled)
        {
            return Err(error("holes.entryInvalid"));
        }
        *self = next;
        Ok(())
    }
}

pub(super) fn error(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new(key))
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    pub mode: String,
    pub origin: String,
    pub owner: String,
}
impl Entry {
    pub fn read(s: &AppState, row: &holes::Model) -> Result<Option<Self>> {
        row.ingress_config
            .as_ref()
            .map(|raw| {
                let raw = s.vault.open_secret(raw, &aad(row))?;
                serde_json::from_str(&raw).map_err(|e| std::io::Error::other(e).into())
            })
            .transpose()
    }
    pub fn seal(&self, s: &AppState, row: &holes::Model) -> Result<String> {
        s.vault.seal(
            &serde_json::to_string(self).map_err(std::io::Error::other)?,
            &aad(row),
        )
    }
}

/// Dropping an uncommitted preparation closes its listener and kills its child.
pub(super) struct Prepared {
    pub entry: Entry,
    server: Option<JoinHandle<()>>,
    log: Option<JoinHandle<()>>,
    child: Option<Child>,
    directory: Option<std::path::PathBuf>,
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(task) = &self.server {
            task.abort();
        }
        if let Some(task) = &self.log {
            task.abort();
        }
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
        if let Some(dir) = &self.directory {
            let _ = std::fs::remove_file(dir.join("empty.yml"));
            let _ = std::fs::remove_dir(dir);
        }
        // Child has kill_on_drop; no detached helper survives its owning entry.
    }
}
struct Running {
    state: Weak<crate::state::State>,
    generation: String,
    prepared: Prepared,
}
static RUNNING: LazyLock<Mutex<HashMap<i64, Running>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn lease(row: &holes::Model) -> String {
    format!("preview-entry:{}:{}", row.id, row.token_hash)
}

pub(super) fn chosen(
    config: &Configuration,
    mode: Option<&str>,
    acknowledged: bool,
    automatic: bool,
) -> Result<String> {
    let mode = mode.unwrap_or(&config.preview.preview_default_mode);
    let settings = &config.preview;
    if !MODES.contains(&mode) {
        return Err(error("holes.entryInvalid"));
    }
    if mode == "domain" {
        config.require_preview()?;
    } else {
        if !settings.preview_public_acknowledged
            || (!acknowledged && !(automatic && settings.preview_public_automatic))
        {
            return Err(error("holes.entryRisk"));
        }
        if (mode == "ip_port" && !settings.preview_ip_enabled)
            || (mode == "cloudflare" && !settings.preview_quick_enabled)
        {
            return Err(error("holes.entryInvalid"));
        }
    }
    Ok(mode.into())
}

pub(super) async fn prepare(
    s: &AppState,
    row: &holes::Model,
    config: &Configuration,
    mode: &str,
) -> Result<Prepared> {
    let token = s.vault.open_secret(&row.token_sealed, &aad(row))?;
    let mut prepared = Prepared {
        entry: Entry {
            mode: mode.into(),
            origin: String::new(),
            owner: s.instance_id.clone(),
        },
        server: None,
        log: None,
        child: None,
        directory: None,
        #[cfg(windows)]
        job: None,
    };
    if mode == "domain" {
        config.require_preview()?;
        return Ok(prepared);
    }
    let listener = if mode == "ip_port" {
        // This mode advertises a LAN address of the serving node. Refuse a
        // different node's IP instead of returning a deceptively usable URL.
        let public = config
            .preview
            .preview_ip_address
            .parse::<IpAddr>()
            .map_err(|_| error("holes.entryInvalid"))?;
        let check = TcpListener::bind(SocketAddr::new(public, 0))
            .await
            .map_err(|_| error("holes.entryBind"))?;
        drop(check);
        let bind = config
            .preview
            .preview_ip_bind
            .parse::<IpAddr>()
            .map_err(|_| error("holes.entryInvalid"))?;
        let mut listener = None;
        for port in config.preview.preview_port_start..=config.preview.preview_port_end {
            match TcpListener::bind(SocketAddr::new(bind, port)).await {
                Ok(bound) => {
                    listener = Some(bound);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
                Err(_) => return Err(error("holes.entryBind")),
            }
        }
        listener.ok_or_else(|| error("holes.entryPorts"))?
    } else {
        TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| error("holes.entryBind"))?
    };
    let address = listener.local_addr()?;
    if mode == "ip_port" {
        let public = config
            .preview
            .preview_ip_address
            .parse::<IpAddr>()
            .map_err(|_| error("holes.entryInvalid"))?;
        prepared.entry.origin = format!("http://{}", SocketAddr::new(public, address.port()));
    }
    let state = s.clone();
    let expected = mode.to_owned();
    let app = Router::new().fallback(any(move |request: Request| {
        let s = state.clone();
        let token = token.clone();
        let expected = expected.clone();
        async move { super::proxy::serve_entry(s, request, token, &expected).await }
    }));
    prepared.server = Some(tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    }));
    if mode == "cloudflare" {
        let mut command = Command::new(&config.preview.preview_tunnel_binary);
        command
            .args([
                "tunnel",
                "--url",
                &format!("http://{address}"),
                "--no-autoupdate",
                "--protocol",
                "http2",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Explicit isolated config prevents inheriting an operator's named tunnel.
        let dir = s.data_dir.join("preview-tunnels").join(row.id.to_string());
        tokio::fs::create_dir_all(&dir).await?;
        let dir = tokio::fs::canonicalize(dir).await?;
        let config_path = dir.join("empty.yml");
        tokio::fs::write(&config_path, "{}\n").await?;
        command.arg("--config").arg(&config_path);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("TUNNEL_") {
                command.env_remove(key);
            }
        }
        command.current_dir(&dir);
        prepared.directory = Some(dir);
        #[cfg(windows)]
        {
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|_| error("holes.entryBinary"))?;
        #[cfg(windows)]
        {
            prepared.job = Some(child_job(&child)?);
        }
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| error("holes.entryUnavailable"))?;
        prepared.child = Some(child);
        let mut lines = BufReader::new(stderr).lines();
        let ready = tokio::time::timeout(Duration::from_secs(45), async {
            let mut origin = None;
            let mut connected = false;
            while let Some(line) = lines.next_line().await? {
                if origin.is_none() {
                    origin = quick_origin(&line);
                }
                connected |= line.contains("Registered tunnel connection");
                if let Some(url) = origin.as_ref().filter(|_| connected) {
                    return Ok::<_, std::io::Error>(url.clone());
                }
            }
            Err(std::io::Error::other("connector exited before readiness"))
        })
        .await;
        prepared.entry.origin = ready
            .map_err(|_| error("holes.entryTimeout"))?
            .map_err(|_| error("holes.entryUnavailable"))?;
        prepared.log = Some(tokio::spawn(async move {
            while let Ok(Some(_)) = lines.next_line().await {}
        }));
    }
    Ok(prepared)
}

#[cfg(windows)]
fn child_job(child: &Child) -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    // Closing this handle on server termination also terminates the connector.
    // A normal kill_on_drop alone cannot clean up after a forced server exit.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(error("holes.entryUnavailable"));
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let process = child
        .raw_handle()
        .ok_or_else(|| error("holes.entryUnavailable"))?;
    if unsafe {
        SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(raw, process) } == 0
    {
        return Err(error("holes.entryUnavailable"));
    }
    Ok(handle)
}

fn quick_origin(line: &str) -> Option<String> {
    for word in line.split_whitespace() {
        let url = word.trim_matches(|c: char| {
            !c.is_ascii_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '-'
        });
        let parsed = reqwest::Url::parse(url).ok();
        if let Some(parsed) = parsed {
            let host = parsed.host_str()?;
            if parsed.scheme() == "https"
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.port().is_none()
                && parsed.path() == "/"
                && parsed.query().is_none()
                && parsed.fragment().is_none()
                && host
                    .strip_suffix(".trycloudflare.com")
                    .is_some_and(|label| !label.is_empty() && !label.contains('.'))
            {
                return Some(parsed.origin().ascii_serialization());
            }
        }
    }
    None
}

pub(super) async fn activate(
    conn: &impl ConnectionTrait,
    s: &AppState,
    row: &holes::Model,
    prepared: &Prepared,
) -> Result<()> {
    if prepared.entry.mode != "domain"
        && !crate::lock::acquire(conn, &lease(row), &s.instance_id, db::now(), LEASE_MS).await?
    {
        return Err(error("holes.entryUnavailable"));
    }
    Ok(())
}
pub(super) fn publish(s: &AppState, row: &holes::Model, prepared: Prepared) {
    if prepared.entry.mode == "domain" {
        return;
    }
    RUNNING.lock().unwrap().insert(
        row.id,
        Running {
            state: std::sync::Arc::downgrade(s),
            generation: row.token_hash.clone(),
            prepared,
        },
    );
}
pub(super) async fn stop(s: &AppState, id: i64) {
    let running = {
        let mut running = RUNNING.lock().unwrap();
        if running
            .get(&id)
            .is_some_and(|r| r.state.ptr_eq(&std::sync::Arc::downgrade(s)))
        {
            running.remove(&id)
        } else {
            None
        }
    };
    if let Some(running) = running {
        let name = format!("preview-entry:{id}:{}", running.generation);
        drop(running);
        let _ = crate::lock::release(&s.db, &name, &s.instance_id).await;
    }
}
pub(super) async fn available(s: &AppState, row: &holes::Model, entry: &Entry) -> Result<bool> {
    if state(row, db::now()) != "active" {
        return Ok(false);
    }
    if entry.mode == "domain" {
        return Ok(true);
    }
    Ok(crate::entities::locks::Entity::find_by_id(lease(row))
        .one(&s.db)
        .await?
        .is_some_and(|lock| lock.owner == entry.owner && lock.expires_at > db::now()))
}
pub(super) async fn tick(s: &AppState, shutdown: bool) {
    let ids: Vec<_> = {
        let mut running = RUNNING.lock().unwrap();
        running.retain(|_, r| r.state.strong_count() > 0);
        running
            .iter_mut()
            .filter(|(_, r)| r.state.ptr_eq(&std::sync::Arc::downgrade(s)))
            .map(|(id, r)| {
                let exited = r
                    .prepared
                    .server
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
                    || r.prepared
                        .child
                        .as_mut()
                        .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
                (*id, r.generation.clone(), exited)
            })
            .collect()
    };
    for (id, generation, exited) in ids {
        let live = if shutdown || exited {
            false
        } else {
            holes::Entity::find_by_id(id)
                .one(&s.db)
                .await
                .ok()
                .flatten()
                .is_some_and(|row| {
                    row.token_hash == generation && state(&row, db::now()) == "active"
                })
        };
        let name = format!("preview-entry:{id}:{generation}");
        if !live
            || !matches!(
                crate::lock::renew(&s.db, &name, &s.instance_id, db::now(), LEASE_MS).await,
                Ok(1)
            )
        {
            super::proxy::close(id);
            stop(s, id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_settings_require_opt_in_and_validate_addresses() {
        let mut settings = Settings::default();
        assert!(
            settings
                .update(json!({"preview_quick_enabled":true}).as_object().unwrap())
                .is_err()
        );
        settings.update(json!({"preview_public_acknowledged":true,"preview_ip_enabled":true,"preview_ip_address":"192.168.1.9"}).as_object().unwrap()).unwrap();
        for patch in [
            json!({"preview_ip_address":"0.0.0.0"}),
            json!({"preview_port_start":0}),
            json!({"preview_port_end":2}),
            json!({"preview_default_mode":"unknown"}),
        ] {
            assert!(settings.update(patch.as_object().unwrap()).is_err());
        }
        let config = Configuration {
            preview: settings,
            ..Default::default()
        };
        assert!(chosen(&config, Some("ip_port"), false, false).is_err());
        assert_eq!(
            chosen(&config, Some("ip_port"), true, false).unwrap(),
            "ip_port"
        );
        assert!(chosen(&config, Some("ip_port"), false, true).is_err());
    }
    #[test]
    fn connector_output_only_accepts_exact_https_quick_origins() {
        assert_eq!(
            quick_origin("INF | https://synthetic-preview.trycloudflare.com |"),
            Some("https://synthetic-preview.trycloudflare.com".into())
        );
        for value in [
            "http://a.trycloudflare.com",
            "https://a.trycloudflare.com.evil.test",
            "https://a.b.trycloudflare.com",
            "https://a.trycloudflare.com/path",
            "https://user@a.trycloudflare.com",
        ] {
            assert!(quick_origin(value).is_none(), "{value}");
        }
    }
}
