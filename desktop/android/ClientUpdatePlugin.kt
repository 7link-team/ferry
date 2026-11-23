package __APPLICATION_PACKAGE__

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class InstallUpdateArgs { lateinit var path: String }

@TauriPlugin
class ClientUpdatePlugin(private val activity: Activity) : Plugin(activity) {
  init { nativeclient.tls.CertificateVerifier.initialize(activity.applicationContext) }

  @Suppress("DEPRECATION")
  private fun signers(info: PackageInfo): Set<String> =
    (if (Build.VERSION.SDK_INT >= 28) info.signingInfo?.apkContentsSigners else info.signatures)
      ?.map { it.toCharsString() }?.toSet() ?: emptySet()

  @Suppress("DEPRECATION")
  private fun version(info: PackageInfo): Long =
    if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong()

  @Command
  @Suppress("DEPRECATION")
  fun install(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(InstallUpdateArgs::class.java)
      val file = File(args.path).canonicalFile
      val expected = File(activity.cacheDir, "native-updates/update.apk").canonicalFile
      require(file == expected && file.isFile)
      val manager = activity.packageManager
      val flags = if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
      val archive = manager.getPackageArchiveInfo(file.path, flags)
        ?: throw IllegalArgumentException("update_invalid")
      val installed = manager.getPackageInfo(activity.packageName, flags)
      require(archive.packageName == activity.packageName && version(archive) > version(installed))
      val installedSigners = signers(installed)
      if (installedSigners.isEmpty() || signers(archive) != installedSigners) {
        invoke.reject("signature_mismatch")
        return
      }
      if (Build.VERSION.SDK_INT >= 26 && !manager.canRequestPackageInstalls()) {
        activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
          Uri.parse("package:${activity.packageName}")))
        invoke.resolve(JSObject().put("status", "permission_required"))
        return
      }
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.updates", file)
      activity.startActivity(Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, "application/vnd.android.package-archive")
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      })
      invoke.resolve(JSObject().put("status", "installer_opened"))
    } catch (_: Exception) {
      invoke.reject("install_failed")
    }
  }
}
