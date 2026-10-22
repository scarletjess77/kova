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

    private var waiting: Invoke? = null
    private var queued: String? = null

    /** Rust asking for the next request. */
    @Synchronized
    fun next(invoke: Invoke) {
        val q = queued
        if (q != null) {
            queued = null
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
        if (keep) queued = action
        return false
    }
}
