package app.kova.client

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/**
 * The home-screen widget: whether Kova is connected, and a tap to change it. Like the
 * tile, disconnecting happens in place and connecting opens Kova.
 */
class KovaWidget : AppWidgetProvider() {
    companion object {
        fun refresh(context: Context) {
            try {
                val manager = AppWidgetManager.getInstance(context)
                val ids = manager.getAppWidgetIds(ComponentName(context, KovaWidget::class.java))
                if (ids.isNotEmpty()) manager.updateAppWidget(ids, views(context))
            } catch (_: Exception) {
            }
        }

        private fun views(context: Context): RemoteViews {
            val on = KovaVpnService.active
            val tap = if (on) {
                PendingIntent.getService(context, 2, Intent(context, KovaVpnService::class.java).setAction(KovaVpnService.ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
            } else {
                PendingIntent.getActivity(context, 2, MainActivity.connectIntent(context), PendingIntent.FLAG_IMMUTABLE)
            }
            return RemoteViews(context.packageName, R.layout.widget).apply {
                setTextViewText(R.id.widget_state, context.getString(if (on) R.string.widget_on else R.string.widget_off))
                setTextViewText(R.id.widget_action, context.getString(if (on) R.string.disconnect else R.string.widget_connect))
                setImageViewResource(R.id.widget_dot, if (on) R.drawable.widget_dot_on else R.drawable.widget_dot_off)
                setOnClickPendingIntent(R.id.widget_root, tap)
            }
        }
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        manager.updateAppWidget(ids, views(context))
    }
}
