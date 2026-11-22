//! Almacenamiento seguro de la Master Key en Plasma `ksecretd` / KWallet (Freedesktop Secret Service)
//! mediante `oo7`, con respaldo local `0600` cuando no hay demonio de secretos disponible.

use fluxcast_crypto::{
    decode_key_b64, derive_master_key, encode_key_b64, generate_master_key, generate_salt,
    MASTER_KEY_LEN,
};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::Duration,
};

const SECRET_LABEL: &str = "FluxCast E2EE Master Key";
const ATTR_APP: &str = "dev.fluxcast.desktop";
const ATTR_KIND: &str = "e2ee_master_key";

pub struct StoredMasterKey {
    pub key: [u8; MASTER_KEY_LEN],
    pub backend: &'static str,
}

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([("application", ATTR_APP), ("kind", ATTR_KIND)])
}

fn fallback_key_path() -> PathBuf {
    gio::glib::user_data_dir().join("fluxcast/master.key")
}

/// Obtiene la Master Key de `ksecretd` / KWallet o genera una nueva si no existe.
pub async fn load_or_create_master_key() -> Result<StoredMasterKey, String> {
    if let Ok(Some((key, backend))) = try_load_from_kwallet().await {
        return Ok(StoredMasterKey { key, backend });
    }
    if let Ok(key) = load_fallback_file() {
        return Ok(StoredMasterKey {
            key,
            backend: "Archivo local 0600 (respaldo sin ksecretd)",
        });
    }
    let key = generate_master_key().map_err(|e| e.to_string())?;
    let backend = save_master_key(&key).await?;
    Ok(StoredMasterKey { key, backend })
}

/// Deriva una Master Key con Argon2id desde una contraseña y la guarda en KWallet / `ksecretd`.
pub async fn derive_and_store_master_key(passphrase: &str) -> Result<StoredMasterKey, String> {
    if passphrase.trim().len() < 6 {
        return Err("La frase de paso debe tener al menos 6 caracteres".into());
    }
    let salt = generate_salt().map_err(|e| e.to_string())?;
    let key = derive_master_key(passphrase.trim().as_bytes(), &salt).map_err(|e| e.to_string())?;
    let backend = save_master_key(&key).await?;
    Ok(StoredMasterKey { key, backend })
}

pub async fn save_master_key(key: &[u8; MASTER_KEY_LEN]) -> Result<&'static str, String> {
    let encoded = encode_key_b64(key);
    let _ = save_fallback_file(&encoded);
    if try_save_to_kwallet(&encoded).await.is_ok() {
        return Ok("Plasma ksecretd / KWallet (oo7)");
    }
    Ok("Archivo local 0600 (respaldo sin ksecretd)")
}

async fn try_load_from_kwallet() -> Result<Option<([u8; MASTER_KEY_LEN], &'static str)>, String> {
    let fut = async {
        let keyring = oo7::Keyring::new().await.map_err(|e| e.to_string())?;
        let items = keyring
            .search_items(&attributes())
            .await
            .map_err(|e| e.to_string())?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item.secret().await.map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(secret.as_bytes());
        let key = decode_key_b64(&text).map_err(|e| e.to_string())?;
        Ok(Some((key, "Plasma ksecretd / KWallet (oo7)")))
    };
    tokio::time::timeout(Duration::from_millis(1200), fut)
        .await
        .map_err(|_| "Tiempo de espera agotado al consultar ksecretd".to_string())?
}

async fn try_save_to_kwallet(encoded_key: &str) -> Result<(), String> {
    let fut = async {
        let keyring = oo7::Keyring::new().await.map_err(|e| e.to_string())?;
        keyring
            .create_item(SECRET_LABEL, &attributes(), encoded_key, true)
            .await
            .map_err(|e| e.to_string())
    };
    tokio::time::timeout(Duration::from_millis(1200), fut)
        .await
        .map_err(|_| "Tiempo de espera agotado al guardar en ksecretd".to_string())?
}

fn load_fallback_file() -> Result<[u8; MASTER_KEY_LEN], String> {
    let raw = fs::read_to_string(fallback_key_path()).map_err(|e| e.to_string())?;
    decode_key_b64(&raw).map_err(|e| e.to_string())
}

fn save_fallback_file(encoded_key: &str) -> Result<(), String> {
    let path = fallback_key_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(encoded_key.as_bytes())
        .map_err(|e| e.to_string())
}
