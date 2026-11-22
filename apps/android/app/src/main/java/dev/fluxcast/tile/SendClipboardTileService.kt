package dev.fluxcast.tile

import android.app.PendingIntent
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast
import dev.fluxcast.FluxCastApp
import dev.fluxcast.share.ShareReceiverActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Quick Settings Tile en la barra de notificaciones de Android para enviar lo que tengas
 * copiado en el celular hacia Fedora KDE con un solo toque.
 */
class SendClipboardTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        qsTile?.apply {
            state = Tile.STATE_INACTIVE
            label = "Enviar a Fedora"
            updateTile()
        }
    }

    override fun onClick() {
        super.onClick()
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
        val directClip = cm?.primaryClip?.getItemAt(0)?.coerceToText(this)?.toString()?.trim()
        val app = application as? FluxCastApp

        if (!directClip.isNullOrEmpty() && app != null) {
            CoroutineScope(Dispatchers.IO).launch {
                app.repository.pushTextToFedora(directClip)
                withContext(Dispatchers.Main) {
                    Toast.makeText(
                        applicationContext,
                        "FluxCast: portapapeles enviado a Fedora KDE",
                        Toast.LENGTH_SHORT,
                    ).show()
                }
            }
            return
        }

        // En Android 10+ si el sistema requiere foco en primer plano para leer el portapapeles,
        // lanzamos la actividad translúcida instantánea que lo lee y cierra sola.
        val intent = Intent(this, ShareReceiverActivity::class.java).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            putExtra(ShareReceiverActivity.EXTRA_FROM_CLIPBOARD, true)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val pending = PendingIntent.getActivity(
                this,
                0,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            startActivityAndCollapse(pending)
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
