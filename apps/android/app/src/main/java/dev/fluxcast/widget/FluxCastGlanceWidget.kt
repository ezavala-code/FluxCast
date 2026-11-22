package dev.fluxcast.widget

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.Button
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.action.actionStartActivity
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import dev.fluxcast.FluxCastApp
import dev.fluxcast.MainActivity
import dev.fluxcast.share.ShareReceiverActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Widget de pantalla de inicio construido con Jetpack Glance que muestra el último elemento
 * sincronizado con Fedora KDE y ofrece accesos rápidos para abrir la bóveda o enviar a Fedora.
 */
class FluxCastGlanceWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        val app = context.applicationContext as? FluxCastApp
        val latest = app?.database?.listClipboard()?.firstOrNull()
        val topSnippet = app?.database?.listSnippets()?.firstOrNull()

        provideContent {
            GlanceTheme {
                WidgetBody(
                    latestPreview = latest?.preview ?: "Sin elementos sincronizados aún",
                    latestType = latest?.contentType?.uppercase() ?: "E2EE",
                    topSnippetLabel = topSnippet?.let { "${it.title} (${it.keyword ?: it.language})" }
                        ?: "Bóveda de snippets lista",
                )
            }
        }
    }

    @Composable
    private fun WidgetBody(
        latestPreview: String,
        latestType: String,
        topSnippetLabel: String,
    ) {
        Column(
            modifier = GlanceModifier
                .fillMaxSize()
                .background(GlanceTheme.colors.surface)
                .padding(12.dp),
        ) {
            Row(
                modifier = GlanceModifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "⚡ FluxCast · $latestType",
                    style = TextStyle(
                        color = GlanceTheme.colors.primary,
                        fontWeight = FontWeight.Bold,
                        fontSize = 13.sp,
                    ),
                )
            }
            Spacer(GlanceModifier.height(6.dp))
            Text(
                text = latestPreview,
                maxLines = 2,
                style = TextStyle(
                    color = GlanceTheme.colors.onSurface,
                    fontSize = 12.sp,
                ),
            )
            Spacer(GlanceModifier.height(4.dp))
            Text(
                text = "Snippet frecuente: $topSnippetLabel",
                maxLines = 1,
                style = TextStyle(
                    color = GlanceTheme.colors.secondary,
                    fontSize = 11.sp,
                ),
            )
            Spacer(GlanceModifier.height(8.dp))
            Row(modifier = GlanceModifier.fillMaxWidth()) {
                Button(
                    text = "Abrir Feed",
                    onClick = actionStartActivity<MainActivity>(),
                )
                Spacer(GlanceModifier.width(8.dp))
                Button(
                    text = "Enviar a Fedora",
                    onClick = actionStartActivity<ShareReceiverActivity>(),
                )
            }
        }
    }

    companion object {
        fun refreshAll(context: Context) {
            CoroutineScope(Dispatchers.IO).launch {
                runCatching { FluxCastGlanceWidget().updateAll(context) }
            }
        }
    }
}

class FluxCastWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = FluxCastGlanceWidget()
}
