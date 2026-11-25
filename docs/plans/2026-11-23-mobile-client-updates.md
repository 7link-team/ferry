# Mobile client updates — 2026-11-23

## Goal and approved scope

Add online version checks and user-confirmed native update handoff to the existing
bundled connection settings. The user approved the proposed Android download,
verification and system-installer flow and iOS distribution-link flow. They have
no signing material or distribution link yet: implement configuration support and
accurate unavailable states. This change does not alter the immutable
v0.10.11-alpha tag or interrupt its second CI attempt.

## Implementation plan

- Reuse the shared launcher update section, native bundled-document authorization,
  release repository/channel configuration and GitHub Release assets.
- Share native release types/configuration with the existing desktop updater.
  Preserve desktop selection, signature verification and installation behavior.
- Add mobile release selection by SemVer/channel/Android ABI. Download the exact
  configured repository/tag asset in chunks, verify its GitHub SHA256 and size,
  then check Android package identity, increasing versionCode and installed signer
  equality before invoking the system installer. No silent installation.
- Follow the Android unknown-source settings flow and permit retry/cancel. Only
  the update cache subdirectory is exposed by a non-exported FileProvider.
- On iOS, open the configured HTTPS distribution landing page through UIKit.
  Missing distribution configuration disables installation and explains why.
- Support a persistent Android keystore through four optional GitHub secrets.
  Retain explicitly disclosed debug acceptance builds when no key is configured;
  partial credentials fail the build. Keep all secrets out of source/artifacts.
- Test release selection/integrity, launcher behavior and native builds; inspect
  the responsive launcher. Direct execution: changes share native/build contracts,
  so delegation would add coordination without isolating independent contracts.

## Review contract

| Dimension | Rule / evidence |
|---|---|
| Fact sources | `desktop/src/{mobile,updater}.rs`, launcher, native preparation generator and mobile release/workflow scripts. |
| Scope / excluded | Client version check/install handoff/configuration; no store publication, new signing identity, silent install or server business changes. |
| Rollout | Next client build; existing installed binaries do not acquire the new updater automatically. |
| Flow | Idle → checking → current/available/unavailable → downloading → verified → system permission/installer or iOS distribution; errors allow retry. |
| Exceptions | Network/HTTP/rate-limit/invalid metadata, wrong ABI, no compatible asset, missing iOS URL, bad hash/size/signature, cancelled installer. |
| Audit | Existing CI logs/artifact checksums and native errors; no business audit records or new database writes. |
| Delete / retention | Fixed update cache file overwritten on retry; failed download removed, OS may evict cache. No business deletion. |
| Permissions | Existing exact bundled launcher document check before native work and after awaits; remote workspace gets no native update grant. Android installer requires user approval. |
| Calculation | Strict increasing SemVer; Android additionally requires increasing OS versionCode; size and SHA256 computed over streamed bytes. |
| Modes | Android APK install vs iOS HTTPS distribution; desktop updater remains desktop-only. |
| Fields | Existing `plugins.release.repository/includePrereleases`; new `mobile.packageName` and nullable `mobile.iosDistributionUrl` in native configuration. No invented store URL. |
| State operations | Check clears pending metadata; mutex rejects overlapping native updates; install accepts no caller-supplied URL/path. |
| Validation | Repository validation reused; exact asset name/repository/tag; HTTPS metadata links; 64-hex SHA256; package identity/version/signer. |
| Model | Native config and in-memory pending release only; no SQL model. |
| Time audit | No timestamp or audit columns added. |
| Indexes | None. |
| SQL | None; no migration. |
| Copy | zh-CN/en-US/ms-MY; untrusted notes rendered as text. |
| Configuration | Build/deployment config and CI secrets, not business workspace values. Source contains no credentials. |
| States | Current version, checking, no update, available, unconfigured, downloading, permission-required, handed-off, failed/retry. |
| Core flow impact | Client settings only; remote connection and workspace behavior retained. |
| Historical impact | None; previous temporary debug signatures cannot be retroactively made persistent. |
| Third parties | Existing Tauri, GitHub Releases, Android installer/FileProvider, UIKit; SHA256 uses existing workspace dependency family. |
| Compatibility | Additive native commands/config; mobile build required. No old tag mutation. |
| Conflicts | Mobile does not enable the desktop updater plugin; native installer handoff is platform-specific. |
| Tests | Launcher success/failure/retry/progress/iOS missing config/locales; Rust SemVer/ABI/channel/hash/URL tests; Android compile; iOS requires macOS CI. |
| Acceptance tracking | Tests and browser screenshots; no Figma supplied. Actual signed upgrade remains unavailable without keys. |
| Responsive | Desktop ≥1200, Pad 768–1199, Phone 320–767: existing single-column settings, wrapping controls/notes, local notes scroll, 44px targets, safe-area padding. |
| Ambiguity | No installation-success claim from opening an installer; unsigned iOS cannot self-install. |

## Configuration

`desktop/tauri.conf.json`:

```json
{"plugins":{"release":{"mobile":{"packageName":"<release asset prefix>","iosDistributionUrl":null}}}}
```

Use an actual HTTPS TestFlight, App Store or enterprise distribution landing page
for `iosDistributionUrl`, then rebuild. Keep `null` until a valid channel exists.

GitHub Actions secrets (configure all four together; required for tagged releases since
2026-11-25): `MOBILE_ANDROID_KEYSTORE_BASE64`, `MOBILE_ANDROID_STORE_PASSWORD`,
`MOBILE_ANDROID_KEY_ALIAS`, `MOBILE_ANDROID_KEY_PASSWORD`. Android APKs are release
builds signed with that key; the release certificate SHA-256 is pinned in
`.github/workflows/mobile-client.yml` (`7d:fd:8b:48:…:65:2c`) and staging refuses any
other signer on a tag. Builds without the secrets (forks, pull requests) sign release
APKs with the debug key so they stay installable for acceptance.
Retain and securely back up the keystore for every later release. Existing
ephemeral debug-signed installations may require a one-time reinstall; the client
refuses a mismatched signer and never uninstalls or deletes user data itself.

## Verification status

UI RED reproduced before implementation (7 failures); updated launcher suite
29 PASS. Native Rust 15 PASS / 2 existing ignored tests; real HTTP integrity and
partial-file cleanup covered. ARM64/x86_64 Android APK builds PASS. Disposable
Android emulator acceptance 14 PASS, including guarded native update commands
and remote-page isolation. Four rendered viewport/locale cases PASS, including
Malay 320px with enlarged fonts. Generator idempotence and optional signing
configuration were exercised with a disposable keystore. iOS source checks and
7 package fixtures PASS. Actual new-source iOS simulator build, native iPhone
interaction acceptance and unsigned device IPA build all passed in macOS CI.
All 31 release jobs passed; published APK/IPA payloads and all 31 asset digests
were verified against the same immutable source.
This document does not assert physical-device or production-signed installation.

Full candidate verification: `cargo test --workspace` 1429 PASS / 26 existing
ignored; `cargo check --workspace`, `npm run typecheck`, production build and
full frontend 974 PASS / 1 existing skipped. Signing configuration availability
is recorded as a boolean at build time and displayed even when no newer release
exists; no signing path, password or key material is embedded in the client.
