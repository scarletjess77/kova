package app.kova.client

import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  companion object {
    /**
     * Open Kova and connect: from the Quick Settings tile, or for always-on VPN. It goes
     * through `.Connect`, an entry other apps can't open, so only Kova can ask for it.
     */
    fun connectIntent(context: Context): Intent = Intent().setClassName(context, "app.kova.client.Connect")
    private const val HANDLED = "app.kova.client.HANDLED"
    /** A window was made in this process before. */
    private var opened = false
    /** Kova's windows that exist now. */
    @Volatile
    private var live = 0

    /**
     * Whether Kova has a window. While it has none (closed from the recent apps, Kova
     * running on for the VPN), nothing may call into the page's plugins: they need one.
     * The Rust side asks this directly.
     */
    @JvmStatic
    fun isOpen(): Boolean = live > 0

    private const val PREFS = "kova.window"
    private const val FULL_SCREEN = "full_screen"

    /** Whether the status and navigation bars are kept hidden (full screen). */
    fun fullScreen(context: Context): Boolean = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(FULL_SCREEN, false)

    fun setFullScreen(context: Context, on: Boolean) {
      context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(FULL_SCREEN, on).apply()
    }
  }

  /**
   * Hide the system bars, or show them again. A swipe from the screen's edge brings them
   * back for a moment, and they hide again by themselves.
   */
  fun applyFullScreen() {
    val bars = WindowCompat.getInsetsController(window, window.decorView)
    if (fullScreen(this)) {
      bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      bars.hide(WindowInsetsCompat.Type.systemBars())
    } else {
      bars.show(WindowInsetsCompat.Type.systemBars())
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    live++
    // Android draws apps under the status and navigation bars; keep the page clear of
    // them (and of a camera cutout or the keyboard), with the app's background behind.
    val root = findViewById<View>(android.R.id.content)
    root.setBackgroundColor(Color.parseColor("#101012"))
    ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    // Closed from the recent apps while connected, Kova keeps running for the VPN; opened
    // again, it needs its page in this new window.
    if (opened) Actions.send(Actions.WINDOW, keep = true)
    opened = true
    applyFullScreen()
    connectIfAsked(intent)
  }

  override fun onDestroy() {
    live--
    super.onDestroy()
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
    if (hasFocus) {
      focusPage(20)
      // Android shows the bars again after a dialog, the keyboard or a swipe.
      applyFullScreen()
    }
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
