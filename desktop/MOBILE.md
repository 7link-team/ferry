# Client-only native shell

Android and iOS reuse the bundled connection launcher and the server-hosted Web
UI. There is one webview, no default server address, and no bundled server,
sandbox, desktop worker resource, updater, tray, autostart or single-instance
plugin. Desktop keeps its existing local-service and remote-client modes.

The launcher verifies the selected HTTP(S) server's `/api/status` JSON (`ok:
true`) before saving `connection.json` and navigating. A server deployed under a
base path is probed under that path. Errors leave the address editable. A fresh
process restores the saved address once. A process-local `auto_connect` response
flag also marks `#connection-settings` after a native history return, because
Android WebView Back can bypass its navigation-policy callback. Persisted
`connection.json` still contains only `server_url`.

Remote content has no capability with a remote origin. Native preference commands
also verify the current webview is the exact bundled document captured during
setup. `app-settings://connection` only requests navigation to that document;
it never accepts configuration values or writes preferences.

The initialization script exposes a **non-security** integration marker,
`document.documentElement.dataset.nativeClient = 'mobile'`, and dispatches
`window` event `client-shell:ready` when the DOM is ready. The shared app can
provide an ordinary link to the settings sentinel and put
`data-client-connection-entry` on its visible trigger. The native fallback link
hides while that entry exists and returns when it disappears. No remote IPC
permission is needed. Android Back retains Tauri's default WebView history
behavior; returning through history is rewritten to the local settings fragment.

Main-document load starts trigger a generation-checked native status probe and
a finite 30-second settings-reachability window. Native evaluation checks for a
visible shared connection entry or fallback; a fresh per-document nonce and the
armed generation reject stale callbacks, including a failed same-URL reload.
The supported native `eval_with_callback` API reads the top document directly;
readiness never navigates the page or invokes a remote native command.
Injection is guarded against child frames; Android's
broader navigation-policy hook does not advance the main-document generation.
A failed status probe or missing liveness acknowledgement returns to local
settings with a localized error. Page-load `Finished` triggers a check and never
proves connectivity itself. The watchdog does not GET arbitrary navigation or
OAuth URLs and does not run periodically after success. Recovery must be tested
with a stopped fixture server and an unavailable second origin while the saved
server remains healthy, on each native platform.

## Android

From the repository root, with the Android SDK/NDK and Java configured:

```powershell
npm run tauri -- android init --ci --skip-targets-install
node desktop/mobile-prepare.mjs
$env:CARGO_TARGET_DIR = "$PWD/desktop/target-mobile"
$env:CARGO_PROFILE_DEV_DEBUG = "0" # smaller emulator APK; keeps debug assertions and WebView debugging
npm run tauri -- android build --debug --target x86_64 --apk --ci
```

`mobile-prepare.mjs` is the source-controlled project preparation input. It updates
the **editable** native manifest to permit the explicitly selected HTTP server
and resize around the keyboard, and the editable app Gradle configuration to
API 36. Tauri CLI 2.12.1 currently emits API 37 while the available SDK repository
does not provide that platform. AndroidX `checkAarMetadata` remains mandatory;
dependencies are not downgraded. No `AUTO-GENERATED` Gradle output is patched.
Run preparation again after regenerating the native project.

The native acceptance runner refuses to overwrite an existing app on the
emulator and uses only disposable local HTTP fixtures:

```powershell
node desktop/mobile-native-acceptance.mjs <path-to-debug.apk>
```

It checks fresh configuration, real HTTP navigation, cookie login, remote native
command rejection, saved selection after process restart, settings, Android Back,
server switching, actual offline page-load recovery and retry. It keeps screenshots
and JSON evidence under ignored `desktop/target-mobile/native-acceptance/`.
It does not stand in for the shared product's full login/conversation/form tests.

Android's editable activity installs the source-controlled `android/` system-bar
protection during `mobile-prepare.mjs`. Native black inset surfaces and light
icons retain contrast with either remote page theme. Protection views receive
no input or accessibility focus; bottom protection hides while the IME owns the
navigation area, preserving the final input and action above the keyboard.
Protection follows all actual inset edges, including side navigation in landscape;
an IME hides only the navigation protection on the edge it occupies.

## iOS

On macOS, use the official `tauri ios init` and `tauri ios build` commands. The
platform override excludes desktop resources and updater artifacts. Tauri merges
`Info.ios.plist`, which permits a user-selected HTTP endpoint in WKWebView.
Simulator compilation and device compilation/signing are separate gates. A
Windows build, an unsigned device binary or source review is not iOS acceptance.

`ios/` is the app-owned native Tauri plugin. Its retained delegate proxy forwards
Wry's existing callbacks and detects main-document failures before WebKit commits
a page. A 30-second provisional timeout and current-navigation identity prevent
an unavailable page from trapping the user; canceled and stale navigation failures
do not reset the current page. The trusted local failure URL comes from Rust's
bundled creation configuration. No remote plugin commands or permissions are added.
The macOS build links this Swift package with the public Tauri library helper;
`.tauri/tauri-api` is a generated copy of the pinned dependency, never committed.
Verify refused connections, DNS/TLS failure, provisional timeout, canceled/stale
loads and iframe failures on an iOS simulator in addition to compilation.

The local-network purpose string covers connecting to the address entered by the
user, with no Bonjour or device-discovery declaration. `Info.ios.plist` supplies
an English fallback. `ios/Localization/{en,zh-Hans,zh-Hant,ms,ja,ko}.lproj/InfoPlist.strings`
provides the six native translations. The supported `bundle.iOS.template` extension
uses `ios/project.yml`, derived from the pinned CLI 2.12.1 template with only a
host-bundle localization resource group added. Default CLI `assets` is a folder
reference and would nest these files incorrectly, so it is not used for prompt
localization. Regenerate via official `tauri ios init`; do not edit generated Xcode
output. On macOS, inspect the built `.app` for root-level language `.lproj`
directories and run `plutil -lint` on its plist/strings. Local-network allow/deny
and a user-selected LAN server connection require a signed physical iOS device;
the simulator does not establish that privacy acceptance gate.

## Focused checks

```powershell
npm exec vitest run -- --config desktop/vitest.config.js
$env:CARGO_TARGET_DIR = "$PWD/desktop/target-mobile"
# Skip packaging inputs only for source tests in a checkout without built sidecars.
$env:TAURI_CONFIG = '{"bundle":{"externalBin":[],"resources":null}}'
cargo test --manifest-path desktop/Cargo.toml --lib
```

The source-test configuration override is not a packaged Desktop acceptance.
Existing ignored sidecar-lifecycle and signed-updater tests retain their explicit
fixture requirements.
