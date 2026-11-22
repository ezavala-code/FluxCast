//! Bindings UniFFI exportados a Kotlin (Android) para criptografía E2EE, HLC y emparejamiento QR.

use fluxcast_crypto::{
    blind_dedup_hmac_hex, decode_key_b64, derive_master_key, encode_key_b64, generate_master_key,
    generate_salt, open, seal, CryptoError,
};
use fluxcast_protocol::{blob_associated_data, hlc_is_newer, PairingQrPayload};
use std::fmt;
use uuid::Uuid;

uniffi::setup_scaffolding!();

#[derive(Debug, Eq, PartialEq, uniffi::Error)]
pub enum UniffiCryptoError {
    InvalidKey,
    InvalidSalt,
    InvalidNonce,
    InvalidEntityId,
    EncryptionFailed,
    DecryptionFailed,
    QrInvalidOrExpired,
}

impl fmt::Display for UniffiCryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::InvalidKey => "Master Key inválida (debe tener 32 bytes)",
            Self::InvalidSalt => "Salt inválido (debe tener al menos 16 bytes)",
            Self::InvalidNonce => "Nonce inválido (debe tener 24 bytes)",
            Self::InvalidEntityId => "UUID de entidad inválido",
            Self::EncryptionFailed => "Fallo al cifrar con XChaCha20-Poly1305",
            Self::DecryptionFailed => "Fallo al autenticar o descifrar con XChaCha20-Poly1305",
            Self::QrInvalidOrExpired => "Código QR de vinculación inválido o expirado",
        };
        f.write_str(msg)
    }
}

impl From<CryptoError> for UniffiCryptoError {
    fn from(value: CryptoError) -> Self {
        match value {
            CryptoError::InvalidKeyLength | CryptoError::EncodingError => Self::InvalidKey,
            CryptoError::InvalidSaltLength | CryptoError::KdfFailed => Self::InvalidSalt,
            CryptoError::InvalidNonceLength => Self::InvalidNonce,
            CryptoError::EncryptionFailed | CryptoError::RandomnessFailed => Self::EncryptionFailed,
            CryptoError::DecryptionFailed => Self::DecryptionFailed,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct EncryptedEnvelope {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub dedup_hmac: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct PairingSessionInfo {
    pub server_url: String,
    pub pairing_token: String,
    pub master_key: Vec<u8>,
    pub expires_at: i64,
}

#[uniffi::export]
pub fn uniffi_generate_master_key() -> Result<Vec<u8>, UniffiCryptoError> {
    Ok(generate_master_key()?.to_vec())
}

#[uniffi::export]
pub fn uniffi_generate_salt() -> Result<Vec<u8>, UniffiCryptoError> {
    Ok(generate_salt()?.to_vec())
}

#[uniffi::export]
pub fn uniffi_derive_master_key(
    passphrase: String,
    salt: Vec<u8>,
) -> Result<Vec<u8>, UniffiCryptoError> {
    Ok(derive_master_key(passphrase.as_bytes(), &salt)?.to_vec())
}

#[uniffi::export]
pub fn uniffi_encode_master_key_b64(master_key: Vec<u8>) -> Result<String, UniffiCryptoError> {
    if master_key.len() != fluxcast_crypto::MASTER_KEY_LEN {
        return Err(UniffiCryptoError::InvalidKey);
    }
    Ok(encode_key_b64(&master_key))
}

#[uniffi::export]
pub fn uniffi_decode_master_key_b64(encoded: String) -> Result<Vec<u8>, UniffiCryptoError> {
    Ok(decode_key_b64(&encoded)?.to_vec())
}

#[uniffi::export]
pub fn uniffi_compute_dedup_hmac_hex(
    master_key: Vec<u8>,
    content: String,
) -> Result<String, UniffiCryptoError> {
    Ok(blind_dedup_hmac_hex(&master_key, content.as_bytes())?)
}

#[uniffi::export]
pub fn uniffi_seal_entity(
    master_key: Vec<u8>,
    entity_id: String,
    entity_kind: i16,
    hlc_timestamp: String,
    plaintext: Vec<u8>,
) -> Result<EncryptedEnvelope, UniffiCryptoError> {
    let uuid = Uuid::parse_str(&entity_id).map_err(|_| UniffiCryptoError::InvalidEntityId)?;
    let aad = blob_associated_data(&uuid, entity_kind, &hlc_timestamp);
    let sealed = seal(&master_key, &plaintext, &aad)?;
    Ok(EncryptedEnvelope {
        nonce: sealed.nonce,
        ciphertext: sealed.ciphertext,
        dedup_hmac: sealed.dedup_hmac,
    })
}

#[uniffi::export]
pub fn uniffi_open_entity(
    master_key: Vec<u8>,
    entity_id: String,
    entity_kind: i16,
    hlc_timestamp: String,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
) -> Result<Vec<u8>, UniffiCryptoError> {
    let uuid = Uuid::parse_str(&entity_id).map_err(|_| UniffiCryptoError::InvalidEntityId)?;
    let aad = blob_associated_data(&uuid, entity_kind, &hlc_timestamp);
    Ok(open(&master_key, &nonce, &ciphertext, &aad)?)
}

#[uniffi::export]
pub fn uniffi_hlc_should_apply_remote(local_hlc: String, remote_hlc: String) -> bool {
    hlc_is_newer(&remote_hlc, &local_hlc)
}

#[uniffi::export]
pub fn uniffi_parse_pairing_qr(
    qr_json: String,
    now_unix_secs: i64,
) -> Result<PairingSessionInfo, UniffiCryptoError> {
    let payload: PairingQrPayload =
        serde_json::from_str(&qr_json).map_err(|_| UniffiCryptoError::QrInvalidOrExpired)?;
    if payload.version != 1
        || payload.is_expired(now_unix_secs)
        || payload.server_url.trim().is_empty()
        || payload.pairing_token.trim().is_empty()
    {
        return Err(UniffiCryptoError::QrInvalidOrExpired);
    }
    let key = decode_key_b64(&payload.master_key_b64)?;
    Ok(PairingSessionInfo {
        server_url: payload.server_url,
        pairing_token: payload.pairing_token,
        master_key: key.to_vec(),
        expires_at: payload.expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniffi_exports_seal_open_qr_and_hlc_consistently() {
        let key = uniffi_generate_master_key().unwrap();
        let entity_id = Uuid::now_v7().to_string();
        let hlc = "001700000000100-0001-fedora".to_owned();
        let msg = b"{\"content\":\"token-secreto\"}".to_vec();

        let env = uniffi_seal_entity(key.clone(), entity_id.clone(), 1, hlc.clone(), msg.clone())
            .unwrap();
        let plain = uniffi_open_entity(
            key.clone(),
            entity_id,
            1,
            hlc.clone(),
            env.nonce,
            env.ciphertext,
        )
        .unwrap();
        assert_eq!(plain, msg);

        assert!(uniffi_hlc_should_apply_remote(
            hlc,
            "001700000000200-0000-android".into()
        ));

        let qr = serde_json::to_string(&PairingQrPayload::new(
            "https://sync.fluxcast.local",
            "tok_123",
            uniffi_encode_master_key_b64(key.clone()).unwrap(),
            2000,
        ))
        .unwrap();
        let info = uniffi_parse_pairing_qr(qr.clone(), 1950).unwrap();
        assert_eq!(info.master_key, key);
        assert_eq!(
            uniffi_parse_pairing_qr(qr, 2001),
            Err(UniffiCryptoError::QrInvalidOrExpired)
        );
    }
}
