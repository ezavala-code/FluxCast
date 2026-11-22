use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const JWT_TTL_SECS: i64 = 30 * 24 * 3600;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Claims {
    pub sub: Uuid,
    pub dev: Uuid,
    pub exp: i64,
}

#[derive(Clone)]
pub struct JwtSigner {
    secret: Vec<u8>,
}

impl JwtSigner {
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    pub fn issue(&self, user_id: Uuid, device_id: Uuid) -> String {
        let claims = Claims {
            sub: user_id,
            dev: device_id,
            exp: unix_secs() + JWT_TTL_SECS,
        };
        self.encode_claims(&claims)
    }

    pub fn encode_claims(&self, claims: &Claims) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("Claims serializables"));
        let signing_input = format!("{header}.{payload}");
        let sig = self.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(sig))
    }

    pub fn verify(&self, token: &str) -> Result<Claims, &'static str> {
        let parts: Vec<&str> = token.trim().split('.').collect();
        if parts.len() != 3 {
            return Err("Formato JWT inválido");
        }
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| "Firma Base64URL inválida")?;
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&self.secret)
            .map_err(|_| "Secreto JWT inválido")?;
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| "Firma JWT no válida")?;
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| "Payload JWT inválido")?;
        let claims: Claims =
            serde_json::from_slice(&payload_bytes).map_err(|_| "Claims JWT inválidos")?;
        if claims.exp <= unix_secs() {
            return Err("Token JWT expirado");
        }
        Ok(claims)
    }

    fn sign(&self, data: &[u8]) -> Vec<u8> {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&self.secret).expect("HMAC acepta cualquier llave");
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }
}

pub fn generate_pairing_token() -> String {
    let bytes = fluxcast_crypto::generate_master_key().unwrap_or_else(|_| {
        *Uuid::new_v4()
            .as_bytes()
            .repeat(2)
            .first_chunk::<32>()
            .unwrap()
    });
    format!("pair_{}", URL_SAFE_NO_PAD.encode(&bytes[..24]))
}

pub fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
