//! Time zone for the processes this runner starts.
//!
//! Precedence: the CLI model's zone for an Agent turn, then the zone pinned on
//! this host in the platform (`Control::Settings`), then the host's own zone.
//! It is applied as `TZ` on child processes only; the host is never changed,
//! so nothing has to restart.
//!
//! The MSVC C runtime reads `TZ` as a POSIX rule and turns an IANA name into
//! UTC, so on Windows `TZ` is only set when the wanted zone differs from the
//! host's; there the platform warns that some Windows tools ignore it.

use std::sync::{OnceLock, RwLock};
use std::time::Duration;

static PINNED: RwLock<Option<String>> = RwLock::new(None);
static DETECTED: OnceLock<String> = OnceLock::new();

/// The host's IANA zone, or empty when the OS does not say.
///
/// Asked once per process and bounded: it runs before Hello, and a runner that
/// never says Hello fails update activation and is rolled back. A host zone
/// change is picked up when the runner restarts.
pub(crate) fn detected() -> String {
    DETECTED
        .get_or_init(|| {
            bounded(Duration::from_secs(2), || {
                iana_time_zone::get_timezone().ok()
            })
        })
        .clone()
}

/// Runs `lookup` on its own thread; a slow or failing lookup yields "".
fn bounded(limit: Duration, lookup: impl FnOnce() -> Option<String> + Send + 'static) -> String {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(lookup());
    });
    receiver
        .recv_timeout(limit)
        .ok()
        .flatten()
        .filter(|zone| valid(zone))
        .unwrap_or_default()
}

/// A bounded IANA-looking name; anything else is ignored rather than passed on.
pub(crate) fn valid(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 64
        && !zone.contains("..")
        && !zone.starts_with('/')
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b))
}

/// Replaces the host setting; `None` (or an invalid name) means the host's own zone.
pub(crate) fn pin(zone: Option<String>) {
    let zone = zone.filter(|zone| valid(zone));
    *PINNED.write().unwrap_or_else(|p| p.into_inner()) = zone;
}

fn pinned() -> Option<String> {
    PINNED.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// The `TZ` value a child should get, if any.
fn resolve(
    model: Option<&str>,
    pinned: Option<String>,
    host: &str,
    windows: bool,
) -> Option<String> {
    let wanted = model
        .filter(|zone| valid(zone))
        .map(str::to_owned)
        .or(pinned)?;
    (!(windows && wanted == host)).then_some(wanted)
}

/// Sets `TZ` on a child process according to the precedence above.
pub(crate) fn apply(command: &mut tokio::process::Command, model: Option<&str>) {
    if let Some(zone) = resolve(model, pinned(), &detected(), cfg!(windows)) {
        command.env("TZ", zone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_zone_wins_then_host_setting_then_nothing() {
        let pinned = Some("Europe/Berlin".to_owned());
        assert_eq!(
            resolve(Some("Asia/Tokyo"), pinned.clone(), "UTC", false).as_deref(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            resolve(None, pinned.clone(), "UTC", false).as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(resolve(None, None, "UTC", false), None);
        // An empty or malformed model zone falls back to the host setting.
        assert_eq!(
            resolve(Some(""), pinned.clone(), "UTC", false).as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(resolve(Some("../etc/passwd"), None, "UTC", false), None);
    }

    #[test]
    fn windows_leaves_tz_unset_when_the_host_already_matches() {
        assert_eq!(
            resolve(Some("Asia/Shanghai"), None, "Asia/Shanghai", true),
            None
        );
        assert_eq!(
            resolve(Some("Asia/Shanghai"), None, "Asia/Shanghai", false).as_deref(),
            Some("Asia/Shanghai")
        );
        assert_eq!(
            resolve(Some("Asia/Tokyo"), None, "Asia/Shanghai", true).as_deref(),
            Some("Asia/Tokyo")
        );
    }

    #[test]
    fn a_slow_or_failing_lookup_never_delays_startup() {
        let started = std::time::Instant::now();
        let zone = bounded(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(5));
            Some("Asia/Tokyo".to_owned())
        });
        assert_eq!(zone, "");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(bounded(Duration::from_secs(1), || None), "");
        assert_eq!(
            bounded(Duration::from_secs(1), || Some("bad zone; rm".to_owned())),
            ""
        );
        assert_eq!(
            bounded(Duration::from_secs(1), || Some("Asia/Tokyo".to_owned())),
            "Asia/Tokyo"
        );
        // Asked once: later calls are answered from the first result.
        assert_eq!(detected(), detected());
    }

    #[test]
    fn invalid_names_are_never_pinned() {
        assert!(valid("America/Argentina/Buenos_Aires"));
        assert!(valid("Etc/GMT+8"));
        assert!(!valid(""));
        assert!(!valid("Asia/Shanghai; rm -rf /"));
        assert!(!valid(&"a".repeat(65)));
    }

    #[test]
    fn a_pinned_zone_reaches_every_shell_command() {
        if cfg!(windows) && detected() == "Pacific/Chatham" {
            return;
        }
        let _serial = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tz = |command: &tokio::process::Command| {
            command
                .as_std()
                .get_envs()
                .find(|(key, _)| *key == "TZ")
                .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };
        pin(Some("Pacific/Chatham".into()));
        let pinned = tz(&crate::session::shell_command("true"));
        pin(None);
        let auto = tz(&crate::session::shell_command("true"));
        assert_eq!(pinned.as_deref(), Some("Pacific/Chatham"));
        assert_eq!(auto, None);
    }
}

/// Tests that read or change the process-wide setting run one at a time.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
