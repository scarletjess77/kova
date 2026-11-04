package app.kova.client

import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

/**
 * Requests that come from outside Kova's window (the Quick Settings tile, the
 * notification's Disconnect button, Android starting the VPN for always-on), passed to
 * the Rust side. Rust keeps one call waiting for the next request; it is answered when
 * one comes.
 */
object Actions {
    const val CONNECT = "connect"
    const val DISCONNECT = "disconnect"
    /** Kova's window was opened again while the VPN kept Kova running without it. */
    const val WINDOW = "window"

    private var waiting: Invoke? = null
    private var queued: String? = null
    /** The latest network change (`network:wifi`, `network:mobile`), kept apart so it
     *  never pushes out a waiting connect request; only the newest one matters. */
    private var queuedNetwork: String? = null
    /** Likewise the latest DNS servers of the network (`dns:…`). */
    private var queuedDns: String? = null

    /** Rust asking for the next request. */
    @Synchronized
    fun next(invoke: Invoke) {
        val q = queued ?: queuedNetwork ?: queuedDns
        if (q != null) {
            when {
                q === queued -> queued = null
                q === queuedNetwork -> queuedNetwork = null
                else -> queuedDns = null
            }
            invoke.resolve(JSObject().put("action", q))
        } else {
            waiting = invoke
        }
    }

    /**
     * Pass [action] to Rust. False when Kova isn't running to take it; with [keep] the
     * request then waits until Kova starts.
     */
    @Synchronized
    fun send(action: String, keep: Boolean): Boolean {
        val w = waiting
        if (w != null) {
            waiting = null
            w.resolve(JSObject().put("action", action))
            return true
        }
        if (keep) {
            when {
                action.startsWith("network:") -> queuedNetwork = action
                action.startsWith("dns:") -> queuedDns = action
                else -> queued = action
            }
        }
        return false
    }
}
