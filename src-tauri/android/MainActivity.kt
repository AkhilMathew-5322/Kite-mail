// Copied over the generated MainActivity by CI (see .github/workflows/build.yml).
// Android 15+ draws apps edge-to-edge (behind the status bar, navigation bar and keyboard).
// This passes the size of those system areas to the web UI as CSS variables
// (--sat top, --sab bottom, --sal/--sar sides, --kb keyboard) so the layout can avoid them.
package com.tlr.kitemail

import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  @Volatile private var top = 0f
  @Volatile private var bottom = 0f
  @Volatile private var left = 0f
  @Volatile private var right = 0f
  @Volatile private var ime = 0f
  private var web: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { v, insets ->
      val d = resources.displayMetrics.density
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      val kb = insets.getInsets(WindowInsetsCompat.Type.ime())
      top = bars.top / d
      bottom = bars.bottom / d
      left = bars.left / d
      right = bars.right / d
      ime = kb.bottom / d
      push()
      ViewCompat.onApplyWindowInsets(v, insets)
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    web = webView
    webView.addJavascriptInterface(this, "KiteInsets")
    push()
  }

  /** Read synchronously by the web UI at start-up. */
  @JavascriptInterface
  fun get(): String = "{\"top\":$top,\"bottom\":$bottom,\"left\":$left,\"right\":$right,\"ime\":$ime}"

  private fun push() {
    val w = web ?: return
    val js = "window.__kiteInsets && window.__kiteInsets(${get()})"
    // the page may still be loading: send again a little later
    for (delay in longArrayOf(0, 400, 1500, 4000)) {
      w.postDelayed({ w.evaluateJavascript(js, null) }, delay)
    }
  }
}
