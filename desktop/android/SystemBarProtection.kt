package __APPLICATION_PACKAGE__

import android.graphics.Color
import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import android.webkit.WebView
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import java.util.WeakHashMap

// Configuration callbacks reuse the content view; never add insets cumulatively.
private val originalContentPadding = WeakHashMap<View, Rect>()

// Bar colours follow the page: top = its header, bottom = its body. Black until known.
private var pageColors = Color.BLACK to Color.BLACK

private fun parseCss(value: String): Int? {
  val parts = Regex("[0-9.]+").findAll(value).map { it.value.toFloat() }.toList()
  if (parts.size < 3 || (parts.size >= 4 && parts[3] < 0.5f)) return null
  return Color.rgb(parts[0].toInt(), parts[1].toInt(), parts[2].toInt())
}

private fun isLight(color: Int) =
  (0.299 * Color.red(color) + 0.587 * Color.green(color) + 0.114 * Color.blue(color)) > 160

private const val PAGE_COLORS = """(function(){var b=getComputedStyle(document.body).backgroundColor;
var h=document.querySelector('[data-conversation-header],[data-app-header]');
var t=h&&h.offsetHeight?getComputedStyle(h).backgroundColor:b;
if(!t||t==='rgba(0, 0, 0, 0)')t=b;return t+'|'+b;})()"""

/** Reads the page's colours (no native access is given to the page) and paints the bars to match. */
fun MainActivity.followPageColors(webView: WebView) {
  val handler = Handler(Looper.getMainLooper())
  val poll = object : Runnable {
    override fun run() {
      webView.evaluateJavascript(PAGE_COLORS) { raw ->
        val values = raw?.trim('"')?.split('|') ?: emptyList()
        val top = values.getOrNull(0)?.let(::parseCss)
        val bottom = values.getOrNull(1)?.let(::parseCss)
        if (top != null && bottom != null && (top to bottom) != pageColors) {
          pageColors = top to bottom
          val decor = window.decorView as ViewGroup
          WindowCompat.getInsetsController(window, decor).apply {
            isAppearanceLightStatusBars = isLight(top)
            isAppearanceLightNavigationBars = isLight(bottom)
          }
          for (i in 0 until decor.childCount) {
            val child = decor.getChildAt(i)
            val tag = child.tag as? String ?: continue
            if (tag.startsWith("client-status-bar-protection")) child.setBackgroundColor(top)
            else if (tag.startsWith("client-navigation-bar-protection")) child.setBackgroundColor(bottom)
          }
        }
      }
      if (!isDestroyed) handler.postDelayed(this, 800)
    }
  }
  handler.post(poll)
}

// Edge-to-edge system bars are transparent on current Android versions. Protect
// their contrast independently of the untrusted remote document's chosen theme.
fun MainActivity.protectSystemBars() {
  val decor = window.decorView as ViewGroup
  WindowCompat.getInsetsController(window, decor).apply {
    isAppearanceLightStatusBars = isLight(pageColors.first)
    isAppearanceLightNavigationBars = isLight(pageColors.second)
  }
  val content = findViewById<View>(android.R.id.content)
  val padding = originalContentPadding.getOrPut(content) {
    Rect(content.paddingLeft, content.paddingTop, content.paddingRight, content.paddingBottom)
  }
  ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
    val safe = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
    val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
    val keyboardVisible = insets.isVisible(WindowInsetsCompat.Type.ime())
    // Edge-to-edge windows are not resized for the keyboard (adjustResize has no
    // effect), so reserve the keyboard height here; the WebView then shrinks and
    // the page keeps its composer above the keyboard.
    view.setPadding(
      padding.left + safe.left,
      padding.top + safe.top,
      padding.right + safe.right,
      padding.bottom + if (keyboardVisible) maxOf(ime.bottom, safe.bottom) else safe.bottom
    )
    // Native padding owns these safe areas. Forward IME/gesture insets unchanged.
    WindowInsetsCompat.Builder(insets)
      .setInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(), Insets.NONE)
      .setInsetsIgnoringVisibility(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(), Insets.NONE)
      .setDisplayCutout(null)
      .build()
  }
  ViewCompat.requestApplyInsets(content)
  for ((kind, type) in listOf(
    "status" to WindowInsetsCompat.Type.statusBars(),
    "navigation" to WindowInsetsCompat.Type.navigationBars()
  )) for (edge in listOf(Gravity.TOP, Gravity.BOTTOM, Gravity.LEFT, Gravity.RIGHT)) {
    val name = "client-$kind-bar-protection-$edge"
    if (decor.findViewWithTag<View>(name) != null) continue
    val protection = View(this).apply {
      tag = name
      setBackgroundColor(if (kind == "status") pageColors.first else pageColors.second)
      importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
      isClickable = false
      layoutParams = FrameLayout.LayoutParams(0, 0, edge)
    }
    ViewCompat.setOnApplyWindowInsetsListener(protection) { view, insets ->
      // Older Android versions forward consumed child insets to later siblings.
      val windowInsets = ViewCompat.getRootWindowInsets(view) ?: insets
      val bars = windowInsets.getInsets(type)
      val ime = windowInsets.getInsets(WindowInsetsCompat.Type.ime())
      val thickness = when (edge) {
        Gravity.TOP -> bars.top
        Gravity.BOTTOM -> bars.bottom
        Gravity.LEFT -> bars.left
        else -> bars.right
      }
      val imeOwnsEdge = when (edge) {
        Gravity.TOP -> ime.top > 0
        Gravity.BOTTOM -> ime.bottom > 0
        Gravity.LEFT -> ime.left > 0
        else -> ime.right > 0
      }
      // adjustResize moves the app's bottom above the IME. Its navigation bar
      // belongs to the keyboard window; do not paint over the app's last row.
      // Preserve side navigation protection when an IME occupies only bottom.
      view.alpha = if (kind == "navigation" && windowInsets.isVisible(WindowInsetsCompat.Type.ime()) && imeOwnsEdge) 0f else 1f
      view.layoutParams = view.layoutParams.apply {
        width = if (edge == Gravity.LEFT || edge == Gravity.RIGHT) thickness else ViewGroup.LayoutParams.MATCH_PARENT
        height = if (edge == Gravity.TOP || edge == Gravity.BOTTOM) thickness else ViewGroup.LayoutParams.MATCH_PARENT
      }
      insets
    }
    decor.addView(protection)
    ViewCompat.requestApplyInsets(protection)
  }
}
