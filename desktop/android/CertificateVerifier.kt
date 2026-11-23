package nativeclient.tls

import android.content.Context

// Stable JNI component namespace; independent of the deployment application ID.
object CertificateVerifier {
  @JvmStatic external fun initialize(context: Context)
}
