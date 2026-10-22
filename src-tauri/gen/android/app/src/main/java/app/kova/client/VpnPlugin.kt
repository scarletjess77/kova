package app.kova.client

import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.net.VpnService
import android.os.Build
import android.provider.Settings
import android.util.Base64
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.PermissionState
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayOutputStream

@InvokeArg
class DetailsArgs {
    var node: String = ""
    var speeds: String = ""
}

@InvokeArg
class ApkArgs {
    var path: String = ""
}

@InvokeArg
class TunnelArgs {
    var mtu: Int = 9000
    /** "all", "only" or "except", for [apps]. */
    var mode: String = "all"
    var apps: Array<String> = arrayOf()
    var label: String = ""
}

/**
 * Kova's VPN as the Rust side sees it: asking for permission, opening the interface
 * (whose descriptor Rust hands to the core), closing it, whether it is still up, the
 * installed apps to choose from, and requests from the tile and the notification.
 */
@TauriPlugin(permissions = [Permission(strings = [android.Manifest.permission.POST_NOTIFICATIONS], alias = "notifications")])
class VpnPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun prepare(invoke: Invoke) {
        // Android 13 and later hide the VPN's notification, Disconnect button included,
        // until the app may post notifications: ask first. Android itself stops showing
        // the question once it has been turned down.
        if (Build.VERSION.SDK_INT >= 33 && getPermissionState("notifications") != PermissionState.GRANTED) {
            requestPermissionForAlias("notifications", invoke, "notificationsAnswered")
        } else {
            prepareVpn(invoke)
        }
    }

    @PermissionCallback
    fun notificationsAnswered(invoke: Invoke) {
        prepareVpn(invoke)
    }

    private fun prepareVpn(invoke: Invoke) {
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
        KovaVpnService.open(activity, args) { fd, error ->
            if (fd != null) invoke.resolve(JSObject().put("fd", fd)) else invoke.reject(error ?: "Could not start the VPN")
        }
    }

    @Command
    fun stop(invoke: Invoke) {
        KovaVpnService.close()
        invoke.resolve()
    }

    /** Whether the VPN is up, and whether the screen is on (to work less while it is off). */
    @Command
    fun status(invoke: Invoke) {
        val power = activity.getSystemService(android.os.PowerManager::class.java)
        invoke.resolve(JSObject().put("active", KovaVpnService.active).put("screenOn", power?.isInteractive ?: true))
    }

    @Command
    fun details(invoke: Invoke) {
        val args = invoke.parseArgs(DetailsArgs::class.java)
        KovaVpnService.setDetails(args.node, args.speeds)
        invoke.resolve()
    }

    /** Answered with the next request from the tile, the notification or always-on. */
    @Command
    fun nextAction(invoke: Invoke) {
        Actions.next(invoke)
    }

    @Command
    fun openVpnSettings(invoke: Invoke) {
        try {
            activity.startActivity(Intent(Settings.ACTION_VPN_SETTINGS))
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Could not open the VPN settings")
        }
    }

    /** What kind of device this is: a TV (no touch, a remote) and whether it has a camera. */
    @Command
    fun device(invoke: Invoke) {
        val ui = activity.getSystemService(android.app.UiModeManager::class.java)
        val tv = ui?.currentModeType == android.content.res.Configuration.UI_MODE_TYPE_TELEVISION
        val camera = activity.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)
        invoke.resolve(JSObject().put("tv", tv).put("camera", camera))
    }

    /** Open Android's installer for a downloaded update. */
    @Command
    fun installApk(invoke: Invoke) {
        val args = invoke.parseArgs(ApkArgs::class.java)
        try {
            if (Build.VERSION.SDK_INT >= 26 && !activity.packageManager.canRequestPackageInstalls()) {
                // Android lets an app install others only once the user allows it.
                activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, android.net.Uri.parse("package:" + activity.packageName)))
                invoke.reject("Allow Kova to install apps, then press Download update again")
                return
            }
            val uri = androidx.core.content.FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", java.io.File(args.path))
            val view = Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(view)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Could not open the installer")
        }
    }

    /** The installed apps that may use the network, with a small icon each. */
    @Command
    fun apps(invoke: Invoke) {
        Thread {
            try {
                val pm = activity.packageManager
                val installed = if (Build.VERSION.SDK_INT >= 33) {
                    pm.getInstalledApplications(PackageManager.ApplicationInfoFlags.of(0))
                } else {
                    @Suppress("DEPRECATION") pm.getInstalledApplications(0)
                }
                val list = JSArray()
                for (info in installed) {
                    if (info.packageName == activity.packageName) continue
                    if (pm.checkPermission(android.Manifest.permission.INTERNET, info.packageName) != PackageManager.PERMISSION_GRANTED) continue
                    list.put(
                        JSObject()
                            .put("package", info.packageName)
                            .put("label", pm.getApplicationLabel(info).toString())
                            .put("system", info.flags and ApplicationInfo.FLAG_SYSTEM != 0)
                            .put("icon", iconUrl(pm.getApplicationIcon(info)))
                    )
                }
                invoke.resolve(JSObject().put("apps", list))
            } catch (e: Exception) {
                invoke.reject(e.message ?: "Could not list the apps")
            }
        }.start()
    }

    private fun iconUrl(icon: Drawable): String {
        val size = 64
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        icon.setBounds(0, 0, size, size)
        icon.draw(Canvas(bitmap))
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
        bitmap.recycle()
        return "data:image/png;base64," + Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
    }
}
