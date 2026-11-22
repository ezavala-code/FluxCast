package dev.fluxcast.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.fluxcast.data.ClipboardEntity
import dev.fluxcast.data.SnippetEntity
import dev.fluxcast.data.SyncRepository
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FluxCastRootScreen(repository: SyncRepository) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val clipboardFeed by repository.clipboardFeed.collectAsState()
    val snippets by repository.snippets.collectAsState()
    val syncState by repository.syncState.collectAsState()
    var currentTab by remember { mutableIntStateOf(0) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text("⚡ FluxCast", fontWeight = FontWeight.Bold)
                        Text(
                            syncState.statusMessage,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { scope.launch { repository.syncDelta() } }) {
                        Icon(Icons.Default.Refresh, contentDescription = "Sincronizar E2EE ahora")
                    }
                },
            )
        },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    selected = currentTab == 0,
                    onClick = { currentTab = 0 },
                    icon = { Icon(Icons.Default.ContentPaste, contentDescription = null) },
                    label = { Text("Portapapeles") },
                )
                NavigationBarItem(
                    selected = currentTab == 1,
                    onClick = { currentTab = 1 },
                    icon = { Icon(Icons.Default.Code, contentDescription = null) },
                    label = { Text("Snippets") },
                )
                NavigationBarItem(
                    selected = currentTab == 2,
                    onClick = { currentTab = 2 },
                    icon = { Icon(Icons.AutoMirrored.Filled.Send, contentDescription = null) },
                    label = { Text("A Fedora") },
                )
                NavigationBarItem(
                    selected = currentTab == 3,
                    onClick = { currentTab = 3 },
                    icon = { Icon(Icons.Default.Settings, contentDescription = null) },
                    label = { Text("Ajustes") },
                )
            }
        },
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding)
                .padding(horizontal = 14.dp, vertical = 8.dp),
        ) {
            when (currentTab) {
                0 -> ClipboardFeedScreen(
                    items = clipboardFeed,
                    onSearchChange = { q, pinned -> repository.refreshLocalFlows(clipQuery = q, pinnedOnly = pinned) },
                    onCopy = { item -> copyToAndroidClipboard(context, item.content) },
                    onPin = { item ->
                        repository.db.togglePin(item.id)
                        repository.refreshLocalFlows()
                    },
                    onDelete = { item ->
                        repository.db.deleteClipboard(item.id)
                        repository.refreshLocalFlows()
                    },
                )
                1 -> SnippetsOfflineScreen(
                    snippets = snippets,
                    onFilterChange = { q, tag -> repository.refreshLocalFlows(snippetQuery = q, snippetTag = tag) },
                    onExpandAndCopy = { snippet ->
                        val currentClip = readAndroidClipboard(context)
                        val expanded = repository.db.useSnippet(snippet.id, currentClip) ?: snippet.body
                        copyToAndroidClipboard(context, expanded)
                        repository.refreshLocalFlows()
                    },
                    onCreateSnippet = { title, keyword, tags, body ->
                        repository.db.saveSnippet(title, keyword, body, "shell", tags)
                        repository.refreshLocalFlows()
                    },
                )
                2 -> SendToFedoraScreen(
                    onSend = { text ->
                        scope.launch {
                            repository.pushTextToFedora(text).fold(
                                onSuccess = {
                                    Toast.makeText(context, "Enviado a Fedora KDE (E2EE)", Toast.LENGTH_SHORT).show()
                                },
                                onFailure = {
                                    Toast.makeText(context, "Encolado offline: ${it.message}", Toast.LENGTH_SHORT).show()
                                },
                            )
                        }
                    },
                )
                3 -> SettingsAndQrScreen(repository = repository)
            }
        }
    }
}

@Composable
private fun ClipboardFeedScreen(
    items: List<ClipboardEntity>,
    onSearchChange: (String, Boolean) -> Unit,
    onCopy: (ClipboardEntity) -> Unit,
    onPin: (ClipboardEntity) -> Unit,
    onDelete: (ClipboardEntity) -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var pinnedOnly by remember { mutableStateOf(false) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = query,
                onValueChange = {
                    query = it
                    onSearchChange(it, pinnedOnly)
                },
                label = { Text("Buscar en historial E2EE…") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            FilterChip(
                selected = pinnedOnly,
                onClick = {
                    pinnedOnly = !pinnedOnly
                    onSearchChange(query, pinnedOnly)
                },
                label = { Text("Fijados") },
            )
        }

        if (items.isEmpty()) {
            Text(
                "No hay elementos en el historial. Copia algo en Fedora KDE o envía desde la pestaña «A Fedora».",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(top = 24.dp),
            )
        } else {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(items, key = { it.id }) { item ->
                    Card(modifier = Modifier.fillMaxWidth()) {
                        Column(modifier = Modifier.padding(12.dp)) {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                AssistChip(
                                    onClick = { onCopy(item) },
                                    label = { Text("${item.contentType.uppercase()} · ${item.sourceDeviceId}") },
                                )
                                Row {
                                    IconButton(onClick = { onPin(item) }) {
                                        Icon(
                                            Icons.Default.PushPin,
                                            contentDescription = "Fijar",
                                            tint = if (item.isPinned) MaterialTheme.colorScheme.primary
                                            else MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    IconButton(onClick = { onCopy(item) }) {
                                        Icon(Icons.Default.ContentCopy, contentDescription = "Copiar")
                                    }
                                    IconButton(onClick = { onDelete(item) }) {
                                        Icon(Icons.Default.Delete, contentDescription = "Eliminar")
                                    }
                                }
                            }
                            Text(
                                text = item.content,
                                fontFamily = FontFamily.Monospace,
                                style = MaterialTheme.typography.bodySmall,
                                maxLines = 6,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SnippetsOfflineScreen(
    snippets: List<SnippetEntity>,
    onFilterChange: (String, String?) -> Unit,
    onExpandAndCopy: (SnippetEntity) -> Unit,
    onCreateSnippet: (String, String?, List<String>, String) -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var selectedTag by remember { mutableStateOf<String?>(null) }
    var showForm by remember { mutableStateOf(false) }
    var newTitle by remember { mutableStateOf("") }
    var newKeyword by remember { mutableStateOf("") }
    var newTags by remember { mutableStateOf("fedora, docker") }
    var newBody by remember { mutableStateOf("") }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = query,
                onValueChange = {
                    query = it
                    onFilterChange(it, selectedTag)
                },
                label = { Text("Buscar snippet o !alias…") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { showForm = !showForm }) {
                Icon(Icons.Default.Add, contentDescription = "Nuevo snippet")
            }
        }

        Row(
            modifier = Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            listOf("fedora", "docker", "git", "sql", "kotlin", "rust").forEach { tag ->
                FilterChip(
                    selected = selectedTag == tag,
                    onClick = {
                        selectedTag = if (selectedTag == tag) null else tag
                        onFilterChange(query, selectedTag)
                    },
                    label = { Text("#$tag") },
                )
            }
        }

        if (showForm) {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text("Nuevo Snippet Offline", fontWeight = FontWeight.Bold)
                    OutlinedTextField(
                        value = newTitle,
                        onValueChange = { newTitle = it },
                        label = { Text("Título") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = newKeyword,
                        onValueChange = { newKeyword = it },
                        label = { Text("Alias (ej. !dclean)") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = newTags,
                        onValueChange = { newTags = it },
                        label = { Text("Etiquetas (separadas por coma)") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = newBody,
                        onValueChange = { newBody = it },
                        label = { Text("Plantilla ({{clipboard}}, {{date}}, {{uuid}})") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(
                        onClick = {
                            if (newTitle.isNotBlank() && newBody.isNotBlank()) {
                                onCreateSnippet(
                                    newTitle,
                                    newKeyword.ifBlank { null },
                                    newTags.split(",").map { it.trim() },
                                    newBody,
                                )
                                newTitle = ""
                                newKeyword = ""
                                newBody = ""
                                showForm = false
                            }
                        },
                    ) {
                        Text("Guardar snippet")
                    }
                }
            }
        }

        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(snippets, key = { it.id }) { snippet ->
                Card(modifier = Modifier.fillMaxWidth()) {
                    Column(modifier = Modifier.padding(12.dp)) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = "${snippet.title} ${snippet.keyword ?: ""}",
                                    fontWeight = FontWeight.SemiBold,
                                )
                                Text(
                                    text = "${snippet.tags.joinToString(" · ")} · ${snippet.useCount} usos",
                                    style = MaterialTheme.typography.labelSmall,
                                )
                            }
                            Button(onClick = { onExpandAndCopy(snippet) }) {
                                Text("Expandir")
                            }
                        }
                        Spacer(Modifier.height(4.dp))
                        Text(
                            text = snippet.body,
                            fontFamily = FontFamily.Monospace,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SendToFedoraScreen(onSend: (String) -> Unit) {
    val context = LocalContext.current
    var text by remember { mutableStateOf("") }

    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(
            "Enviar texto, comando o URL directo al portapapeles de Fedora KDE",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
        OutlinedTextField(
            value = text,
            onValueChange = { text = it },
            label = { Text("Escribe o pega contenido…") },
            modifier = Modifier
                .fillMaxWidth()
                .height(170.dp),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { text = readAndroidClipboard(context) },
            ) {
                Icon(Icons.Default.ContentPaste, contentDescription = null)
                Text(" Pegar de Android")
            }
            Button(
                onClick = {
                    if (text.isNotBlank()) {
                        onSend(text)
                        text = ""
                    }
                },
            ) {
                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = null)
                Text(" Enviar cifrado a Fedora")
            }
        }
    }
}

@Composable
private fun SettingsAndQrScreen(repository: SyncRepository) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val syncState by repository.syncState.collectAsState()
    var serverUrl by remember(syncState.serverUrl) { mutableStateOf(syncState.serverUrl) }
    var qrPayloadInput by remember { mutableStateOf("") }
    var showCameraScanner by remember { mutableStateOf(false) }
    var biometricEnabled by remember { mutableStateOf(repository.keystore.isBiometricLockEnabled()) }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("Vinculación con Fedora KDE (QR E2EE 120 s)", fontWeight = FontWeight.Bold)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { showCameraScanner = !showCameraScanner }) {
                        Icon(Icons.Default.QrCodeScanner, contentDescription = null)
                        Text(" Escanear QR con CameraX")
                    }
                }
                if (showCameraScanner) {
                    QrScannerView(
                        onQrDetected = { rawQr ->
                            showCameraScanner = false
                            qrPayloadInput = rawQr
                            scope.launch {
                                repository.pairWithQrPayload(rawQr).onSuccess { msg ->
                                    Toast.makeText(context, msg, Toast.LENGTH_SHORT).show()
                                }
                            }
                        },
                        onClose = { showCameraScanner = false },
                    )
                }
                OutlinedTextField(
                    value = qrPayloadInput,
                    onValueChange = { qrPayloadInput = it },
                    label = { Text("O pega aquí el JSON del QR de Fedora") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Button(
                    onClick = {
                        if (qrPayloadInput.isNotBlank()) {
                            scope.launch {
                                repository.pairWithQrPayload(qrPayloadInput).fold(
                                    onSuccess = {
                                        Toast.makeText(context, it, Toast.LENGTH_SHORT).show()
                                    },
                                    onFailure = {
                                        Toast.makeText(context, "Error: ${it.message}", Toast.LENGTH_SHORT).show()
                                    },
                                )
                            }
                        }
                    },
                ) {
                    Text("Vincular dispositivo")
                }
            }
        }

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("Seguridad y Servidor Sync Relay", fontWeight = FontWeight.Bold)
                OutlinedTextField(
                    value = serverUrl,
                    onValueChange = { serverUrl = it },
                    label = { Text("URL del Sync Relay Server") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Button(onClick = { repository.keystore.setServerUrl(serverUrl) }) {
                    Text("Guardar URL")
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("Desbloqueo con BiometricPrompt (Huella / Rostro)")
                    Switch(
                        checked = biometricEnabled,
                        onCheckedChange = {
                            biometricEnabled = it
                            repository.keystore.setBiometricLockEnabled(it)
                        },
                    )
                }
            }
        }
    }
}

private fun copyToAndroidClipboard(context: Context, text: String) {
    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
    cm.setPrimaryClip(ClipData.newPlainText("FluxCast", text))
    Toast.makeText(context, "Copiado al portapapeles", Toast.LENGTH_SHORT).show()
}

private fun readAndroidClipboard(context: Context): String {
    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return ""
    return cm.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString() ?: ""
}
