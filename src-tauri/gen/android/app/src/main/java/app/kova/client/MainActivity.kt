package app.kova.client

import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  companion object {
    /**
     * Open Kova and connect: from the Quick Settings tile, or for always-on VPN. It goes
     * through `.Connect`, an entry other apps can't open, so only Kova can ask for it.
     */
    fun connectIntent(context: Context): Intent = Intent().setClassName(context, "app.kova.client.Connect")
    private const val HANDLED = "app.kova.client.HANDLED"
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android draws apps under the status and navigation bars; keep the page clear of
    // them (and of a camera cutout or the keyboard), with the app's background behind.
    val root = findViewById<View>(android.R.id.content)
    root.setBackgroundColor(Color.parseColor("#101012"))
    ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    connectIfAsked(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    connectIfAsked(intent)
  }

  // With a TV remote the first key press would only move Android's focus onto the page
  // (Android swallows it when leaving touch mode) and do nothing the user can see; the
  // page takes focus as soon as the window has it instead, touch mode or not.
  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) focusPage(20)
  }

  /** The page view appears a moment after the window, so look for it a few times. */
  private fun focusPage(tries: Int) {
    val web = findWebView(findViewById(android.R.id.content))
    if (web == null) {
      if (tries > 0) window.decorView.postDelayed({ focusPage(tries - 1) }, 250)
      return
    }
    web.isFocusableInTouchMode = true
    if (!web.hasFocus()) web.requestFocus()
  }

  private fun findWebView(view: View?): android.webkit.WebView? = when (view) {
    is android.webkit.WebView -> view
    is android.view.ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { findWebView(view.getChildAt(it)) }
    else -> null
  }

  private fun connectIfAsked(intent: Intent?) {
    // Once only: the same intent comes back when Android recreates the window.
    if (intent?.component?.className != "app.kova.client.Connect" || intent.getBooleanExtra(HANDLED, false)) return
    intent.putExtra(HANDLED, true)
    // Waits for Kova to be ready when it is only just starting.
    Actions.send(Actions.CONNECT, keep = true)
  }
}
