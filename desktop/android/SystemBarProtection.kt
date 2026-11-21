package __APPLICATION_PACKAGE__

import android.graphics.Color
import android.graphics.Rect
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import java.util.WeakHashMap

// Configuration callbacks reuse the content view; never add insets cumulatively.
private val originalContentPadding = WeakHashMap<View, Rect>()

// Edge-to-edge system bars are transparent on current Android versions. Protect
// their contrast independently of the untrusted remote document's chosen theme.
fun MainActivity.protectSystemBars() {
  val decor = window.decorView as ViewGroup
  WindowCompat.getInsetsController(window, decor).apply {
    isAppearanceLightStatusBars = false
    isAppearanceLightNavigationBars = false
  }
  val content = findViewById<View>(android.R.id.content)
  val padding = originalContentPadding.getOrPut(content) {
    Rect(content.paddingLeft, content.paddingTop, content.paddingRight, content.paddingBottom)
  }
  ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
    val safe = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
    val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
    val keyboardVisible = insets.isVisible(WindowInsetsCompat.Type.ime())
    // adjustResize already keeps the content above keyboard-owned edges.
    view.setPadding(
      padding.left + if (keyboardVisible && ime.left > 0) 0 else safe.left,
      padding.top + if (keyboardVisible && ime.top > 0) 0 else safe.top,
      padding.right + if (keyboardVisible && ime.right > 0) 0 else safe.right,
      padding.bottom + if (keyboardVisible && ime.bottom > 0) 0 else safe.bottom
    )
    insets
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
      setBackgroundColor(Color.BLACK)
      importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
      isClickable = false
      layoutParams = FrameLayout.LayoutParams(0, 0, edge)
    }
    ViewCompat.setOnApplyWindowInsetsListener(protection) { view, insets ->
      val bars = insets.getInsets(type)
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
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
      view.alpha = if (kind == "navigation" && insets.isVisible(WindowInsetsCompat.Type.ime()) && imeOwnsEdge) 0f else 1f
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
