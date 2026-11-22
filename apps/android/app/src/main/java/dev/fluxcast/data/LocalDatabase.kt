package dev.fluxcast.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import dev.fluxcast.core.uniffiHlcShouldApplyRemote
import java.security.MessageDigest
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONArray
import org.json.JSONObject

data class ClipboardEntity(
    val id: String,
    val contentType: String,
    val content: String,
    val preview: String,
    val dedupHmac: String,
    val sourceDeviceId: String,
    val isPinned: Boolean,
    val hlcTimestamp: String,
    val createdAt: Long,
    val updatedAt: Long,
)

data class SnippetEntity(
    val id: String,
    val title: String,
    val keyword: String?,
    val body: String,
    val language: String,
    val tags: List<String>,
    val useCount: Int,
    val hlcTimestamp: String,
    val createdAt: Long,
    val updatedAt: Long,
)

data class OutboxRecord(
    val entityId: String,
    val entityKind: Short,
    val hlcTimestamp: String,
    val isDeleted: Boolean,
    val payloadJson: String,
)

object ContentTools {
    private val hlcSeq = AtomicInteger(0)

    fun nextHlc(nowMs: Long = System.currentTimeMillis(), nodeId: String = "android"): String {
        val seq = hlcSeq.getAndIncrement() and 0xFFFF
        return String.format(Locale.US, "%015d-%04x-%s", nowMs, seq, nodeId)
    }

    fun classify(content: String): String {
        val t = content.trim()
        if (t.split('.').size == 3 && !t.contains(' ') && t.startsWith("eyJ")) return "jwt"
        if (t.matches(Regex("^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$")) ||
            t.lowercase().startsWith("rgb(")
        ) {
            return "color"
        }
        if ((t.startsWith("{") && t.endsWith("}")) || (t.startsWith("[") && t.endsWith("]"))) {
            val isJson = runCatching {
                if (t.startsWith("{")) JSONObject(t) else JSONArray(t)
                true
            }.getOrDefault(false)
            if (isJson) return "json"
        }
        if (!t.contains(' ') && (t.startsWith("https://") || t.startsWith("http://"))) return "url"
        if (listOf("fun ", "fn ", "val ", "const ", "import ", "SELECT ", "docker ", "git ", "sudo dnf ")
                .any { t.contains(it) }
        ) {
            return "code"
        }
        return "text"
    }

    fun preview(content: String): String =
        content.trim().replace(Regex("\\s+"), " ").take(240)

    fun sha256Hex(content: String): String {
        val digest = MessageDigest.getInstance("SHA-256")
            .digest(("fluxcast-dedup-v1:" + content.trim()).encodeToByteArray())
        return digest.joinToString("") { "%02x".format(it) }
    }

    fun expandTemplate(template: String, clipboard: String, nowMs: Long = System.currentTimeMillis()): String {
        val isoDate = SimpleDateFormat("yyyy-MM-dd", Locale.US).apply {
            timeZone = TimeZone.getTimeZone("UTC")
        }.format(Date(nowMs))
        val isoDateTime = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.US).apply {
            timeZone = TimeZone.getTimeZone("UTC")
        }.format(Date(nowMs))
        return template
            .replace("{{clipboard}}", clipboard)
            .replace("{{date}}", isoDate)
            .replace("{{date:YYYY-MM-DD}}", isoDate)
            .replace("{{date:YYYY-MM-DD HH:mm}}", isoDateTime)
            .replace("{{uuid}}", UUID.randomUUID().toString())
            .replace("{{uuid4}}", UUID.randomUUID().toString())
    }
}

/**
 * Base de datos SQLite local (esquema idéntico a Fedora `clipboard_items`, `snippets`, `sync_outbox`)
 * con resolución Last-Write-Wins sobre `hlc_timestamp`.
 */
class LocalDatabase(context: Context) : SQLiteOpenHelper(context, "fluxcast_android.db", null, 1) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS clipboard_items (
                id TEXT PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                preview TEXT NOT NULL,
                dedup_hmac TEXT NOT NULL,
                source_device_id TEXT NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                hlc_timestamp TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted_at INTEGER
            )
            """.trimIndent(),
        )
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS snippets (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                keyword TEXT UNIQUE,
                body TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'plaintext',
                tags_json TEXT NOT NULL DEFAULT '[]',
                use_count INTEGER NOT NULL DEFAULT 0,
                hlc_timestamp TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted_at INTEGER
            )
            """.trimIndent(),
        )
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS sync_outbox (
                entity_id TEXT PRIMARY KEY,
                entity_kind INTEGER NOT NULL,
                queued_at INTEGER NOT NULL
            )
            """.trimIndent(),
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit

    fun seedDefaultSnippetsIfEmpty() {
        val db = writableDatabase
        val count = db.rawQuery("SELECT COUNT(*) FROM snippets", null).use {
            if (it.moveToFirst()) it.getInt(0) else 0
        }
        if (count > 0) return
        saveSnippet(
            title = "Limpiar contenedores Docker/Podman",
            keyword = "!dprune",
            body = "docker system prune -af --volumes",
            language = "shell",
            tags = listOf("docker", "fedora"),
        )
        saveSnippet(
            title = "Logs de servicio systemd en Fedora",
            keyword = "!klog",
            body = "journalctl --user -n 100 --no-pager -u {{clipboard}}",
            language = "shell",
            tags = listOf("fedora", "systemd"),
        )
    }

    fun recordClipboard(content: String, sourceDevice: String = "android"): ClipboardEntity? {
        val trimmed = content.trim()
        if (trimmed.isEmpty()) return null
        val now = System.currentTimeMillis()
        val hlc = ContentTools.nextHlc(now, sourceDevice)
        val hmac = ContentTools.sha256Hex(trimmed)
        val type = ContentTools.classify(trimmed)
        val prev = ContentTools.preview(trimmed)
        val db = writableDatabase

        val existing = db.rawQuery(
            "SELECT id, is_pinned, created_at FROM clipboard_items WHERE dedup_hmac = ? AND deleted_at IS NULL LIMIT 1",
            arrayOf(hmac),
        ).use { c ->
            if (c.moveToFirst()) Triple(c.getString(0), c.getInt(1) != 0, c.getLong(2)) else null
        }

        val id: String
        val pinned: Boolean
        val createdAt: Long
        if (existing != null) {
            id = existing.first
            pinned = existing.second
            createdAt = existing.third
            val cv = ContentValues().apply {
                put("content", content)
                put("preview", prev)
                put("content_type", type)
                put("hlc_timestamp", hlc)
                put("updated_at", now)
            }
            db.update("clipboard_items", cv, "id = ?", arrayOf(id))
        } else {
            id = UUID.randomUUID().toString()
            pinned = false
            createdAt = now
            val cv = ContentValues().apply {
                put("id", id)
                put("content_type", type)
                put("content", content)
                put("preview", prev)
                put("dedup_hmac", hmac)
                put("source_device_id", sourceDevice)
                put("is_pinned", 0)
                put("hlc_timestamp", hlc)
                put("created_at", now)
                put("updated_at", now)
            }
            db.insertWithOnConflict("clipboard_items", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
        }
        queueOutbox(db, id, 1, now)
        return ClipboardEntity(id, type, content, prev, hmac, sourceDevice, pinned, hlc, createdAt, now)
    }

    fun listClipboard(query: String = "", pinnedOnly: Boolean = false): List<ClipboardEntity> {
        val db = readableDatabase
        val q = query.trim()
        val sql = buildString {
            append(
                "SELECT id, content_type, content, preview, dedup_hmac, source_device_id, is_pinned, hlc_timestamp, created_at, updated_at " +
                    "FROM clipboard_items WHERE deleted_at IS NULL",
            )
            if (pinnedOnly) append(" AND is_pinned = 1")
            if (q.isNotEmpty()) append(" AND (content LIKE ? OR content_type LIKE ?)")
            append(" ORDER BY is_pinned DESC, updated_at DESC LIMIT 100")
        }
        val args = if (q.isNotEmpty()) arrayOf("%$q%", "%$q%") else emptyArray()
        val items = mutableListOf<ClipboardEntity>()
        db.rawQuery(sql, args).use { c ->
            while (c.moveToNext()) {
                items.add(
                    ClipboardEntity(
                        id = c.getString(0),
                        contentType = c.getString(1),
                        content = c.getString(2),
                        preview = c.getString(3),
                        dedupHmac = c.getString(4),
                        sourceDeviceId = c.getString(5),
                        isPinned = c.getInt(6) != 0,
                        hlcTimestamp = c.getString(7),
                        createdAt = c.getLong(8),
                        updatedAt = c.getLong(9),
                    ),
                )
            }
        }
        return items
    }

    fun togglePin(id: String): Boolean {
        val db = writableDatabase
        val current = db.rawQuery(
            "SELECT is_pinned FROM clipboard_items WHERE id = ? AND deleted_at IS NULL",
            arrayOf(id),
        ).use { if (it.moveToFirst()) it.getInt(0) != 0 else return false }
        val next = !current
        val now = System.currentTimeMillis()
        val cv = ContentValues().apply {
            put("is_pinned", if (next) 1 else 0)
            put("hlc_timestamp", ContentTools.nextHlc(now))
            put("updated_at", now)
        }
        db.update("clipboard_items", cv, "id = ?", arrayOf(id))
        queueOutbox(db, id, 1, now)
        return next
    }

    fun deleteClipboard(id: String) {
        val db = writableDatabase
        val now = System.currentTimeMillis()
        val cv = ContentValues().apply {
            put("deleted_at", now)
            put("hlc_timestamp", ContentTools.nextHlc(now))
            put("updated_at", now)
        }
        db.update("clipboard_items", cv, "id = ?", arrayOf(id))
        queueOutbox(db, id, 1, now)
    }

    fun saveSnippet(
        title: String,
        keyword: String?,
        body: String,
        language: String = "shell",
        tags: List<String> = emptyList(),
    ): SnippetEntity {
        val db = writableDatabase
        val now = System.currentTimeMillis()
        val id = UUID.randomUUID().toString()
        val hlc = ContentTools.nextHlc(now)
        val normKw = keyword?.trim()?.takeIf { it.isNotEmpty() }?.let {
            if (it.startsWith("!")) it else "!$it"
        }
        val cleanTags = tags.map { it.trim().lowercase() }.filter { it.isNotEmpty() }.distinct()
        val tagsJson = JSONArray(cleanTags).toString()
        val cv = ContentValues().apply {
            put("id", id)
            put("title", title.trim())
            put("keyword", normKw)
            put("body", body.trim())
            put("language", language)
            put("tags_json", tagsJson)
            put("use_count", 0)
            put("hlc_timestamp", hlc)
            put("created_at", now)
            put("updated_at", now)
        }
        db.insertWithOnConflict("snippets", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
        queueOutbox(db, id, 2, now)
        return SnippetEntity(id, title.trim(), normKw, body.trim(), language, cleanTags, 0, hlc, now, now)
    }

    fun listSnippets(query: String = "", tag: String? = null): List<SnippetEntity> {
        val db = readableDatabase
        val q = query.trim()
        val list = mutableListOf<SnippetEntity>()
        db.rawQuery(
            "SELECT id, title, keyword, body, language, tags_json, use_count, hlc_timestamp, created_at, updated_at " +
                "FROM snippets WHERE deleted_at IS NULL ORDER BY use_count DESC, updated_at DESC",
            emptyArray(),
        ).use { c ->
            while (c.moveToNext()) {
                val arr = runCatching { JSONArray(c.getString(5)) }.getOrNull()
                val tags = buildList {
                    if (arr != null) {
                        for (i in 0 until arr.length()) add(arr.getString(i))
                    }
                }
                val item = SnippetEntity(
                    id = c.getString(0),
                    title = c.getString(1),
                    keyword = c.getString(2),
                    body = c.getString(3),
                    language = c.getString(4),
                    tags = tags,
                    useCount = c.getInt(6),
                    hlcTimestamp = c.getString(7),
                    createdAt = c.getLong(8),
                    updatedAt = c.getLong(9),
                )
                val tagOk = tag.isNullOrBlank() || item.tags.any { it.equals(tag, ignoreCase = true) }
                val queryOk = q.isEmpty() ||
                    item.title.contains(q, ignoreCase = true) ||
                    (item.keyword?.contains(q, ignoreCase = true) == true) ||
                    item.body.contains(q, ignoreCase = true)
                if (tagOk && queryOk) list.add(item)
            }
        }
        return list
    }

    fun useSnippet(id: String, currentClipboard: String): String? {
        val db = writableDatabase
        val body = db.rawQuery(
            "SELECT body, use_count FROM snippets WHERE id = ? AND deleted_at IS NULL",
            arrayOf(id),
        ).use { if (it.moveToFirst()) Pair(it.getString(0), it.getInt(1)) else null } ?: return null
        val now = System.currentTimeMillis()
        val cv = ContentValues().apply {
            put("use_count", body.second + 1)
            put("hlc_timestamp", ContentTools.nextHlc(now))
            put("updated_at", now)
        }
        db.update("snippets", cv, "id = ?", arrayOf(id))
        queueOutbox(db, id, 2, now)
        return ContentTools.expandTemplate(body.first, currentClipboard, now)
    }

    fun pendingOutbox(): List<OutboxRecord> {
        val db = readableDatabase
        val pairs = mutableListOf<Pair<String, Short>>()
        db.rawQuery("SELECT entity_id, entity_kind FROM sync_outbox ORDER BY queued_at ASC LIMIT 100", emptyArray())
            .use { c ->
                while (c.moveToNext()) pairs.add(c.getString(0) to c.getShort(1))
            }
        val out = mutableListOf<OutboxRecord>()
        for ((entityId, kind) in pairs) {
            if (kind.toInt() == 1) {
                db.rawQuery(
                    "SELECT content_type, content, preview, dedup_hmac, source_device_id, is_pinned, hlc_timestamp, created_at, updated_at, deleted_at " +
                        "FROM clipboard_items WHERE id = ?",
                    arrayOf(entityId),
                ).use { c ->
                    if (c.moveToFirst()) {
                        val json = JSONObject().apply {
                            put("content_type", c.getString(0))
                            put("content", c.getString(1))
                            put("preview", c.getString(2))
                            put("dedup_hmac", c.getString(3))
                            put("source_device_id", c.getString(4))
                            put("is_pinned", c.getInt(5) != 0)
                            put("created_at", c.getLong(7))
                            put("updated_at", c.getLong(8))
                        }.toString()
                        out.add(OutboxRecord(entityId, kind, c.getString(6), !c.isNull(9), json))
                    }
                }
            } else if (kind.toInt() == 2) {
                db.rawQuery(
                    "SELECT title, keyword, body, language, tags_json, use_count, hlc_timestamp, created_at, updated_at, deleted_at " +
                        "FROM snippets WHERE id = ?",
                    arrayOf(entityId),
                ).use { c ->
                    if (c.moveToFirst()) {
                        val json = JSONObject().apply {
                            put("title", c.getString(0))
                            put("keyword", if (c.isNull(1)) JSONObject.NULL else c.getString(1))
                            put("body", c.getString(2))
                            put("language", c.getString(3))
                            put("tags", JSONArray(c.getString(4)))
                            put("use_count", c.getInt(5))
                            put("created_at", c.getLong(7))
                            put("updated_at", c.getLong(8))
                        }.toString()
                        out.add(OutboxRecord(entityId, kind, c.getString(6), !c.isNull(9), json))
                    }
                }
            }
        }
        return out
    }

    fun acknowledgeOutbox(ids: List<String>) {
        val db = writableDatabase
        for (id in ids) {
            db.delete("sync_outbox", "entity_id = ?", arrayOf(id))
        }
    }

    fun applyRemoteEntity(
        entityId: String,
        entityKind: Short,
        remoteHlc: String,
        isDeleted: Boolean,
        payloadJson: String,
    ): Boolean {
        val db = writableDatabase
        val table = if (entityKind.toInt() == 1) "clipboard_items" else "snippets"
        val localHlc = db.rawQuery("SELECT hlc_timestamp FROM $table WHERE id = ?", arrayOf(entityId)).use {
            if (it.moveToFirst()) it.getString(0) else null
        }
        if (localHlc != null) {
            val shouldApply = runCatching { uniffiHlcShouldApplyRemote(localHlc, remoteHlc) }
                .getOrElse { remoteHlc > localHlc }
            if (!shouldApply) return false
        }

        val obj = JSONObject(payloadJson)
        val now = System.currentTimeMillis()
        if (entityKind.toInt() == 1) {
            val cv = ContentValues().apply {
                put("id", entityId)
                put("content_type", obj.optString("content_type", "text"))
                put("content", obj.optString("content", ""))
                put("preview", obj.optString("preview", ""))
                put("dedup_hmac", obj.optString("dedup_hmac", ""))
                put("source_device_id", obj.optString("source_device_id", "fedora-kde"))
                put("is_pinned", if (obj.optBoolean("is_pinned", false)) 1 else 0)
                put("hlc_timestamp", remoteHlc)
                put("created_at", obj.optLong("created_at", now))
                put("updated_at", obj.optLong("updated_at", now))
                if (isDeleted) put("deleted_at", now) else putNull("deleted_at")
            }
            db.insertWithOnConflict("clipboard_items", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
            return true
        } else if (entityKind.toInt() == 2) {
            val cv = ContentValues().apply {
                put("id", entityId)
                put("title", obj.optString("title", "Snippet"))
                val kw = if (obj.isNull("keyword") || isDeleted) null else obj.optString("keyword")
                if (kw != null) put("keyword", kw) else putNull("keyword")
                put("body", obj.optString("body", ""))
                put("language", obj.optString("language", "plaintext"))
                put("tags_json", (obj.optJSONArray("tags") ?: JSONArray()).toString())
                put("use_count", obj.optInt("use_count", 0))
                put("hlc_timestamp", remoteHlc)
                put("created_at", obj.optLong("created_at", now))
                put("updated_at", obj.optLong("updated_at", now))
                if (isDeleted) put("deleted_at", now) else putNull("deleted_at")
            }
            db.insertWithOnConflict("snippets", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
            return true
        }
        return false
    }

    private fun queueOutbox(db: SQLiteDatabase, entityId: String, kind: Int, now: Long) {
        val cv = ContentValues().apply {
            put("entity_id", entityId)
            put("entity_kind", kind)
            put("queued_at", now)
        }
        db.insertWithOnConflict("sync_outbox", null, cv, SQLiteDatabase.CONFLICT_REPLACE)
    }
}
