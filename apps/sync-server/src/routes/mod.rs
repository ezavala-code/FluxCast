use crate::{
    auth::{generate_pairing_token, unix_secs, Claims, JwtSigner},
    ws::{ws_handler, WsHub},
};
use axum::{
    extract::{Query, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use fluxcast_protocol::{
    hlc_is_newer, AuthSessionResponse, ClaimPairingRequest, DeviceInfo, EncryptedBlob,
    PairingTokenResponse, PullResponse, PushBlob, PushRequest, PushResponse, RegisterDeviceRequest,
    PAIRING_TTL_SECS,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub store: Storage,
    pub jwt: Arc<JwtSigner>,
    pub hub: Arc<WsHub>,
}

#[derive(Clone)]
pub enum Storage {
    Memory(Arc<Mutex<MemoryStore>>),
    Postgres(PgPool),
}

#[derive(Default)]
pub struct MemoryStore {
    next_seq: i64,
    devices: Vec<(Uuid, DeviceInfo)>,
    pairings: HashMap<String, (Uuid, i64, bool)>,
    blobs: HashMap<(Uuid, Uuid), EncryptedBlob>,
}

impl Storage {
    pub fn memory() -> Self {
        Self::Memory(Arc::new(Mutex::new(MemoryStore::default())))
    }

    pub async fn connect_postgres(database_url: &str) -> Result<Self, String> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|e| format!("No se pudo conectar a PostgreSQL: {e}"))?;
        let schema = include_str!("../../migrations/0001_init.sql");
        for statement in schema.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&pool)
                    .await
                    .map_err(|e| format!("Fallo en migración SQL: {e}"))?;
            }
        }
        Ok(Self::Postgres(pool))
    }

    pub async fn register_device(
        &self,
        user_id: Uuid,
        device_name: &str,
        platform: &str,
    ) -> Result<DeviceInfo, String> {
        let device = DeviceInfo {
            device_id: Uuid::now_v7(),
            device_name: device_name.trim().to_owned(),
            platform: platform.trim().to_owned(),
            created_at: unix_secs(),
        };
        match self {
            Self::Memory(mem) => {
                mem.lock().unwrap().devices.push((user_id, device.clone()));
            }
            Self::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO sync_devices (device_id, user_id, device_name, platform, created_at)
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(device.device_id)
                .bind(user_id)
                .bind(&device.device_name)
                .bind(&device.platform)
                .bind(device.created_at)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
        Ok(device)
    }

    pub async fn list_devices(&self, user_id: Uuid) -> Result<Vec<DeviceInfo>, String> {
        match self {
            Self::Memory(mem) => {
                let guard = mem.lock().unwrap();
                Ok(guard
                    .devices
                    .iter()
                    .filter(|(u, _)| *u == user_id)
                    .map(|(_, d)| d.clone())
                    .collect())
            }
            Self::Postgres(pool) => {
                let rows = sqlx::query(
                    "SELECT device_id, device_name, platform, created_at
                     FROM sync_devices WHERE user_id = $1 ORDER BY created_at ASC",
                )
                .bind(user_id)
                .fetch_all(pool)
                .await
                .map_err(|e| e.to_string())?;
                Ok(rows
                    .into_iter()
                    .map(|r| DeviceInfo {
                        device_id: r.get(0),
                        device_name: r.get(1),
                        platform: r.get(2),
                        created_at: r.get(3),
                    })
                    .collect())
            }
        }
    }

    pub async fn create_pairing_token(
        &self,
        user_id: Uuid,
    ) -> Result<PairingTokenResponse, String> {
        let token = generate_pairing_token();
        let expires_at = unix_secs() + PAIRING_TTL_SECS;
        match self {
            Self::Memory(mem) => {
                mem.lock()
                    .unwrap()
                    .pairings
                    .insert(token.clone(), (user_id, expires_at, false));
            }
            Self::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO pairing_tokens (token, user_id, expires_at, consumed)
                     VALUES ($1, $2, $3, FALSE)",
                )
                .bind(&token)
                .bind(user_id)
                .bind(expires_at)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
        Ok(PairingTokenResponse {
            pairing_token: token,
            expires_at,
        })
    }

    pub async fn claim_pairing_token(
        &self,
        token: &str,
        device_name: &str,
        platform: &str,
    ) -> Result<(Uuid, DeviceInfo), String> {
        let now = unix_secs();
        let user_id = match self {
            Self::Memory(mem) => {
                let mut guard = mem.lock().unwrap();
                let (uid, expires_at, consumed) = guard
                    .pairings
                    .get_mut(token)
                    .ok_or_else(|| "Token de emparejamiento inválido".to_string())?;
                if *consumed || *expires_at <= now {
                    return Err("Token de emparejamiento expirado o ya utilizado".into());
                }
                *consumed = true;
                *uid
            }
            Self::Postgres(pool) => {
                let row = sqlx::query(
                    "UPDATE pairing_tokens
                     SET consumed = TRUE
                     WHERE token = $1 AND consumed = FALSE AND expires_at > $2
                     RETURNING user_id",
                )
                .bind(token)
                .bind(now)
                .fetch_optional(pool)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| {
                    "Token de emparejamiento inválido, expirado o ya usado".to_string()
                })?;
                row.get(0)
            }
        };
        let device = self.register_device(user_id, device_name, platform).await?;
        Ok((user_id, device))
    }

    pub async fn push_blobs(
        &self,
        user_id: Uuid,
        blobs: &[PushBlob],
        hub: &WsHub,
    ) -> Result<PushResponse, String> {
        let mut accepted = 0usize;
        let mut latest_seq = 0i64;

        match self {
            Self::Memory(mem) => {
                let mut guard = mem.lock().unwrap();
                latest_seq = guard.next_seq;
                for incoming in blobs {
                    let key = (user_id, incoming.entity_id);
                    let should_write = match guard.blobs.get(&key) {
                        Some(existing) => {
                            hlc_is_newer(&incoming.hlc_timestamp, &existing.hlc_timestamp)
                        }
                        None => true,
                    };
                    if should_write {
                        guard.next_seq += 1;
                        let seq = guard.next_seq;
                        latest_seq = seq;
                        let record = EncryptedBlob {
                            seq,
                            user_id,
                            entity_id: incoming.entity_id,
                            entity_kind: incoming.entity_kind,
                            device_id: incoming.device_id,
                            hlc_timestamp: incoming.hlc_timestamp.clone(),
                            dedup_hmac: incoming.dedup_hmac.clone(),
                            nonce: incoming.nonce.clone(),
                            ciphertext: incoming.ciphertext.clone(),
                            is_deleted: incoming.is_deleted,
                        };
                        guard.blobs.insert(key, record.clone());
                        hub.broadcast(&record);
                        accepted += 1;
                    }
                }
            }
            Self::Postgres(pool) => {
                for incoming in blobs {
                    let existing_hlc: Option<String> = sqlx::query_scalar(
                        "SELECT hlc_timestamp FROM sync_blobs WHERE user_id = $1 AND entity_id = $2",
                    )
                    .bind(user_id)
                    .bind(incoming.entity_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?;

                    if existing_hlc
                        .as_deref()
                        .is_some_and(|curr| !hlc_is_newer(&incoming.hlc_timestamp, curr))
                    {
                        continue;
                    }

                    let seq: i64 = sqlx::query_scalar(
                        "INSERT INTO sync_blobs (
                            user_id, entity_id, entity_kind, device_id, hlc_timestamp,
                            dedup_hmac, nonce, ciphertext, is_deleted
                         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                         ON CONFLICT (user_id, entity_id) DO UPDATE SET
                            seq = nextval(pg_get_serial_sequence('sync_blobs', 'seq')),
                            entity_kind = EXCLUDED.entity_kind,
                            device_id = EXCLUDED.device_id,
                            hlc_timestamp = EXCLUDED.hlc_timestamp,
                            dedup_hmac = EXCLUDED.dedup_hmac,
                            nonce = EXCLUDED.nonce,
                            ciphertext = EXCLUDED.ciphertext,
                            is_deleted = EXCLUDED.is_deleted,
                            created_at = NOW()
                         RETURNING seq",
                    )
                    .bind(user_id)
                    .bind(incoming.entity_id)
                    .bind(incoming.entity_kind)
                    .bind(incoming.device_id)
                    .bind(&incoming.hlc_timestamp)
                    .bind(&incoming.dedup_hmac)
                    .bind(&incoming.nonce)
                    .bind(&incoming.ciphertext)
                    .bind(incoming.is_deleted)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| e.to_string())?;

                    latest_seq = latest_seq.max(seq);
                    let record = EncryptedBlob {
                        seq,
                        user_id,
                        entity_id: incoming.entity_id,
                        entity_kind: incoming.entity_kind,
                        device_id: incoming.device_id,
                        hlc_timestamp: incoming.hlc_timestamp.clone(),
                        dedup_hmac: incoming.dedup_hmac.clone(),
                        nonce: incoming.nonce.clone(),
                        ciphertext: incoming.ciphertext.clone(),
                        is_deleted: incoming.is_deleted,
                    };
                    hub.broadcast(&record);
                    accepted += 1;
                }
            }
        }
        Ok(PushResponse {
            accepted,
            latest_seq,
        })
    }

    pub async fn pull_blobs(&self, user_id: Uuid, since_seq: i64) -> Result<PullResponse, String> {
        match self {
            Self::Memory(mem) => {
                let guard = mem.lock().unwrap();
                let mut blobs: Vec<EncryptedBlob> = guard
                    .blobs
                    .values()
                    .filter(|b| b.user_id == user_id && b.seq > since_seq)
                    .cloned()
                    .collect();
                blobs.sort_by_key(|b| b.seq);
                let latest_seq = blobs.last().map(|b| b.seq).unwrap_or(since_seq);
                Ok(PullResponse { blobs, latest_seq })
            }
            Self::Postgres(pool) => {
                let rows = sqlx::query(
                    "SELECT seq, user_id, entity_id, entity_kind, device_id, hlc_timestamp,
                            dedup_hmac, nonce, ciphertext, is_deleted
                     FROM sync_blobs
                     WHERE user_id = $1 AND seq > $2
                     ORDER BY seq ASC
                     LIMIT 500",
                )
                .bind(user_id)
                .bind(since_seq)
                .fetch_all(pool)
                .await
                .map_err(|e| e.to_string())?;

                let blobs: Vec<EncryptedBlob> = rows
                    .into_iter()
                    .map(|r| EncryptedBlob {
                        seq: r.get(0),
                        user_id: r.get(1),
                        entity_id: r.get(2),
                        entity_kind: r.get(3),
                        device_id: r.get(4),
                        hlc_timestamp: r.get(5),
                        dedup_hmac: r.get(6),
                        nonce: r.get(7),
                        ciphertext: r.get(8),
                        is_deleted: r.get(9),
                    })
                    .collect();
                let latest_seq = blobs.last().map(|b| b.seq).unwrap_or(since_seq);
                Ok(PullResponse { blobs, latest_seq })
            }
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(register_device))
        .route("/api/v1/devices/pair", post(create_pair_token))
        .route("/api/v1/devices/claim", post(claim_pair_token))
        .route("/api/v1/devices", get(list_devices))
        .route("/api/v1/sync/push", post(push_sync))
        .route("/api/v1/sync/pull", get(pull_sync))
        .route("/api/v1/ws", get(ws_handler))
        .with_state(state)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    zero_knowledge: bool,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        zero_knowledge: true,
    })
}

fn authenticate(headers: &HeaderMap, jwt: &JwtSigner) -> Result<Claims, (StatusCode, String)> {
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Falta cabecera Authorization".into(),
            )
        })?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Se esperaba Bearer token".into()))?;
    jwt.verify(token)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_owned()))
}

async fn register_device(
    State(state): State<AppState>,
    Json(req): Json<RegisterDeviceRequest>,
) -> Result<Json<AuthSessionResponse>, (StatusCode, String)> {
    let user_id = req.user_id.unwrap_or_else(Uuid::now_v7);
    let device = state
        .store
        .register_device(user_id, &req.device_name, &req.platform)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let access_token = state.jwt.issue(user_id, device.device_id);
    Ok(Json(AuthSessionResponse {
        user_id,
        device_id: device.device_id,
        access_token,
    }))
}

async fn create_pair_token(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PairingTokenResponse>, (StatusCode, String)> {
    let claims = authenticate(&headers, &state.jwt)?;
    let resp = state
        .store
        .create_pairing_token(claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

async fn claim_pair_token(
    State(state): State<AppState>,
    Json(req): Json<ClaimPairingRequest>,
) -> Result<Json<AuthSessionResponse>, (StatusCode, String)> {
    let (user_id, device) = state
        .store
        .claim_pairing_token(&req.pairing_token, &req.device_name, &req.platform)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let access_token = state.jwt.issue(user_id, device.device_id);
    Ok(Json(AuthSessionResponse {
        user_id,
        device_id: device.device_id,
        access_token,
    }))
}

async fn list_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DeviceInfo>>, (StatusCode, String)> {
    let claims = authenticate(&headers, &state.jwt)?;
    let devices = state
        .store
        .list_devices(claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(devices))
}

async fn push_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, (StatusCode, String)> {
    let claims = authenticate(&headers, &state.jwt)?;
    let resp = state
        .store
        .push_blobs(claims.sub, &req.blobs, &state.hub)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[derive(Deserialize)]
struct PullQuery {
    #[serde(default)]
    since_seq: i64,
}

async fn pull_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PullQuery>,
) -> Result<Json<PullResponse>, (StatusCode, String)> {
    let claims = authenticate(&headers, &state.jwt)?;
    let resp = state
        .store
        .pull_blobs(claims.sub, query.since_seq)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}
