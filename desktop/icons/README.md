# Native application icons

The single artwork master is `public/brand/logo.png`, also used by the Web UI.
The old placeholder master is no longer used. Do not edit generated images.

Run `npm run brand:icons` from the repository root to regenerate desktop PNG,
Windows ICO/Store, macOS ICNS, Android launcher/adaptive foreground and iOS icons
with the pinned Tauri CLI. The original Web artwork remains unchanged. Desktop
icons retain transparency; iOS uses the generator's required opaque white canvas.

Run `npm run brand:icons:check` to reject missing or stale native exports.
After native project initialization, run
`node scripts/export-native-icons.mjs --sync android` or `--sync ios` from the
repository root. Mobile CI performs these steps before packaging; it copies the
generated artwork into the platform's declared icon resources and leaves the
official native project templates/asset manifests intact.
