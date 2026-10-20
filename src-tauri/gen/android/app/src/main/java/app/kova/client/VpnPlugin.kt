package app.kova.client

import android.app.Activity
import android.net.VpnService
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class TunnelArgs {
    var mtu: Int = 9000
    var ipv6: Boolean = true
}

/**
 * Kova's VPN as the Rust side sees it: asking for permission, opening the interface
 * (whose descriptor Rust hands to the core), closing it, and whether it is still up.
 */
@TauriPlugin
class VpnPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun prepare(invoke: Invoke) {
        // Asks only the first time, or after another VPN app took over.
        val intent = VpnService.prepare(activity)
        if (intent == null) {
            invoke.resolve(JSObject().put("granted", true))
        } else {
            startActivityForResult(invoke, intent, "prepared")
        }
    }

    @ActivityCallback
    fun prepared(invoke: Invoke, result: ActivityResult) {
        invoke.resolve(JSObject().put("granted", result.resultCode == Activity.RESULT_OK))
    }

    @Command
    fun establish(invoke: Invoke) {
        val args = invoke.parseArgs(TunnelArgs::class.java)
        KovaVpnService.open(activity, args.mtu, args.ipv6) { fd, error ->
            if (fd != null) invoke.resolve(JSObject().put("fd", fd)) else invoke.reject(error ?: "Could not start the VPN")
        }
    }

    @Command
    fun stop(invoke: Invoke) {
        KovaVpnService.close()
        invoke.resolve()
    }

    @Command
    fun status(invoke: Invoke) {
        invoke.resolve(JSObject().put("active", KovaVpnService.active))
    }
}
