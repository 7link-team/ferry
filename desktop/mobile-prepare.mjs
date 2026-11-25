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
if (!source.includes('android.permission.REQUEST_INSTALL_PACKAGES')) {
  source = source.replace('<application', '<uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES" />\n    <application');
}
if (!source.includes('android:authorities="${applicationId}.updates"')) {
  source = source.replace('</application>', `    <provider android:name="androidx.core.content.FileProvider"
            android:authorities="\${applicationId}.updates" android:exported="false" android:grantUriPermissions="true">
            <meta-data android:name="android.support.FILE_PROVIDER_PATHS" android:resource="@xml/update_paths" />
        </provider>
    </application>`);
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
// The JVM verifier must match the Android companion selected by Cargo.
const lock = readFileSync(new URL('./Cargo.lock', import.meta.url), 'utf8');
const verifierVersion = lock.match(/name = "rustls-platform-verifier-android"\r?\nversion = "([^"]+)"/)?.[1];
if (!verifierVersion) throw new Error('Android certificate verifier missing from Cargo.lock');
build = build.replace(/\n\/\/ Android system certificate verifier\nrepositories \{[^\n]+\}\ndependencies \{[^\n]+\}\n/g, '\n').trimEnd();
build += `\n// Android system certificate verifier\nrepositories { maven { url = uri("https://raw.githubusercontent.com/rustls/rustls-platform-verifier/maven-archive/android-release-support/maven/") } }\ndependencies { implementation("org.rustls:rustls-platform-verifier:${verifierVersion}") }\n`;
build = build.replace(/compileSdk = \d+/, 'compileSdk = 36').replace(/targetSdk = \d+/, 'targetSdk = 36');
if (!build.includes('// Persistent mobile release signing')) {
  build += `
// Persistent mobile release signing. Secrets come from the build environment.
// Without them (forks, pull requests) release APKs fall back to the debug key so
// they stay installable for acceptance; tagged releases require the persistent key.
android {
    val updateStore = System.getenv("MOBILE_ANDROID_KEYSTORE_PATH")
    val persistent = if (updateStore.isNullOrBlank()) null else signingConfigs.create("persistent") {
        storeFile = file(updateStore)
        storePassword = requireNotNull(System.getenv("MOBILE_ANDROID_STORE_PASSWORD"))
        keyAlias = requireNotNull(System.getenv("MOBILE_ANDROID_KEY_ALIAS"))
        keyPassword = requireNotNull(System.getenv("MOBILE_ANDROID_KEY_PASSWORD"))
    }
    buildTypes {
        getByName("debug") { if (persistent != null) signingConfig = persistent }
        getByName("release") { signingConfig = persistent ?: signingConfigs.getByName("debug") }
    }
}
`;
}
writeFileSync(gradle, build);
writeFileSync(fileURLToPath(new URL('./gen/android/app/proguard-native-client.pro', import.meta.url)), `# Classes reached from Rust through JNI by name must survive release shrinking.
-keep class org.rustls.platformverifier.** { *; }
-keep class nativeclient.tls.** { *; }
-keep class ${JSON.parse(readFileSync(new URL('./tauri.conf.json', import.meta.url), 'utf8')).identifier}.ClientUpdatePlugin { *; }
-keep class ${JSON.parse(readFileSync(new URL('./tauri.conf.json', import.meta.url), 'utf8')).identifier}.InstallUpdateArgs { *; }
`);

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
if (!kotlin.includes('override fun onWebViewCreate')) {
  kotlin = kotlin.replace(/\n}\s*$/, '\n\n  override fun onWebViewCreate(webView: android.webkit.WebView) {\n    super.onWebViewCreate(webView)\n    followPageColors(webView)\n  }\n}\n');
}
if (!kotlin.includes('super.onCreate(savedInstanceState)\n    enableEdgeToEdge()') || !kotlin.includes('override fun onConfigurationChanged')) {
  throw new Error('Unexpected Android Activity; update the preparation generator against the installed template.');
}
writeFileSync(activity, kotlin);
const protection = readFileSync(new URL('./android/SystemBarProtection.kt', import.meta.url), 'utf8')
  .replace('package __APPLICATION_PACKAGE__', `package ${config.identifier}`);
writeFileSync(activity.replace('MainActivity.kt', 'SystemBarProtection.kt'), protection);
writeFileSync(activity.replace('MainActivity.kt', 'ClientUpdatePlugin.kt'),
  readFileSync(new URL('./android/ClientUpdatePlugin.kt', import.meta.url), 'utf8')
    .replace('package __APPLICATION_PACKAGE__', `package ${config.identifier}`));
const { mkdirSync } = await import('node:fs');
const tlsDirectory = new URL('./gen/android/app/src/main/java/nativeclient/tls/', import.meta.url);
mkdirSync(tlsDirectory, { recursive: true });
writeFileSync(new URL('CertificateVerifier.kt', tlsDirectory),
  readFileSync(new URL('./android/CertificateVerifier.kt', import.meta.url), 'utf8'));
const xml = new URL('./gen/android/app/src/main/res/xml/', import.meta.url);
mkdirSync(xml, { recursive: true });
// rustls-platform-verifier's manifest installs a network security config that forbids
// cleartext everywhere, overriding usesCleartextTraffic. The connection page supports
// HTTP servers, so the app's own resource of the same name takes precedence; HTTPS still
// uses the system trust store.
writeFileSync(new URL('network_security_config.xml', xml), `<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
  <base-config cleartextTrafficPermitted="true">
    <trust-anchors><certificates src="system" /></trust-anchors>
  </base-config>
</network-security-config>
`);
writeFileSync(new URL('update_paths.xml', xml),
  '<?xml version="1.0" encoding="utf-8"?>\n<paths xmlns:android="http://schemas.android.com/apk/res/android"><cache-path name="updates" path="native-updates/" /></paths>\n');
