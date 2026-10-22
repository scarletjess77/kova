package app.kova.client

import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * The Quick Settings tile. Disconnecting happens in place. Connecting opens Kova:
 * Android lets an app start a VPN only while it is in front, and Kova connects as soon
 * as it opens.
 */
class KovaTileService : TileService() {
    companion object {
        /** Ask Android to redraw the tile after the VPN went up or down. */
        fun refresh(context: Context) {
            try {
                requestListeningState(context, ComponentName(context, KovaTileService::class.java))
            } catch (_: Exception) {
            }
        }
    }

    override fun onStartListening() {
        update()
    }

    override fun onClick() {
        if (KovaVpnService.active) {
            KovaVpnService.requestStop()
            return
        }
        val open = Intent(this, MainActivity::class.java)
            .setAction(MainActivity.ACTION_CONNECT)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE))
        } else {
            @Suppress("DEPRECATION", "StartActivityAndCollapseDeprecated")
            startActivityAndCollapse(open)
        }
    }

    private fun update() {
        val tile = qsTile ?: return
        tile.state = if (KovaVpnService.active) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = getString(R.string.app_name)
        tile.updateTile()
    }
}
