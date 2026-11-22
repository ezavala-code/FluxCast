package dev.fluxcast.share

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import dev.fluxcast.FluxCastApp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Actividad transparente para Android Share Sheet (`Intent.ACTION_SEND`) y envío rápido
 * desde el Quick Settings Tile hacia el portapapeles de Fedora KDE (cifrado con XChaCha20-Poly1305).
 */
class ShareReceiverActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val app = application as? FluxCastApp
        if (app == null) {
            finish()
            return
        }

        val sharedText = when {
            intent?.action == Intent.ACTION_SEND && intent.type == "text/plain" ->
                intent.getStringExtra(Intent.EXTRA_TEXT)
            intent?.getBooleanExtra(EXTRA_FROM_CLIPBOARD, false) == true -> {
                val cm = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                cm?.primaryClip?.getItemAt(0)?.coerceToText(this)?.toString()
            }
            else -> null
        }?.trim()

        if (sharedText.isNullOrEmpty()) {
            Toast.makeText(this, "FluxCast: no hay texto para enviar a Fedora", Toast.LENGTH_SHORT).show()
            finish()
            return
        }

        CoroutineScope(Dispatchers.IO).launch {
            val result = app.repository.pushTextToFedora(sharedText)
            withContext(Dispatchers.Main) {
                val message = result.fold(
                    onSuccess = { "FluxCast: enviado a Fedora KDE (E2EE)" },
                    onFailure = { "FluxCast: guardado en cola offline (${it.message})" },
                )
                Toast.makeText(applicationContext, message, Toast.LENGTH_SHORT).show()
                finish()
            }
        }
    }

    companion object {
        const val EXTRA_FROM_CLIPBOARD = "dev.fluxcast.EXTRA_FROM_CLIPBOARD"
    }
}
