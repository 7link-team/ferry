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

use std::sync::RwLock;

static PINNED: RwLock<Option<String>> = RwLock::new(None);

/// The host's IANA zone, or empty when the OS does not say.
pub(crate) fn detected() -> String {
    iana_time_zone::get_timezone()
        .ok()
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
