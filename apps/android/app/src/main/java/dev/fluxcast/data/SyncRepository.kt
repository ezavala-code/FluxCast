package dev.fluxcast.data

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.fluxcast.FluxCastApp
import dev.fluxcast.core.uniffiGenerateMasterKey
import dev.fluxcast.core.uniffiOpenEntity
import dev.fluxcast.core.uniffiParsePairingQr
import dev.fluxcast.core.uniffiSealEntity
import dev.fluxcast.security.KeystoreManager
import dev.fluxcast.widget.FluxCastGlanceWidget
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject

data class SyncUiState(
    val serverUrl: String = "http://10.0.2.2:8787",
    val isPaired: Boolean = false,
    val wsConnected: Boolean = false,
    val lastSeq: Long = 0L,
    val statusMessage: String = "Offline-first activo · Vincula por QR con Fedora KDE",
)

/**
 * Repositorio offline-first que coordina SQLite local, cifrado E2EE en Rust vía UniFFI,
 * delta sync (Push/Pull REST), WebSocket en tiempo real (OkHttp) y encolado en WorkManager.
 */
class SyncRepository(
    private val appContext: Context,
    val db: LocalDatabase,
    val keystore: KeystoreManager,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val http = OkHttpClient.Builder()
        .connectTimeout(6, TimeUnit.SECONDS)
        .readTimeout(8, TimeUnit.SECONDS)
        .pingInterval(20, TimeUnit.SECONDS)
        .build()

    private var webSocket: WebSocket? = null

    private val _clipboardFeed = MutableStateFlow<List<ClipboardEntity>>(emptyList())
    val clipboardFeed: StateFlow<List<ClipboardEntity>> = _clipboardFeed.asStateFlow()

    private val _snippets = MutableStateFlow<List<SnippetEntity>>(emptyList())
    val snippets: StateFlow<List<SnippetEntity>> = _snippets.asStateFlow()

    private val _syncState = MutableStateFlow(
        SyncUiState(
            serverUrl = keystore.serverUrl(),
            isPaired = keystore.loadAccessToken() != null && keystore.loadMasterKey() != null,
            lastSeq = keystore.lastSeq(),
        ),
    )
    val syncState: StateFlow<SyncUiState> = _syncState.asStateFlow()

    init {
        db.seedDefaultSnippetsIfEmpty()
        refreshLocalFlows()
        if (_syncState.value.isPaired) {
            connectRealtimeWebSocket()
        }
    }

    fun refreshLocalFlows(clipQuery: String = "", pinnedOnly: Boolean = false, snippetQuery: String = "", snippetTag: String? = null) {
        _clipboardFeed.value = db.listClipboard(clipQuery, pinnedOnly)
        _snippets.value = db.listSnippets(snippetQuery, snippetTag)
    }

    fun ensureMasterKey(): ByteArray {
        keystore.loadMasterKey()?.let { return it }
        val generated = uniffiGenerateMasterKey()
        keystore.saveMasterKey(generated)
        return generated
    }

    /**
     * Vincula la app Android en un paso escaneando el QR efímero (120 s) generado por Fedora KDE.
     */
    suspend fun pairWithQrPayload(qrJson: String): Result<String> = withContext(Dispatchers.IO) {
        runCatching {
            val nowSecs = System.currentTimeMillis() / 1000L
            val info = uniffiParsePairingQr(qrJson.trim(), nowSecs)
            val claimBody = JSONObject().apply {
                put("pairing_token", info.pairingToken)
                put("device_name", android.os.Build.MODEL ?: "Android FluxCast")
                put("platform", "android")
            }.toString().toRequestBody(JSON_MEDIA)

            val req = Request.Builder()
                .url("${info.serverUrl.trimEnd('/')}/api/v1/devices/claim")
                .post(claimBody)
                .build()

            http.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) {
                    error("El servidor rechazó el token QR (${resp.code})")
                }
                val body = JSONObject(resp.body?.string() ?: "{}")
                val userId = body.getString("user_id")
                val deviceId = body.getString("device_id")
                val accessToken = body.getString("access_token")

                keystore.saveMasterKey(info.masterKey)
                keystore.saveSession(info.serverUrl, userId, deviceId, accessToken)
                keystore.setLastSeq(0L)
            }

            _syncState.value = _syncState.value.copy(
                serverUrl = info.serverUrl,
                isPaired = true,
                lastSeq = 0L,
                statusMessage = "Vinculado con Fedora KDE · Sincronizando E2EE…",
            )
            connectRealtimeWebSocket()
            syncDelta()
            "Vinculado con éxito a ${info.serverUrl}"
        }.onFailure { err ->
            _syncState.value = _syncState.value.copy(
                statusMessage = "Error al vincular QR: ${err.message}",
            )
        }
    }

    /**
     * Guarda texto o enlace localmente, lo cifra con XChaCha20-Poly1305 vía UniFFI y lo envía a Fedora.
     */
    suspend fun pushTextToFedora(content: String): Result<ClipboardEntity> = withContext(Dispatchers.IO) {
        runCatching {
            val saved = db.recordClipboard(content, "android")
                ?: error("El contenido está vacío")
            refreshLocalFlows()
            FluxCastGlanceWidget.refreshAll(appContext)
            if (keystore.loadAccessToken() != null) {
                runCatching { syncDelta() }.onFailure { scheduleBackgroundSync() }
            } else {
                scheduleBackgroundSync()
            }
            saved
        }
    }

    /**
     * Ejecuta un ciclo completo Delta Sync E2EE (Push de `sync_outbox` + Pull incremental `since_seq`).
     */
    suspend fun syncDelta(): Result<Pair<Int, Int>> = withContext(Dispatchers.IO) {
        runCatching {
            val token = keystore.loadAccessToken() ?: error("Dispositivo no vinculado")
            val deviceId = keystore.deviceId() ?: error("Falta device_id")
            val masterKey = ensureMasterKey()
            val baseUrl = keystore.serverUrl()

            val outbox = db.pendingOutbox()
            var pushedCount = 0
            if (outbox.isNotEmpty()) {
                val blobsArray = JSONArray()
                val ackIds = mutableListOf<String>()
                for (item in outbox) {
                    val envelope = uniffiSealEntity(
                        masterKey = masterKey,
                        entityId = item.entityId,
                        entityKind = item.entityKind,
                        hlcTimestamp = item.hlcTimestamp,
                        plaintext = item.payloadJson.encodeToByteArray(),
                    )
                    val blobObj = JSONObject().apply {
                        put("entity_id", item.entityId)
                        put("entity_kind", item.entityKind.toInt())
                        put("device_id", deviceId)
                        put("hlc_timestamp", item.hlcTimestamp)
                        put("dedup_hmac", envelope.dedupHmac.toJsonIntArray())
                        put("nonce", envelope.nonce.toJsonIntArray())
                        put("ciphertext", envelope.ciphertext.toJsonIntArray())
                        put("is_deleted", item.isDeleted)
                    }
                    blobsArray.put(blobObj)
                    ackIds.add(item.entityId)
                }

                val pushReq = Request.Builder()
                    .url("$baseUrl/api/v1/sync/push")
                    .header("Authorization", "Bearer $token")
                    .post(JSONObject().put("blobs", blobsArray).toString().toRequestBody(JSON_MEDIA))
                    .build()

                http.newCall(pushReq).execute().use { resp ->
                    if (!resp.isSuccessful) error("Push rechazado (${resp.code})")
                    val json = JSONObject(resp.body?.string() ?: "{}")
                    pushedCount = json.optInt("accepted", 0)
                    db.acknowledgeOutbox(ackIds)
                }
            }

            val sinceSeq = keystore.lastSeq()
            val pullReq = Request.Builder()
                .url("$baseUrl/api/v1/sync/pull?since_seq=$sinceSeq")
                .header("Authorization", "Bearer $token")
                .get()
                .build()

            var pulledCount = 0
            http.newCall(pullReq).execute().use { resp ->
                if (!resp.isSuccessful) error("Pull rechazado (${resp.code})")
                val json = JSONObject(resp.body?.string() ?: "{}")
                val blobs = json.optJSONArray("blobs") ?: JSONArray()
                for (i in 0 until blobs.length()) {
                    if (applyEncryptedBlobJson(masterKey, blobs.getJSONObject(i))) {
                        pulledCount++
                    }
                }
                val latestSeq = json.optLong("latest_seq", sinceSeq).coerceAtLeast(sinceSeq)
                keystore.setLastSeq(latestSeq)
                _syncState.value = _syncState.value.copy(
                    lastSeq = latestSeq,
                    statusMessage = "Sincronizado E2EE · ↑$pushedCount ↓$pulledCount (seq $latestSeq)",
                )
            }

            refreshLocalFlows()
            FluxCastGlanceWidget.refreshAll(appContext)
            pushedCount to pulledCount
        }.onFailure { err ->
            _syncState.value = _syncState.value.copy(
                statusMessage = "Modo offline · pendiente de red (${err.message})",
            )
        }
    }

    fun connectRealtimeWebSocket() {
        val token = keystore.loadAccessToken() ?: return
        val wsUrl = keystore.serverUrl()
            .replaceFirst("https://", "wss://")
            .replaceFirst("http://", "ws://") + "/api/v1/ws?token=$token"

        webSocket?.cancel()
        val req = Request.Builder().url(wsUrl).build()
        webSocket = http.newWebSocket(
            req,
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    _syncState.value = _syncState.value.copy(
                        wsConnected = true,
                        statusMessage = "Conectado en tiempo real con Fedora KDE (WSS E2EE)",
                    )
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    scope.launch {
                        runCatching {
                            val frame = JSONObject(text)
                            if (frame.optString("type") == "broadcast") {
                                val blob = frame.getJSONObject("blob")
                                val key = keystore.loadMasterKey() ?: return@launch
                                if (applyEncryptedBlobJson(key, blob)) {
                                    val seq = blob.optLong("seq", keystore.lastSeq())
                                    if (seq > keystore.lastSeq()) keystore.setLastSeq(seq)
                                    refreshLocalFlows()
                                    FluxCastGlanceWidget.refreshAll(appContext)
                                }
                            }
                        }
                    }
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                    _syncState.value = _syncState.value.copy(wsConnected = false)
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    _syncState.value = _syncState.value.copy(wsConnected = false)
                }
            },
        )
    }

    private fun applyEncryptedBlobJson(masterKey: ByteArray, blob: JSONObject): Boolean {
        val entityId = blob.getString("entity_id")
        val entityKind = blob.getInt("entity_kind").toShort()
        val hlc = blob.getString("hlc_timestamp")
        val isDeleted = blob.optBoolean("is_deleted", false)
        val nonce = blob.getJSONArray("nonce").toByteArray()
        val ciphertext = blob.getJSONArray("ciphertext").toByteArray()

        val plaintextBytes = uniffiOpenEntity(
            masterKey = masterKey,
            entityId = entityId,
            entityKind = entityKind,
            hlcTimestamp = hlc,
            nonce = nonce,
            ciphertext = ciphertext,
        )
        return db.applyRemoteEntity(
            entityId = entityId,
            entityKind = entityKind,
            remoteHlc = hlc,
            isDeleted = isDeleted,
            payloadJson = plaintextBytes.decodeToString(),
        )
    }

    fun scheduleBackgroundSync() {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .build()
        val request = OneTimeWorkRequestBuilder<DeltaSyncWorker>()
            .setConstraints(constraints)
            .build()
        WorkManager.getInstance(appContext)
            .enqueueUniqueWork("fluxcast_delta_sync", ExistingWorkPolicy.REPLACE, request)
    }

    companion object {
        private val JSON_MEDIA = "application/json; charset=utf-8".toMediaType()

        private fun ByteArray.toJsonIntArray(): JSONArray {
            val arr = JSONArray()
            for (b in this) arr.put(b.toInt() and 0xFF)
            return arr
        }

        private fun JSONArray.toByteArray(): ByteArray {
            val out = ByteArray(length())
            for (i in 0 until length()) out[i] = (getInt(i) and 0xFF).toByte()
            return out
        }
    }
}

class DeltaSyncWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {
    override suspend fun doWork(): Result {
        val app = applicationContext as? FluxCastApp ?: return Result.success()
        return app.repository.syncDelta().fold(
            onSuccess = { Result.success() },
            onFailure = { Result.retry() },
        )
    }
}
