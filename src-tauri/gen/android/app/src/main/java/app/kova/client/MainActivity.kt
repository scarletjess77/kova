package app.kova.client

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  companion object {
    /** Opened to connect: from the Quick Settings tile, or for always-on VPN. */
    const val ACTION_CONNECT = "app.kova.client.CONNECT"
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

  private fun connectIfAsked(intent: Intent?) {
    if (intent?.action != ACTION_CONNECT) return
    intent.action = null
    // Waits for Kova to be ready when it is only just starting.
    Actions.send(Actions.CONNECT, keep = true)
  }
}
