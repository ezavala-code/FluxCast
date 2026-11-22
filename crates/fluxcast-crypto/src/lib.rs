//! Cifrado de extremo a extremo Zero-Knowledge (Argon2id + XChaCha20-Poly1305 + HMAC-SHA256).

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;

pub const MASTER_KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
pub const XNONCE_LEN: usize = 24;
pub const HMAC_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CryptoError {
    InvalidKeyLength,
    InvalidSaltLength,
    InvalidNonceLength,
    KdfFailed,
    EncryptionFailed,
    DecryptionFailed,
    RandomnessFailed,
    EncodingError,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::InvalidKeyLength => "La Master Key debe tener 32 bytes (256 bits)",
            Self::InvalidSaltLength => "El salt de Argon2id debe tener al menos 16 bytes",
            Self::InvalidNonceLength => "El nonce de XChaCha20-Poly1305 debe tener 24 bytes",
            Self::KdfFailed => "Error al derivar la llave con Argon2id",
            Self::EncryptionFailed => "Error al cifrar con XChaCha20-Poly1305",
            Self::DecryptionFailed => {
                "Error de autenticación o descifrado con XChaCha20-Poly1305 (AEAD)"
            }
            Self::RandomnessFailed => "Fallo en el generador criptográfico del sistema",
            Self::EncodingError => "Formato Base64URL inválido",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for CryptoError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SealedBlob {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub dedup_hmac: Vec<u8>,
}

/// Genera una Master Key aleatoria de 32 bytes (256 bits).
pub fn generate_master_key() -> Result<[u8; MASTER_KEY_LEN], CryptoError> {
    let mut key = [0u8; MASTER_KEY_LEN];
    getrandom::getrandom(&mut key).map_err(|_| CryptoError::RandomnessFailed)?;
    Ok(key)
}

/// Genera un salt aleatorio de 16 bytes para Argon2id.
pub fn generate_salt() -> Result<[u8; SALT_LEN], CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).map_err(|_| CryptoError::RandomnessFailed)?;
    Ok(salt)
}

/// Deriva una Master Key de 32 bytes usando Argon2id con parámetros recomendados por OWASP
/// (m = 19 MiB, t = 2 iteraciones, p = 1 grado de paralelismo).
pub fn derive_master_key(
    passphrase: &[u8],
    salt: &[u8],
) -> Result<[u8; MASTER_KEY_LEN], CryptoError> {
    if salt.len() < SALT_LEN {
        return Err(CryptoError::InvalidSaltLength);
    }
    let params =
        Params::new(19 * 1024, 2, 1, Some(MASTER_KEY_LEN)).map_err(|_| CryptoError::KdfFailed)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = [0u8; MASTER_KEY_LEN];
    argon2
        .hash_password_into(passphrase, salt, &mut output)
        .map_err(|_| CryptoError::KdfFailed)?;
    Ok(output)
}

/// Calcula un HMAC-SHA256 ciego con subllave derivada de la Master Key para deduplicación
/// sin revelar el contenido al Sync Relay Server.
pub fn blind_dedup_hmac(
    master_key: &[u8],
    plaintext: &[u8],
) -> Result<[u8; HMAC_LEN], CryptoError> {
    if master_key.len() != MASTER_KEY_LEN {
        return Err(CryptoError::InvalidKeyLength);
    }
    let mut subkey_mac = <HmacSha256 as Mac>::new_from_slice(master_key)
        .map_err(|_| CryptoError::InvalidKeyLength)?;
    subkey_mac.update(b"fluxcast-e2ee-dedup-subkey-v1");
    let subkey = subkey_mac.finalize().into_bytes();

    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(&subkey).map_err(|_| CryptoError::InvalidKeyLength)?;
    mac.update(plaintext);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; HMAC_LEN];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn blind_dedup_hmac_hex(master_key: &[u8], plaintext: &[u8]) -> Result<String, CryptoError> {
    let bytes = blind_dedup_hmac(master_key, plaintext)?;
    let mut hex = String::with_capacity(HMAC_LEN * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

/// Cifra `plaintext` con `XChaCha20-Poly1305` (nonce aleatorio de 24 bytes / 192 bits)
/// autenticando `associated_data` (ej. `entity_id` + `hlc_timestamp`) y calcula `dedup_hmac`.
pub fn seal(
    master_key: &[u8],
    plaintext: &[u8],
    associated_data: &[u8],
) -> Result<SealedBlob, CryptoError> {
    if master_key.len() != MASTER_KEY_LEN {
        return Err(CryptoError::InvalidKeyLength);
    }
    let mut nonce = [0u8; XNONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| CryptoError::RandomnessFailed)?;
    seal_with_nonce(master_key, &nonce, plaintext, associated_data)
}

pub fn seal_with_nonce(
    master_key: &[u8],
    nonce: &[u8],
    plaintext: &[u8],
    associated_data: &[u8],
) -> Result<SealedBlob, CryptoError> {
    if master_key.len() != MASTER_KEY_LEN {
        return Err(CryptoError::InvalidKeyLength);
    }
    if nonce.len() != XNONCE_LEN {
        return Err(CryptoError::InvalidNonceLength);
    }
    let cipher =
        XChaCha20Poly1305::new_from_slice(master_key).map_err(|_| CryptoError::InvalidKeyLength)?;
    let xnonce = XNonce::from_slice(nonce);
    let ciphertext = cipher
        .encrypt(
            xnonce,
            Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| CryptoError::EncryptionFailed)?;
    let dedup = blind_dedup_hmac(master_key, plaintext)?;
    Ok(SealedBlob {
        nonce: nonce.to_vec(),
        ciphertext,
        dedup_hmac: dedup.to_vec(),
    })
}

/// Descifra y verifica la autenticidad (Poly1305 MAC + AAD) de un blob `XChaCha20-Poly1305`.
pub fn open(
    master_key: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
    associated_data: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if master_key.len() != MASTER_KEY_LEN {
        return Err(CryptoError::InvalidKeyLength);
    }
    if nonce.len() != XNONCE_LEN {
        return Err(CryptoError::InvalidNonceLength);
    }
    let cipher =
        XChaCha20Poly1305::new_from_slice(master_key).map_err(|_| CryptoError::InvalidKeyLength)?;
    let xnonce = XNonce::from_slice(nonce);
    cipher
        .decrypt(
            xnonce,
            Payload {
                msg: ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| CryptoError::DecryptionFailed)
}

pub fn encode_key_b64(key: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

pub fn decode_key_b64(encoded: &str) -> Result<[u8; MASTER_KEY_LEN], CryptoError> {
    let raw = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .map_err(|_| CryptoError::EncodingError)?;
    raw.try_into().map_err(|_| CryptoError::InvalidKeyLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2id_kdf_and_xchacha20_poly1305_roundtrip_and_tamper_detection() {
        let salt = b"fluxcast-salt-16";
        let key1 = derive_master_key(b"correct horse battery staple", salt).unwrap();
        let key2 = derive_master_key(b"correct horse battery staple", salt).unwrap();
        assert_eq!(key1, key2);

        let aad = b"entity:019bf62b:hlc:001700000000-0001-fedora";
        let plaintext = br#"{"kind":"clipboard","content":"sudo dnf upgrade"}"#;
        let sealed = seal(&key1, plaintext, aad).unwrap();
        assert_eq!(sealed.nonce.len(), XNONCE_LEN);
        assert_eq!(sealed.dedup_hmac.len(), HMAC_LEN);
        assert_ne!(sealed.ciphertext, plaintext);

        let opened = open(&key1, &sealed.nonce, &sealed.ciphertext, aad).unwrap();
        assert_eq!(opened, plaintext);

        // Alterar 1 bit del ciphertext o del AAD debe fallar la autenticación Poly1305.
        let mut tampered = sealed.ciphertext.clone();
        tampered[0] ^= 0x01;
        assert_eq!(
            open(&key1, &sealed.nonce, &tampered, aad),
            Err(CryptoError::DecryptionFailed)
        );
        assert_eq!(
            open(&key1, &sealed.nonce, &sealed.ciphertext, b"wrong-aad"),
            Err(CryptoError::DecryptionFailed)
        );

        let encoded = encode_key_b64(&key1);
        assert_eq!(decode_key_b64(&encoded).unwrap(), key1);
    }
}
