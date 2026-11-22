//! Cliente de sincronización delta E2EE (Push/Pull + WebSocket en tiempo real) y generador QR.

use crate::keyring::{derive_and_store_master_key, load_or_create_master_key};
use fluxcast_core::store::Store;
use fluxcast_crypto::{encode_key_b64, open, seal, MASTER_KEY_LEN};
use fluxcast_protocol::{
    blob_associated_data, AuthSessionResponse, DeviceInfo, PairingQrPayload, PairingTokenResponse,
    PullResponse, PushBlob, PushRequest, PushResponse, RegisterDeviceRequest, WsFrame,
};
use futures_util::StreamExt;
use qrcode::{render::svg, QrCode};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncConfig {
    pub server_url: String,
    pub user_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub access_token: Option<String>,
    pub last_seq: i64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            server_url: "http://127.0.0.1:8787".into(),
            user_id: None,
            device_id: None,
            access_token: None,
            last_seq: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SyncStatus {
    pub server_url: String,
    pub registered: bool,
    pub user_id: Option<String>,
    pub device_id: Option<String>,
    pub last_seq: i64,
    pub pending_outbox: usize,
    pub keyring_backend: String,
    pub key_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PairingQrView {
    pub qr_svg: String,
    pub payload_json: String,
    pub pairing_token: String,
    pub expires_at: i64,
    pub server_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SyncCycleReport {
    pub pushed: usize,
    pub pulled: usize,
    pub last_seq: i64,
}

pub struct SyncWorker {
    config: Mutex<SyncConfig>,
    config_path: PathBuf,
    client: reqwest::Client,
}

impl Default for SyncWorker {
    fn default() -> Self {
        let config_path = gio::glib::user_data_dir().join("fluxcast/sync.json");
        let config = fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .build()
            .unwrap_or_default();
        Self {
            config: Mutex::new(config),
            config_path,
            client,
        }
    }
}

impl SyncWorker {
    fn save_config(&self, cfg: &SyncConfig) {
        if let Some(parent) = self.config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(cfg) {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&self.config_path)
            {
                let _ = file.write_all(json.as_bytes());
            }
        }
    }

    pub fn set_server_url(&self, url: &str) -> Result<SyncConfig, String> {
        let trimmed = url.trim().trim_end_matches('/');
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err("La URL del servidor debe iniciar con http:// o https://".into());
        }
        let mut guard = self.config.lock().unwrap();
        if guard.server_url != trimmed {
            guard.server_url = trimmed.to_owned();
            guard.access_token = None;
            guard.device_id = None;
            guard.last_seq = 0;
            self.save_config(&guard);
        }
        Ok(guard.clone())
    }

    pub async fn status(&self, store: &Store) -> Result<SyncStatus, String> {
        let stored = load_or_create_master_key().await?;
        let fingerprint = encode_key_b64(&stored.key[..6]);
        let cfg = self.config.lock().unwrap().clone();
        Ok(SyncStatus {
            server_url: cfg.server_url,
            registered: cfg.access_token.is_some(),
            user_id: cfg.user_id.map(|u| u.to_string()),
            device_id: cfg.device_id.map(|d| d.to_string()),
            last_seq: cfg.last_seq,
            pending_outbox: store.outbox_count().unwrap_or(0),
            keyring_backend: stored.backend.into(),
            key_fingerprint: fingerprint,
        })
    }

    pub async fn rotate_passphrase(
        &self,
        store: &Store,
        passphrase: &str,
    ) -> Result<SyncStatus, String> {
        let _ = derive_and_store_master_key(passphrase).await?;
        self.status(store).await
    }

    async fn ensure_session(&self) -> Result<(String, Uuid, Uuid, String), String> {
        let current = self.config.lock().unwrap().clone();
        if let (Some(uid), Some(did), Some(tok)) =
            (current.user_id, current.device_id, current.access_token)
        {
            return Ok((current.server_url, uid, did, tok));
        }
        let resp: AuthSessionResponse = self
            .client
            .post(format!("{}/api/v1/auth/register", current.server_url))
            .json(&RegisterDeviceRequest {
                user_id: current.user_id,
                device_name: "Fedora KDE Plasma".into(),
                platform: "fedora-kde-wayland".into(),
            })
            .send()
            .await
            .map_err(|e| format!("No se pudo conectar con {}: {e}", current.server_url))?
            .error_for_status()
            .map_err(|e| format!("El servidor rechazó el registro: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let mut guard = self.config.lock().unwrap();
        guard.user_id = Some(resp.user_id);
        guard.device_id = Some(resp.device_id);
        guard.access_token = Some(resp.access_token.clone());
        self.save_config(&guard);
        Ok((
            guard.server_url.clone(),
            resp.user_id,
            resp.device_id,
            resp.access_token,
        ))
    }

    pub async fn create_pairing_qr(&self) -> Result<PairingQrView, String> {
        let stored = load_or_create_master_key().await?;
        let (server_url, _, _, token) = self.ensure_session().await?;
        let pair_resp: PairingTokenResponse = self
            .client
            .post(format!("{server_url}/api/v1/devices/pair"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("No se pudo solicitar el token QR al servidor: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Error al crear token de emparejamiento: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let payload = PairingQrPayload::new(
            &server_url,
            &pair_resp.pairing_token,
            encode_key_b64(&stored.key),
            pair_resp.expires_at,
        );
        let payload_json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let qr_svg = render_qr_svg(&payload_json)?;
        Ok(PairingQrView {
            qr_svg,
            payload_json,
            pairing_token: pair_resp.pairing_token,
            expires_at: pair_resp.expires_at,
            server_url,
        })
    }

    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>, String> {
        let (server_url, _, _, token) = self.ensure_session().await?;
        self.client
            .get(format!("{server_url}/api/v1/devices"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn sync_now(
        &self,
        app: &AppHandle,
        store: &Store,
    ) -> Result<SyncCycleReport, String> {
        let stored = load_or_create_master_key().await?;
        let (server_url, _, device_id, token) = self.ensure_session().await?;
        let outbox = store.pending_outbox_entries()?;
        let mut pushed = 0usize;

        if !outbox.is_empty() {
            let mut blobs = Vec::with_capacity(outbox.len());
            let mut ack_ids = Vec::with_capacity(outbox.len());
            for item in &outbox {
                let Ok(entity_id) = Uuid::parse_str(&item.entity_id) else {
                    continue;
                };
                let aad = blob_associated_data(&entity_id, item.entity_kind, &item.hlc_timestamp);
                let sealed = seal(&stored.key, item.payload_json.as_bytes(), &aad)
                    .map_err(|e| e.to_string())?;
                blobs.push(PushBlob {
                    entity_id,
                    entity_kind: item.entity_kind,
                    device_id,
                    hlc_timestamp: item.hlc_timestamp.clone(),
                    dedup_hmac: Some(sealed.dedup_hmac),
                    nonce: sealed.nonce,
                    ciphertext: sealed.ciphertext,
                    is_deleted: item.is_deleted,
                });
                ack_ids.push(item.entity_id.clone());
            }

            let resp: PushResponse = self
                .client
                .post(format!("{server_url}/api/v1/sync/push"))
                .bearer_auth(&token)
                .json(&PushRequest { blobs })
                .send()
                .await
                .map_err(|e| format!("Error al subir blobs cifrados: {e}"))?
                .error_for_status()
                .map_err(|e| format!("Push rechazado: {e}"))?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            store.acknowledge_outbox(&ack_ids)?;
            pushed = resp.accepted;
        }

        let since_seq = self.config.lock().unwrap().last_seq;
        let pull_resp: PullResponse = self
            .client
            .get(format!(
                "{server_url}/api/v1/sync/pull?since_seq={since_seq}"
            ))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("Error al descargar deltas cifrados: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Pull rechazado: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let mut pulled = 0usize;
        for blob in &pull_resp.blobs {
            if apply_encrypted_blob(store, &stored.key, blob).unwrap_or(false) {
                pulled += 1;
            }
        }

        let latest_seq = pull_resp.latest_seq.max(since_seq);
        {
            let mut guard = self.config.lock().unwrap();
            guard.last_seq = latest_seq;
            self.save_config(&guard);
        }

        if pulled > 0 {
            let _ = app.emit("clipboard-updated", ());
        }
        let _ = app.emit("sync-updated", ());
        Ok(SyncCycleReport {
            pushed,
            pulled,
            last_seq: latest_seq,
        })
    }
}

fn apply_encrypted_blob(
    store: &Store,
    master_key: &[u8; MASTER_KEY_LEN],
    blob: &fluxcast_protocol::EncryptedBlob,
) -> Result<bool, String> {
    let aad = blob_associated_data(&blob.entity_id, blob.entity_kind, &blob.hlc_timestamp);
    let plaintext = open(master_key, &blob.nonce, &blob.ciphertext, &aad)
        .map_err(|e| format!("No se pudo descifrar el blob {}: {e}", blob.entity_id))?;
    let json = String::from_utf8(plaintext).map_err(|e| e.to_string())?;
    store.apply_remote_entity(
        &blob.entity_id.to_string(),
        blob.entity_kind,
        &blob.hlc_timestamp,
        blob.is_deleted,
        &json,
    )
}

pub fn render_qr_svg(data: &str) -> Result<String, String> {
    let code = QrCode::new(data.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(210, 210)
        .dark_color(svg::Color("#8edbc4"))
        .light_color(svg::Color("#161a1e"))
        .build())
}

pub fn spawn_realtime_listener(app: AppHandle, store: Arc<Store>, worker: Arc<SyncWorker>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let cfg = worker.config.lock().unwrap().clone();
            let Some(token) = cfg.access_token else {
                continue;
            };
            let Ok(stored) = load_or_create_master_key().await else {
                continue;
            };
            let ws_base = cfg
                .server_url
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1);
            let url = format!("{ws_base}/api/v1/ws?token={token}");
            let Ok((mut stream, _)) = tokio_tungstenite::connect_async(&url).await else {
                continue;
            };
            while let Some(Ok(msg)) = stream.next().await {
                let Ok(text) = msg.into_text() else {
                    continue;
                };
                if let Ok(WsFrame::Broadcast { blob }) = serde_json::from_str::<WsFrame>(&text) {
                    if apply_encrypted_blob(&store, &stored.key, &blob).unwrap_or(false) {
                        let mut guard = worker.config.lock().unwrap();
                        guard.last_seq = guard.last_seq.max(blob.seq);
                        worker.save_config(&guard);
                        let _ = app.emit("clipboard-updated", ());
                        let _ = app.emit("sync-updated", ());
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_valid_svg_qr_for_pairing_payload() {
        let svg = render_qr_svg(r#"{"version":1,"server_url":"http://127.0.0.1:8787"}"#).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("#8edbc4"));
    }
}
