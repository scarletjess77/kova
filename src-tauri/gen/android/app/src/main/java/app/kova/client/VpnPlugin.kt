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
class DocumentArgs {
    var name: String = ""
    var text: String = ""
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

    /** What kind of device this is: a TV (no touch, a remote), whether it has a camera,
     *  whether battery saving leaves Kova alone, and who made it (for its own settings). */
    @Command
    fun device(invoke: Invoke) {
        val ui = activity.getSystemService(android.app.UiModeManager::class.java)
        val tv = ui?.currentModeType == android.content.res.Configuration.UI_MODE_TYPE_TELEVISION
        val camera = activity.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)
        val power = activity.getSystemService(android.os.PowerManager::class.java)
        val batteryFree = power?.isIgnoringBatteryOptimizations(activity.packageName) ?: true
        invoke.resolve(
            JSObject().put("tv", tv).put("camera", camera).put("batteryFree", batteryFree)
                .put("maker", Build.MANUFACTURER.lowercase())
        )
    }

    /** Ask Android to leave Kova out of battery saving, which otherwise may stop the VPN
     *  while the screen is off. */
    @Command
    fun allowBackground(invoke: Invoke) {
        try {
            val ask = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, android.net.Uri.parse("package:" + activity.packageName))
            activity.startActivity(ask)
        } catch (e: Exception) {
            // Some phones don't offer the direct question: the list of apps instead.
            activity.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
        }
        invoke.resolve()
    }

    /** The maker's own autostart or background settings (Xiaomi, Huawei, Oppo, Vivo
     *  and others stop apps on their own), or Kova's app settings where there are none. */
    @Command
    fun openAutostart(invoke: Invoke) {
        val pages = listOf(
            "com.miui.securitycenter" to "com.miui.permcenter.autostart.AutoStartManagementActivity",
            "com.huawei.systemmanager" to "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity",
            "com.coloros.safecenter" to "com.coloros.safecenter.permission.startup.StartupAppListActivity",
            "com.oplus.safecenter" to "com.oplus.safecenter.permission.startup.StartupAppListActivity",
            "com.vivo.permissionmanager" to "com.vivo.permissionmanager.activity.BgStartUpManagerActivity",
        )
        for ((pkg, cls) in pages) {
            try {
                activity.startActivity(Intent().setClassName(pkg, cls).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                invoke.resolve()
                return
            } catch (e: Exception) {
                // Not this maker's: try the next one.
            }
        }
        activity.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.parse("package:" + activity.packageName)))
        invoke.resolve()
    }

    /** Save text where the user picks, through Android's file picker. */
    @Command
    fun saveDocument(invoke: Invoke) {
        val args = invoke.parseArgs(DocumentArgs::class.java)
        val pick = Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
            .setType("application/json").putExtra(Intent.EXTRA_TITLE, args.name)
        startActivityForResult(invoke, pick, "documentSaved")
    }

    @ActivityCallback
    fun documentSaved(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject().put("saved", false))
            return
        }
        try {
            val args = invoke.parseArgs(DocumentArgs::class.java)
            activity.contentResolver.openOutputStream(uri, "wt")?.use { it.write(args.text.toByteArray()) }
            invoke.resolve(JSObject().put("saved", true))
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Could not save the file")
        }
    }

    /** A file the user picks, read as text (up to 8 MB). */
    @Command
    fun openDocument(invoke: Invoke) {
        val pick = Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
        startActivityForResult(invoke, pick, "documentOpened")
    }

    @ActivityCallback
    fun documentOpened(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject())
            return
        }
        try {
            val bytes = activity.contentResolver.openInputStream(uri)?.use { input ->
                val out = ByteArrayOutputStream()
                val buf = ByteArray(64 * 1024)
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    out.write(buf, 0, n)
                    if (out.size() > 8 * 1024 * 1024) throw IllegalStateException("That file is larger than 8 MB; it is unlikely to be a config")
                }
                out.toByteArray()
            } ?: ByteArray(0)
            invoke.resolve(JSObject().put("text", String(bytes)))
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Could not read the file")
        }
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
