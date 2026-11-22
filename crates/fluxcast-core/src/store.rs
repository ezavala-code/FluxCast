use crate::{
    classifier::{classify, dedup_hmac, ContentType},
    templates::expand,
};
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher, Utf32String,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    path::Path,
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const SCHEMA: &str = r#"
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
);

CREATE VIRTUAL TABLE IF NOT EXISTS clipboard_fts USING fts5(
    content,
    content='clipboard_items',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS clipboard_items_ai AFTER INSERT ON clipboard_items BEGIN
    INSERT INTO clipboard_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS clipboard_items_ad AFTER DELETE ON clipboard_items BEGIN
    INSERT INTO clipboard_fts(clipboard_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS clipboard_items_au AFTER UPDATE ON clipboard_items BEGIN
    INSERT INTO clipboard_fts(clipboard_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
    INSERT INTO clipboard_fts(rowid, content) VALUES (new.rowid, new.content);
END;

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
);

CREATE TABLE IF NOT EXISTS sync_outbox (
    entity_id TEXT PRIMARY KEY,
    entity_kind INTEGER NOT NULL,
    queued_at INTEGER NOT NULL
);
"#;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClipboardItem {
    pub id: String,
    pub content_type: ContentType,
    pub content: String,
    pub preview: String,
    pub dedup_hmac: String,
    pub source_device_id: String,
    pub is_pinned: bool,
    pub hlc_timestamp: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub formatted: Option<String>,
    pub language: Option<&'static str>,
    pub color_swatch: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub keyword: Option<String>,
    pub body: String,
    pub language: String,
    pub tags: Vec<String>,
    pub use_count: i64,
    pub hlc_timestamp: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnippetInput {
    pub id: Option<String>,
    pub title: String,
    pub keyword: Option<String>,
    pub body: String,
    pub language: Option<String>,
    pub tags: Vec<String>,
}

pub struct Store {
    conn: Mutex<Connection>,
    device_id: String,
    hlc_counter: AtomicU32,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
            device_id: "fedora-kde".into(),
            hlc_counter: AtomicU32::new(0),
        })
    }

    pub fn seed_default_snippets(&self) -> Result<(), String> {
        let count: i64 = {
            let conn = self.conn.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM snippets", [], |row| row.get(0))
                .map_err(|e| e.to_string())?
        };
        if count > 0 {
            return Ok(());
        }
        for (title, keyword, language, tags, body) in [
            (
                "Limpiar contenedores e imágenes Docker/Podman",
                Some("!dprune"),
                "shell",
                vec!["docker", "fedora"],
                "docker system prune -af --volumes",
            ),
            (
                "Logs recientes del servicio de usuario en Fedora",
                Some("!klog"),
                "shell",
                vec!["fedora", "systemd"],
                "journalctl --user -n 100 --no-pager -u {{clipboard}}",
            ),
            (
                "Commit rápido con fecha e identificador",
                Some("!gstamp"),
                "git",
                vec!["git"],
                "git commit -m \"chore({{date:YYYY-MM-DD}}): {{clipboard}} [{{uuid}}]\"",
            ),
            (
                "Consulta SQL de inspección rápida",
                Some("!sqltop"),
                "sql",
                vec!["sql"],
                "SELECT * FROM {{clipboard}} ORDER BY created_at DESC LIMIT 25;",
            ),
        ] {
            self.save_snippet(SnippetInput {
                id: None,
                title: title.into(),
                keyword: keyword.map(str::to_owned),
                body: body.into(),
                language: Some(language.into()),
                tags: tags.into_iter().map(str::to_owned).collect(),
            })?;
        }
        Ok(())
    }

    fn next_hlc(&self, now_ms: i64) -> String {
        let seq = self.hlc_counter.fetch_add(1, Ordering::Relaxed);
        format!("{now_ms:015}-{seq:04x}-{}", self.device_id)
    }

    pub fn record_clipboard(&self, content: &str) -> Result<Option<ClipboardItem>, String> {
        if content.trim().is_empty() {
            return Ok(None);
        }
        let info = classify(content);
        let hmac = dedup_hmac(content);
        let now = unix_ms();
        let hlc = self.next_hlc(now);
        let conn = self.conn.lock().unwrap();

        let existing: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT id, is_pinned, created_at FROM clipboard_items
                 WHERE dedup_hmac = ?1 AND deleted_at IS NULL
                 ORDER BY updated_at DESC LIMIT 1",
                params![hmac],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let (id, is_pinned, created_at) = if let Some((id, pinned, created_at)) = existing {
            conn.execute(
                "UPDATE clipboard_items
                 SET content = ?1, preview = ?2, content_type = ?3, hlc_timestamp = ?4, updated_at = ?5
                 WHERE id = ?6",
                params![
                    content,
                    info.preview,
                    info.content_type.as_str(),
                    hlc,
                    now,
                    id
                ],
            )
            .map_err(|e| e.to_string())?;
            (id, pinned != 0, created_at)
        } else {
            let id = Uuid::now_v7().to_string();
            conn.execute(
                "INSERT INTO clipboard_items (
                    id, content_type, content, preview, dedup_hmac, source_device_id,
                    is_pinned, hlc_timestamp, created_at, updated_at, deleted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?8, NULL)",
                params![
                    id,
                    info.content_type.as_str(),
                    content,
                    info.preview,
                    hmac,
                    self.device_id,
                    hlc,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
            (id, false, now)
        };

        queue_outbox(&conn, &id, 1, now)?;
        Ok(Some(ClipboardItem {
            id,
            content_type: info.content_type,
            content: content.to_owned(),
            preview: info.preview,
            dedup_hmac: hmac,
            source_device_id: self.device_id.clone(),
            is_pinned,
            hlc_timestamp: hlc,
            created_at,
            updated_at: now,
            formatted: info.formatted,
            language: info.language,
            color_swatch: info.color_swatch,
        }))
    }

    pub fn list_clipboard(
        &self,
        query: &str,
        pinned_only: bool,
        limit: usize,
    ) -> Result<Vec<ClipboardItem>, String> {
        let conn = self.conn.lock().unwrap();
        let trimmed = query.trim();
        let limit_i64 = limit as i64;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ClipboardItem> {
            let content_type_raw: String = row.get(1)?;
            let content: String = row.get(2)?;
            let info = classify(&content);
            Ok(ClipboardItem {
                id: row.get(0)?,
                content_type: ContentType::from_db(&content_type_raw),
                content,
                preview: row.get(3)?,
                dedup_hmac: row.get(4)?,
                source_device_id: row.get(5)?,
                is_pinned: row.get::<_, i64>(6)? != 0,
                hlc_timestamp: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                formatted: info.formatted,
                language: info.language,
                color_swatch: info.color_swatch,
            })
        };

        if let Some(fts_query) = build_fts_query(trimmed) {
            let sql = "SELECT c.id, c.content_type, c.content, c.preview, c.dedup_hmac,
                              c.source_device_id, c.is_pinned, c.hlc_timestamp, c.created_at, c.updated_at
                       FROM clipboard_fts f
                       JOIN clipboard_items c ON c.rowid = f.rowid
                       WHERE clipboard_fts MATCH ?1
                         AND c.deleted_at IS NULL
                         AND (?2 = 0 OR c.is_pinned = 1)
                       ORDER BY c.is_pinned DESC, c.updated_at DESC
                       LIMIT ?3";
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(
                    params![fts_query, i64::from(pinned_only), limit_i64],
                    map_row,
                )
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }

        if !trimmed.is_empty() {
            let pattern = format!("%{trimmed}%");
            let sql = "SELECT id, content_type, content, preview, dedup_hmac,
                              source_device_id, is_pinned, hlc_timestamp, created_at, updated_at
                       FROM clipboard_items
                       WHERE deleted_at IS NULL
                         AND (?1 = 0 OR is_pinned = 1)
                         AND (content LIKE ?2 OR content_type LIKE ?2)
                       ORDER BY is_pinned DESC, updated_at DESC
                       LIMIT ?3";
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![i64::from(pinned_only), pattern, limit_i64], map_row)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            return Ok(rows);
        }

        let sql = "SELECT id, content_type, content, preview, dedup_hmac,
                          source_device_id, is_pinned, hlc_timestamp, created_at, updated_at
                   FROM clipboard_items
                   WHERE deleted_at IS NULL
                     AND (?1 = 0 OR is_pinned = 1)
                   ORDER BY is_pinned DESC, updated_at DESC
                   LIMIT ?2";
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![i64::from(pinned_only), limit_i64], map_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn get_clipboard_item(&self, id: &str) -> Result<Option<ClipboardItem>, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, content_type, content, preview, dedup_hmac,
                    source_device_id, is_pinned, hlc_timestamp, created_at, updated_at
             FROM clipboard_items
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| {
                let content_type_raw: String = row.get(1)?;
                let content: String = row.get(2)?;
                let info = classify(&content);
                Ok(ClipboardItem {
                    id: row.get(0)?,
                    content_type: ContentType::from_db(&content_type_raw),
                    content,
                    preview: row.get(3)?,
                    dedup_hmac: row.get(4)?,
                    source_device_id: row.get(5)?,
                    is_pinned: row.get::<_, i64>(6)? != 0,
                    hlc_timestamp: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    formatted: info.formatted,
                    language: info.language,
                    color_swatch: info.color_swatch,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn latest_clipboard_text(&self) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT content FROM clipboard_items WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn toggle_pin_clipboard(&self, id: &str) -> Result<bool, String> {
        let now = unix_ms();
        let hlc = self.next_hlc(now);
        let conn = self.conn.lock().unwrap();
        let current: i64 = conn
            .query_row(
                "SELECT is_pinned FROM clipboard_items WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| row.get(0),
            )
            .map_err(|_| "Elemento de portapapeles no encontrado".to_string())?;
        let next = i64::from(current == 0);
        conn.execute(
            "UPDATE clipboard_items SET is_pinned = ?1, hlc_timestamp = ?2, updated_at = ?3 WHERE id = ?4",
            params![next, hlc, now, id],
        )
        .map_err(|e| e.to_string())?;
        queue_outbox(&conn, id, 1, now)?;
        Ok(next != 0)
    }

    pub fn delete_clipboard_item(&self, id: &str) -> Result<(), String> {
        let now = unix_ms();
        let hlc = self.next_hlc(now);
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute(
                "UPDATE clipboard_items SET deleted_at = ?1, hlc_timestamp = ?2, updated_at = ?1 WHERE id = ?3 AND deleted_at IS NULL",
                params![now, hlc, id],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Elemento de portapapeles no encontrado".into());
        }
        queue_outbox(&conn, id, 1, now)?;
        Ok(())
    }

    pub fn clear_unpinned_clipboard(&self) -> Result<usize, String> {
        let now = unix_ms();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM clipboard_items WHERE deleted_at IS NULL AND is_pinned = 0")
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for id in &ids {
            let hlc = self.next_hlc(now);
            conn.execute(
                "UPDATE clipboard_items SET deleted_at = ?1, hlc_timestamp = ?2, updated_at = ?1 WHERE id = ?3",
                params![now, hlc, id],
            )
            .map_err(|e| e.to_string())?;
            queue_outbox(&conn, id, 1, now)?;
        }
        Ok(ids.len())
    }

    pub fn save_snippet(&self, input: SnippetInput) -> Result<Snippet, String> {
        let title = input.title.trim();
        let body = input.body.trim();
        if title.is_empty() || body.is_empty() {
            return Err("El título y el cuerpo del snippet son obligatorios".into());
        }
        let keyword = normalize_keyword(input.keyword.as_deref());
        let language = input
            .language
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("plaintext")
            .to_owned();
        let mut tags: Vec<String> = input
            .tags
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        tags.sort();
        tags.dedup();
        let tags_json = serde_json::to_string(&tags).map_err(|e| e.to_string())?;

        let now = unix_ms();
        let hlc = self.next_hlc(now);
        let conn = self.conn.lock().unwrap();

        if let Some(kw) = &keyword {
            let conflict: Option<String> = conn
                .query_row(
                    "SELECT id FROM snippets WHERE keyword = ?1 AND (?2 IS NULL OR id != ?2)",
                    params![kw, input.id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(other_id) = conflict {
                conn.execute(
                    "UPDATE snippets SET keyword = NULL WHERE id = ?1 AND deleted_at IS NOT NULL",
                    params![other_id],
                )
                .map_err(|e| e.to_string())?;
            }
        }

        let (id, use_count, created_at) = if let Some(id) =
            input.id.filter(|s| !s.trim().is_empty())
        {
            let existing: (i64, i64) = conn
                .query_row(
                    "SELECT use_count, created_at FROM snippets WHERE id = ?1 AND deleted_at IS NULL",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| "Snippet no encontrado".to_string())?;
            conn.execute(
                "UPDATE snippets
                 SET title = ?1, keyword = ?2, body = ?3, language = ?4, tags_json = ?5,
                     hlc_timestamp = ?6, updated_at = ?7
                 WHERE id = ?8",
                params![title, keyword, body, language, tags_json, hlc, now, id],
            )
            .map_err(|e| format!("No se pudo guardar el snippet (¿alias duplicado?): {e}"))?;
            (id, existing.0, existing.1)
        } else {
            let id = Uuid::now_v7().to_string();
            conn.execute(
                "INSERT INTO snippets (
                    id, title, keyword, body, language, tags_json,
                    use_count, hlc_timestamp, created_at, updated_at, deleted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?8, NULL)",
                params![id, title, keyword, body, language, tags_json, hlc, now],
            )
            .map_err(|e| format!("No se pudo crear el snippet (¿alias duplicado?): {e}"))?;
            (id, 0, now)
        };

        queue_outbox(&conn, &id, 2, now)?;
        Ok(Snippet {
            id,
            title: title.to_owned(),
            keyword,
            body: body.to_owned(),
            language,
            tags,
            use_count,
            hlc_timestamp: hlc,
            created_at,
            updated_at: now,
        })
    }

    pub fn list_snippets(&self, query: &str, tag: Option<&str>) -> Result<Vec<Snippet>, String> {
        let all = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, keyword, body, language, tags_json,
                            use_count, hlc_timestamp, created_at, updated_at
                     FROM snippets
                     WHERE deleted_at IS NULL
                     ORDER BY use_count DESC, updated_at DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], map_snippet_row)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };

        let tag_filter = tag.map(str::trim).filter(|t| !t.is_empty());
        let filtered: Vec<Snippet> = all
            .into_iter()
            .filter(|s| {
                tag_filter
                    .is_none_or(|target| s.tags.iter().any(|t| t.eq_ignore_ascii_case(target)))
            })
            .collect();

        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(filtered);
        }

        let pattern = Pattern::new(
            trimmed,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut ranked: Vec<_> = filtered
            .into_iter()
            .filter_map(|snippet| {
                let exact_kw = snippet
                    .keyword
                    .as_deref()
                    .is_some_and(|kw| kw.eq_ignore_ascii_case(trimmed));
                let composite = format!(
                    "{} {} {} {}",
                    snippet.title,
                    snippet.keyword.as_deref().unwrap_or_default(),
                    snippet.tags.join(" "),
                    snippet.body
                );
                let haystack = Utf32String::from(composite.as_str());
                let fuzzy_score = pattern.score(haystack.slice(..), &mut matcher)?;
                let frecency_bonus = (snippet.use_count.clamp(0, 500) as u32) * 8;
                Some((
                    !exact_kw,
                    Reverse(fuzzy_score.saturating_add(frecency_bonus)),
                    Reverse(snippet.use_count),
                    Reverse(snippet.updated_at),
                    snippet,
                ))
            })
            .collect();
        ranked.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        Ok(ranked.into_iter().map(|(_, _, _, _, s)| s).collect())
    }

    pub fn find_snippet_by_keyword(&self, keyword: &str) -> Result<Option<Snippet>, String> {
        let Some(norm) = normalize_keyword(Some(keyword)) else {
            return Ok(None);
        };
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, title, keyword, body, language, tags_json,
                    use_count, hlc_timestamp, created_at, updated_at
             FROM snippets
             WHERE LOWER(keyword) = LOWER(?1) AND deleted_at IS NULL",
            params![norm],
            map_snippet_row,
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn use_snippet(&self, id: &str, current_clipboard: &str) -> Result<String, String> {
        let now = unix_ms();
        let hlc = self.next_hlc(now);
        let conn = self.conn.lock().unwrap();
        let body: String = conn
            .query_row(
                "SELECT body FROM snippets WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| row.get(0),
            )
            .map_err(|_| "Snippet no encontrado".to_string())?;
        conn.execute(
            "UPDATE snippets
             SET use_count = use_count + 1, hlc_timestamp = ?1, updated_at = ?2
             WHERE id = ?3",
            params![hlc, now, id],
        )
        .map_err(|e| e.to_string())?;
        queue_outbox(&conn, id, 2, now)?;
        Ok(expand(&body, current_clipboard))
    }

    pub fn delete_snippet(&self, id: &str) -> Result<(), String> {
        let now = unix_ms();
        let hlc = self.next_hlc(now);
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute(
                "UPDATE snippets
                 SET deleted_at = ?1, keyword = NULL, hlc_timestamp = ?2, updated_at = ?1
                 WHERE id = ?3 AND deleted_at IS NULL",
                params![now, hlc, id],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Snippet no encontrado".into());
        }
        queue_outbox(&conn, id, 2, now)?;
        Ok(())
    }

    pub fn outbox_count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count as usize)
    }
}

fn map_snippet_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snippet> {
    let tags_raw: String = row.get(5)?;
    let tags: Vec<String> = serde_json::from_str(&tags_raw).unwrap_or_default();
    Ok(Snippet {
        id: row.get(0)?,
        title: row.get(1)?,
        keyword: row.get(2)?,
        body: row.get(3)?,
        language: row.get(4)?,
        tags,
        use_count: row.get(6)?,
        hlc_timestamp: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn normalize_keyword(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('!') {
        (trimmed.len() > 1).then(|| trimmed.to_owned())
    } else {
        Some(format!("!{trimmed}"))
    }
}

fn build_fts_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect();
    (!tokens.is_empty()).then(|| tokens.join(" AND "))
}

fn queue_outbox(
    conn: &Connection,
    entity_id: &str,
    entity_kind: i64,
    now: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_outbox (entity_id, entity_kind, queued_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(entity_id) DO UPDATE SET queued_at = excluded.queued_at",
        params![entity_id, entity_kind, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
