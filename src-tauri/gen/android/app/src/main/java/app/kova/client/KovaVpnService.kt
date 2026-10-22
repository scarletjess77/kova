package app.kova.client

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import androidx.core.app.NotificationCompat

/**
 * The VPN interface. Android routes the chosen apps' traffic into it; Mihomo, started by
 * the Rust side, reads and answers it through the descriptor this service hands over.
 * Kova's own traffic, the cores' included, is kept out of the VPN so it can reach the
 * servers.
 */
class KovaVpnService : VpnService() {
    companion object {
        /** The interface is up. Turns false when Kova closes it or Android takes it away. */
        @Volatile
        var active = false
            private set

        /** The notification's Disconnect button. */
        const val ACTION_STOP = "app.kova.client.STOP"

        private var instance: KovaVpnService? = null
        private var pending: ((Int?, String?) -> Unit)? = null
        private var mtu = 9000
        private var mode = "all"
        private var apps: List<String> = emptyList()
        private var label = ""
        /** The node in use and the current speeds, from the Rust side while connected. */
        private var node = ""
        private var speeds = ""
        private const val CHANNEL = "vpn"
        private const val NOTIFICATION_ID = 1

        fun open(context: Context, args: TunnelArgs, done: (Int?, String?) -> Unit) {
            mtu = args.mtu
            mode = args.mode
            apps = args.apps.toList()
            label = args.label
            // Always through startService, never a direct call on `instance`: after a
            // disconnect that instance may be on its way out, and its onDestroy would end
            // the new tunnel. Android finishes the old instance before starting anew.
            pending?.invoke(null, "Superseded by a newer connection attempt")
            pending = done
            context.startService(Intent(context, KovaVpnService::class.java))
        }

        fun close() {
            active = false
            instance?.shutdown()
        }

        /** Show the node in use and the speeds in the notification. */
        fun setDetails(node: String, speeds: String) {
            this.node = node
            this.speeds = speeds
            instance?.takeIf { active }?.refreshNotification()
        }

        /** Redraw what shows Kova's state outside its window: the tile and the widget. */
        fun stateChanged(context: Context) {
            KovaTileService.refresh(context)
            KovaWidget.refresh(context)
        }

        /**
         * Disconnect from outside the window (the tile, the notification): through Kova,
         * so the core stops too, or when Kova isn't there to ask, by closing the VPN.
         */
        fun requestStop() {
            if (!Actions.send(Actions.DISCONNECT, keep = false)) close()
        }
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            requestStop()
            return START_NOT_STICKY
        }
        // Started by Android itself rather than by Kova: Kova is the always-on VPN, and
        // Android brings it up at boot or when the setting is turned on.
        val byAndroid = intent?.action == SERVICE_INTERFACE && pending == null && !active
        showNotification(waiting = byAndroid)
        pending?.let {
            pending = null
            establishTunnel(it)
        }
        if (byAndroid && !Actions.send(Actions.CONNECT, keep = true)) {
            // Kova isn't running. Opening it connects; Android may not allow opening it
            // from here, so the notification says to.
            try {
                startActivity(MainActivity.connectIntent(this).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            } catch (_: Exception) {
            }
        }
        return START_NOT_STICKY
    }

    fun establishTunnel(done: (Int?, String?) -> Unit) {
        try {
            val b = Builder()
                .setSession("Kova")
                .setMtu(mtu)
                .addAddress("172.19.0.1", 30)
                .addRoute("0.0.0.0", 0)
                .addDnsServer("172.19.0.2")
            // IPv6 always goes into the VPN, also with IPv6 off in Kova: a VPN that
            // doesn't take it leaves the phone's IPv6 traffic going around it.
            b.addAddress("fdfe:dcba:9876::1", 126)
            b.addRoute("::", 0)
            // Android takes either a list of apps to include or a list to leave out. Apps
            // uninstalled since they were chosen are skipped.
            if (mode == "only") {
                val included = apps.count { p ->
                    try {
                        b.addAllowedApplication(p)
                        true
                    } catch (_: PackageManager.NameNotFoundException) {
                        false
                    }
                }
                if (included == 0) {
                    done(null, "None of the apps chosen for the VPN is installed. Choose apps in Settings")
                    return
                }
            } else {
                if (mode == "except") {
                    for (p in apps) {
                        try {
                            b.addDisallowedApplication(p)
                        } catch (_: PackageManager.NameNotFoundException) {
                        }
                    }
                }
                b.addDisallowedApplication(packageName)
            }
            if (Build.VERSION.SDK_INT >= 29) b.setMetered(false)
            val tunnel = b.establish()
            if (tunnel == null) {
                done(null, "Android did not allow the VPN. If another app is set as always-on VPN, turn that off first")
                return
            }
            active = true
            node = ""
            speeds = ""
            showNotification(waiting = false)
            stateChanged(this)
            // From here the descriptor belongs to the Rust side, which closes it.
            done(tunnel.detachFd(), null)
        } catch (e: Exception) {
            done(null, e.message ?: e.toString())
        }
    }

    override fun onRevoke() {
        // Another VPN app took over, or the user turned Kova off in the system settings.
        active = false
        shutdown()
    }

    override fun onDestroy() {
        if (instance === this) {
            active = false
            instance = null
        }
        stateChanged(this)
        super.onDestroy()
    }

    private fun shutdown() {
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE) else @Suppress("DEPRECATION") stopForeground(true)
        stopSelf()
        stateChanged(this)
    }

    private fun refreshNotification() {
        try {
            getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, buildNotification(waiting = false))
        } catch (_: Exception) {
        }
    }

    private fun buildNotification(waiting: Boolean): android.app.Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "VPN", NotificationManager.IMPORTANCE_LOW))
        }
        val launch = if (waiting) MainActivity.connectIntent(this) else Intent(this, MainActivity::class.java)
        val open = PendingIntent.getActivity(this, 0, launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val state = when {
            waiting -> getString(R.string.vpn_waiting)
            label.isNotEmpty() -> getString(R.string.vpn_on_profile, label)
            else -> getString(R.string.vpn_on)
        }
        val builder = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(if (!waiting && node.isNotEmpty()) "Kova · $node" else "Kova")
            .setContentText(if (!waiting && speeds.isNotEmpty()) "$state · $speeds" else state)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
        if (!waiting) {
            val stop = PendingIntent.getService(this, 1, Intent(this, KovaVpnService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
            builder.addAction(0, getString(R.string.disconnect), stop)
        }
        return builder.build()
    }

    private fun showNotification(waiting: Boolean) {
        val notification = buildNotification(waiting)
        // A foreground service keeps the VPN running while Kova is in the background.
        // Should Android refuse, the VPN still works, only with less protection.
        try {
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
        } catch (_: Exception) {
        }
    }
}
