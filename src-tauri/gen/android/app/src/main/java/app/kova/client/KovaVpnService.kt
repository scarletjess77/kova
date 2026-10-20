package app.kova.client

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import androidx.core.app.NotificationCompat

/**
 * The VPN interface. Android routes all traffic into it; Mihomo, started by the Rust
 * side, reads and answers it through the descriptor this service hands over. Kova's own
 * traffic, the cores' included, is kept out of the VPN so it can reach the servers.
 */
class KovaVpnService : VpnService() {
    companion object {
        /** The interface is up. Turns false when Kova closes it or Android takes it away. */
        @Volatile
        var active = false
            private set

        private var instance: KovaVpnService? = null
        private var pending: ((Int?, String?) -> Unit)? = null
        private var mtu = 9000
        private var ipv6 = true
        private const val CHANNEL = "vpn"
        private const val NOTIFICATION_ID = 1

        fun open(context: Context, mtu: Int, ipv6: Boolean, done: (Int?, String?) -> Unit) {
            this.mtu = mtu
            this.ipv6 = ipv6
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
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        showNotification()
        pending?.let {
            pending = null
            establishTunnel(it)
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
            if (ipv6) {
                b.addAddress("fdfe:dcba:9876::1", 126)
                b.addRoute("::", 0)
            }
            b.addDisallowedApplication(packageName)
            if (Build.VERSION.SDK_INT >= 29) b.setMetered(false)
            val tunnel = b.establish()
            if (tunnel == null) {
                done(null, "Android did not allow the VPN. If another app is set as always-on VPN, turn that off first")
                return
            }
            active = true
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
        super.onDestroy()
    }

    private fun shutdown() {
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE) else @Suppress("DEPRECATION") stopForeground(true)
        stopSelf()
    }

    private fun showNotification() {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "VPN", NotificationManager.IMPORTANCE_LOW))
        }
        val open = PendingIntent.getActivity(this, 0, packageManager.getLaunchIntentForPackage(packageName), PendingIntent.FLAG_IMMUTABLE)
        val notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Kova")
            .setContentText("VPN is on")
            .setContentIntent(open)
            .setOngoing(true)
            .build()
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
