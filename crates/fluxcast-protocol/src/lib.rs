//! Modelos del protocolo de sincronización Zero-Knowledge, HLC (Last-Write-Wins) y emparejamiento QR.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PAIRING_TTL_SECS: i64 = 120;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(i16)]
pub enum EntityKind {
    Clipboard = 1,
    Snippet = 2,
}

impl EntityKind {
    pub fn from_i16(value: i16) -> Option<Self> {
        match value {
            1 => Some(Self::Clipboard),
            2 => Some(Self::Snippet),
            _ => None,
        }
    }
}

/// Hybrid Logical Clock (`<unix_ms:015>-<counter:04x>-<node_id>`) para resolución
/// determinística de conflictos Last-Write-Wins (LWW) entre Fedora y Android.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Hlc {
    pub wall_ms: u64,
    pub logical: u32,
    pub node: String,
}

impl Hlc {
    pub fn new(wall_ms: u64, logical: u32, node: impl Into<String>) -> Self {
        Self {
            wall_ms,
            logical,
            node: node.into(),
        }
    }

    pub fn encode(&self) -> String {
        format!("{:015}-{:04x}-{}", self.wall_ms, self.logical, self.node)
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(3, '-');
        let wall_ms = parts.next()?.parse::<u64>().ok()?;
        let logical = u32::from_str_radix(parts.next()?, 16).ok()?;
        let node = parts.next()?.to_owned();
        if node.is_empty() {
            return None;
        }
        Some(Self {
            wall_ms,
            logical,
            node,
        })
    }

    /// Avanza el reloj local garantizando monotonicidad estricta frente a relojes remotos.
    pub fn observe_and_tick(&mut self, now_ms: u64, remote: Option<&Hlc>) -> String {
        match remote {
            Some(rem) => {
                let max_wall = now_ms.max(self.wall_ms).max(rem.wall_ms);
                if max_wall == self.wall_ms && max_wall == rem.wall_ms {
                    self.logical = self.logical.max(rem.logical) + 1;
                } else if max_wall == self.wall_ms {
                    self.logical += 1;
                } else if max_wall == rem.wall_ms {
                    self.logical = rem.logical + 1;
                } else {
                    self.logical = 0;
                }
                self.wall_ms = max_wall;
            }
            None => {
                if now_ms > self.wall_ms {
                    self.wall_ms = now_ms;
                    self.logical = 0;
                } else {
                    self.logical += 1;
                }
            }
        }
        self.encode()
    }
}

/// Devuelve `true` si el timestamp HLC candidato gana bajo Last-Write-Wins frente al actual.
pub fn hlc_is_newer(candidate: &str, current: &str) -> bool {
    match (Hlc::parse(candidate), Hlc::parse(current)) {
        (Some(cand), Some(curr)) => cand > curr,
        _ => candidate > current,
    }
}

/// Construye los datos autenticados adicionales (AAD) para vincular criptográficamente
/// el blob cifrado con su `entity_id`, `entity_kind` y `hlc_timestamp`.
pub fn blob_associated_data(entity_id: &Uuid, entity_kind: i16, hlc_timestamp: &str) -> Vec<u8> {
    format!("fluxcast:v1:{entity_id}:{entity_kind}:{hlc_timestamp}").into_bytes()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PushBlob {
    pub entity_id: Uuid,
    pub entity_kind: i16,
    pub device_id: Uuid,
    pub hlc_timestamp: String,
    pub dedup_hmac: Option<Vec<u8>>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncryptedBlob {
    pub seq: i64,
    pub user_id: Uuid,
    pub entity_id: Uuid,
    pub entity_kind: i16,
    pub device_id: Uuid,
    pub hlc_timestamp: String,
    pub dedup_hmac: Option<Vec<u8>>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PushRequest {
    pub blobs: Vec<PushBlob>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PushResponse {
    pub accepted: usize,
    pub latest_seq: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PullResponse {
    pub blobs: Vec<EncryptedBlob>,
    pub latest_seq: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegisterDeviceRequest {
    pub user_id: Option<Uuid>,
    pub device_name: String,
    pub platform: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthSessionResponse {
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub access_token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceInfo {
    pub device_id: Uuid,
    pub device_name: String,
    pub platform: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PairingTokenResponse {
    pub pairing_token: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimPairingRequest {
    pub pairing_token: String,
    pub device_name: String,
    pub platform: String,
}

/// Payload codificado en el QR efímero (120 s) mostrado en Fedora KDE para vincular Android.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PairingQrPayload {
    pub version: u8,
    pub server_url: String,
    pub pairing_token: String,
    pub master_key_b64: String,
    pub expires_at: i64,
}

impl PairingQrPayload {
    pub fn new(
        server_url: impl Into<String>,
        pairing_token: impl Into<String>,
        master_key_b64: impl Into<String>,
        expires_at: i64,
    ) -> Self {
        Self {
            version: 1,
            server_url: server_url.into(),
            pairing_token: pairing_token.into(),
            master_key_b64: master_key_b64.into(),
            expires_at,
        }
    }

    pub fn is_expired(&self, now_unix_secs: i64) -> bool {
        now_unix_secs >= self.expires_at
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsFrame {
    Ping,
    Pong,
    Push { blobs: Vec<PushBlob> },
    Broadcast { blob: EncryptedBlob },
    Ack { seq: i64 },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hlc_ordering_and_lww_resolution_are_monotonic_and_deterministic() {
        let mut fedora = Hlc::new(1_700_000_000_000, 0, "fedora");
        let ts1 = fedora.observe_and_tick(1_700_000_000_000, None);
        let ts2 = fedora.observe_and_tick(1_700_000_000_000, None);
        assert!(hlc_is_newer(&ts2, &ts1));

        // Un evento remoto con reloj adelantado hace avanzar el HLC local.
        let android = Hlc::new(1_700_000_005_000, 3, "android");
        let ts3 = fedora.observe_and_tick(1_700_000_001_000, Some(&android));
        assert!(hlc_is_newer(&ts3, &android.encode()));
    }
}
