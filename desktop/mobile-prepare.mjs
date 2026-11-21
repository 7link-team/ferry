// Run after the official `tauri android init` command. The manifest is an editable
// native source file; never patch Tauri's AUTO-GENERATED Gradle files.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const manifest = fileURLToPath(new URL('./gen/android/app/src/main/AndroidManifest.xml', import.meta.url));
let source = readFileSync(manifest, 'utf8').replaceAll('\r\n', '\n');
source = source.replace('android:usesCleartextTraffic="${usesCleartextTraffic}"', 'android:usesCleartextTraffic="true"');
if (!source.includes('android:windowSoftInputMode=')) {
  source = source.replace('android:launchMode="singleTask"', 'android:launchMode="singleTask"\n            android:windowSoftInputMode="adjustResize"');
}
if (!source.includes('android:usesCleartextTraffic="true"') || !source.includes('android:windowSoftInputMode="adjustResize"')) {
  throw new Error('Unexpected Android manifest; update the mobile preparation generator against the installed Tauri template.');
}
writeFileSync(manifest, source);

// CLI 2.12.1's template defaults to API 37, which is not published by the
// available SDK repository. This editable app build config targets supported
// API 36; Gradle's checkAarMetadata still enforces AndroidX minCompileSdk.
const gradle = fileURLToPath(new URL('./gen/android/app/build.gradle.kts', import.meta.url));
let build = readFileSync(gradle, 'utf8').replaceAll('\r\n', '\n');
if (build.includes('AUTO-GENERATED')) throw new Error('Refusing to patch generated Gradle output');
build = build.replace(/compileSdk = \d+/, 'compileSdk = 36').replace(/targetSdk = \d+/, 'targetSdk = 36');
writeFileSync(gradle, build);

// Protect transparent edge-to-edge system bars independently of remote theme.
const config = JSON.parse(readFileSync(new URL('./tauri.conf.json', import.meta.url), 'utf8'));
const activity = fileURLToPath(new URL(`./gen/android/app/src/main/java/${config.identifier.replaceAll('.', '/')}/MainActivity.kt`, import.meta.url));
let kotlin = readFileSync(activity, 'utf8').replaceAll('\r\n', '\n');
if (kotlin.includes('AUTO-GENERATED')) throw new Error('Refusing to patch generated Activity output');
if (!kotlin.includes('import android.content.res.Configuration')) {
  kotlin = kotlin.replace('import android.os.Bundle', 'import android.content.res.Configuration\nimport android.os.Bundle');
}
if (!kotlin.includes('super.onCreate(savedInstanceState)\n    enableEdgeToEdge()')) {
  kotlin = kotlin.replace('    enableEdgeToEdge()\n', '').replace('    super.onCreate(savedInstanceState)', '    super.onCreate(savedInstanceState)\n    enableEdgeToEdge()');
}
if (!kotlin.includes('override fun onConfigurationChanged')) {
  kotlin = kotlin.replace(/\n}\s*$/, '\n\n  override fun onConfigurationChanged(newConfig: Configuration) {\n    super.onConfigurationChanged(newConfig)\n    enableEdgeToEdge()\n  }\n}\n');
}
if (!kotlin.includes('    protectSystemBars()')) {
  kotlin = kotlin.replaceAll('    enableEdgeToEdge()', '    enableEdgeToEdge()\n    protectSystemBars()');
}
if (!kotlin.includes('super.onCreate(savedInstanceState)\n    enableEdgeToEdge()') || !kotlin.includes('override fun onConfigurationChanged')) {
  throw new Error('Unexpected Android Activity; update the preparation generator against the installed template.');
}
writeFileSync(activity, kotlin);
const protection = readFileSync(new URL('./android/SystemBarProtection.kt', import.meta.url), 'utf8')
  .replace('package __APPLICATION_PACKAGE__', `package ${config.identifier}`);
writeFileSync(activity.replace('MainActivity.kt', 'SystemBarProtection.kt'), protection);
