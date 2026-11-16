//! Where the Runner looks for an Agent CLI, and the PATH its child processes get.
//!
//! A Runner installed as a service (launchd, systemd, a Windows service or a
//! logon task) starts with a minimal PATH that misses where people actually
//! install `codex`, `claude` and `node`: Homebrew, nvm, npm-global, Volta,
//! pnpm, bun. The search path is the service PATH, then the execution user's
//! login-shell PATH, then those well-known locations, so the CLI and the
//! `node` it needs are found the same way for probing and for execution.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

static SEARCH: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// How long the login shell may take to report its PATH.
const LOGIN_SHELL_LIMIT: Duration = Duration::from_secs(3);
const MARK: &str = "__FERRY_PATH__";

/// The directories searched for a CLI, in order. Computed once per process.
pub(crate) fn search() -> &'static [PathBuf] {
    SEARCH.get_or_init(|| {
        let home =
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let login = if cfg!(unix) { login_shell_path() } else { None };
        compose(
            std::env::var_os("PATH"),
            login.as_deref(),
            home.as_deref(),
            &windows_roots(),
        )
    })
}

/// Starts the one-off lookup in the background, so no probe waits for a shell.
pub(crate) fn warm() {
    std::thread::spawn(|| {
        let _ = search();
    });
}

/// The PATH value for a CLI child process: the full search path.
pub(crate) fn child_path() -> OsString {
    std::env::join_paths(search()).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// A short, bounded list of the places searched, for "not found" reports.
pub(crate) fn searched() -> String {
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    describe(
        &well_known(home.as_deref(), &windows_roots()),
        home.as_deref(),
    )
}

fn describe(places: &[PathBuf], home: Option<&Path>) -> String {
    let mut text = String::from("PATH");
    for place in places {
        let shown = match home.and_then(|home| place.strip_prefix(home).ok()) {
            Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
            None => place.to_string_lossy().into_owned(),
        };
        if text.len() + shown.len() + 2 > 240 {
            text.push_str(", …");
            break;
        }
        text.push_str(", ");
        text.push_str(&shown);
    }
    text
}

/// Service PATH, then login-shell PATH, then well-known locations; absolute,
/// existing-or-not (PATH entries are kept as given), without duplicates.
pub(super) fn compose(
    service: Option<OsString>,
    login: Option<&str>,
    home: Option<&Path>,
    windows: &WindowsRoots,
) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = Vec::new();
    let mut add = |path: PathBuf| {
        // A relative entry would search the Agent's working directory.
        if path.is_absolute() && !all.contains(&path) {
            all.push(path);
        }
    };
    if let Some(service) = service {
        std::env::split_paths(&service).for_each(&mut add);
    }
    if let Some(login) = login {
        std::env::split_paths(&OsString::from(login)).for_each(&mut add);
    }
    well_known(home, windows).into_iter().for_each(&mut add);
    all
}

/// Windows per-user roots, read from the environment.
#[derive(Default)]
pub(super) struct WindowsRoots {
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
}

fn windows_roots() -> WindowsRoots {
    WindowsRoots {
        appdata: std::env::var_os("APPDATA").map(PathBuf::from),
        local_appdata: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    }
}

/// Install locations of the common Node and CLI tool managers.
fn well_known(home: Option<&Path>, windows: &WindowsRoots) -> Vec<PathBuf> {
    let mut places = Vec::new();
    if cfg!(windows) {
        if let Some(appdata) = &windows.appdata {
            places.push(appdata.join("npm"));
        }
        if let Some(local) = &windows.local_appdata {
            places.push(local.join("pnpm"));
            places.push(local.join("Volta").join("bin"));
        }
        if let Some(home) = home {
            places.push(home.join(".bun").join("bin"));
        }
        return places;
    }
    places.push(PathBuf::from("/opt/homebrew/bin"));
    places.push(PathBuf::from("/usr/local/bin"));
    if let Some(home) = home {
        for relative in [
            ".local/bin",
            ".npm-global/bin",
            ".volta/bin",
            ".bun/bin",
            "Library/pnpm",
            ".local/share/pnpm",
        ] {
            places.push(home.join(relative));
        }
        places.extend(nvm_versions(&home.join(".nvm/versions/node")));
        places.push(home.join(".fnm/aliases/default/bin"));
    }
    places
}

/// `~/.nvm/versions/node/*/bin`, newest Node version first.
fn nvm_versions(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut versions: Vec<(Vec<u64>, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let numbers = name
                .trim_start_matches('v')
                .split('.')
                .map(|part| part.parse::<u64>().ok())
                .collect::<Option<Vec<_>>>()?;
            Some((numbers, entry.path().join("bin")))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().map(|(_, bin)| bin).collect()
}

/// The execution user's login-shell PATH (interactive, so rc files such as an
/// nvm setup in ~/.zshrc apply), bounded in time; None when unavailable.
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| Path::new(shell).is_absolute() && Path::new(shell).is_file())
        .or_else(|| {
            ["/bin/zsh", "/bin/bash", "/bin/sh"]
                .into_iter()
                .find(|shell| Path::new(shell).is_file())
                .map(str::to_owned)
        })?;
    let mut child = std::process::Command::new(shell)
        .args(["-ilc", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + LOGIN_SHELL_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut output).ok()?;
    marked(&output)
}

/// The PATH printed between the markers; rc files may print other text around it.
pub(super) fn marked(output: &str) -> Option<String> {
    let start = output.find(MARK)? + MARK.len();
    let end = start + output[start..].find(MARK)?;
    let path = output[start..end].trim();
    (!path.is_empty() && path.len() <= 64 * 1024).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_path_comes_first_then_login_shell_then_well_known_without_duplicates() {
        let service = std::env::join_paths([
            if cfg!(windows) { "C:\\svc" } else { "/usr/bin" },
            "relative",
        ])
        .unwrap();
        let login = std::env::join_paths([
            if cfg!(windows) {
                "C:\\login"
            } else {
                "/login/bin"
            },
            if cfg!(windows) { "C:\\svc" } else { "/usr/bin" },
        ])
        .unwrap();
        let all = compose(
            Some(service),
            Some(&login.to_string_lossy()),
            None,
            &WindowsRoots::default(),
        );
        let first = PathBuf::from(if cfg!(windows) { "C:\\svc" } else { "/usr/bin" });
        let second = PathBuf::from(if cfg!(windows) {
            "C:\\login"
        } else {
            "/login/bin"
        });
        assert_eq!(all[0], first);
        assert_eq!(all[1], second);
        assert_eq!(all.iter().filter(|p| **p == first).count(), 1);
        assert!(!all.iter().any(|p| p == Path::new("relative")));
        if !cfg!(windows) {
            assert!(all.contains(&PathBuf::from("/opt/homebrew/bin")));
        }
    }

    #[test]
    fn well_known_locations_include_the_newest_nvm_node_first() {
        let home = tempfile::tempdir().unwrap();
        for version in ["v18.20.1", "v20.11.0", "v9.0.0", "not-a-version"] {
            std::fs::create_dir_all(
                home.path()
                    .join(".nvm/versions/node")
                    .join(version)
                    .join("bin"),
            )
            .unwrap();
        }
        let roots = WindowsRoots {
            appdata: Some(home.path().join("AppData/Roaming")),
            local_appdata: Some(home.path().join("AppData/Local")),
        };
        let places = well_known(Some(home.path()), &roots);
        if cfg!(windows) {
            assert_eq!(places[0], home.path().join("AppData/Roaming").join("npm"));
            assert!(places.contains(&home.path().join(".bun").join("bin")));
        } else {
            let nvm: Vec<_> = places
                .iter()
                .filter(|p| p.to_string_lossy().contains(".nvm"))
                .collect();
            assert_eq!(nvm.len(), 3);
            assert!(nvm[0].ends_with("v20.11.0/bin"));
            assert!(nvm[2].ends_with("v9.0.0/bin"));
            assert!(places.contains(&home.path().join(".volta/bin")));
        }
        let shown = describe(&places, Some(home.path()));
        assert!(shown.starts_with("PATH, "));
        assert!(shown.len() <= 245);
    }

    #[test]
    fn a_cli_outside_the_service_path_is_found_and_its_directory_reaches_children() {
        let service = tempfile::tempdir().unwrap();
        let homebrew = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "codex.exe" } else { "codex" };
        std::fs::write(homebrew.path().join(name), b"").unwrap();
        let directories = compose(
            Some(std::env::join_paths([service.path()]).unwrap()),
            Some(&homebrew.path().to_string_lossy()),
            None,
            &WindowsRoots::default(),
        );
        assert_eq!(
            super::super::probe::executable_in(ferry_protocol::agent::Runtime::Codex, &directories),
            Some(homebrew.path().join(name))
        );
        // Not on the service PATH alone.
        assert_eq!(
            super::super::probe::executable_in(
                ferry_protocol::agent::Runtime::Codex,
                &[service.path().to_owned()]
            ),
            None
        );
        // The child PATH carries every searched directory.
        let joined = std::env::join_paths(&directories).unwrap();
        let split: Vec<_> = std::env::split_paths(&joined).collect();
        assert!(split.contains(&homebrew.path().to_owned()));
    }

    #[test]
    fn the_login_shell_path_is_read_between_markers_only() {
        let noisy = format!("Welcome!\nnvm loaded\n{MARK}/a/bin:/b/bin{MARK}\nbye");
        assert_eq!(marked(&noisy).as_deref(), Some("/a/bin:/b/bin"));
        assert_eq!(marked("no markers here"), None);
        assert_eq!(marked(&format!("{MARK}{MARK}")), None);
    }
}
