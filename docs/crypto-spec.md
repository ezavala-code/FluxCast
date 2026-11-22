# FluxCast — Zero-Knowledge E2EE Cryptographic Specification

## 1. Threat Model

The **Sync Relay Server** (`apps/sync-server`) and its PostgreSQL database are treated as **untrusted infrastructure**:
- The server never receives the Master Key, user passphrase, or plaintext clipboard/snippet content.
- All encryption, authentication (AEAD), and deduplication hashing happen client-side inside `crates/fluxcast-crypto` (natively on Fedora KDE and via `libfluxcast_uniffi.so` on Android).

---

## 2. Primitives & Parameters

| Function | Primitive | Parameters |
| :--- | :--- | :--- |
| **Master Key** | Symmetric 256-bit key | 32 bytes (`MASTER_KEY_LEN = 32`) generated via OS CSPRNG (`getrandom`) or derived via Argon2id. |
| **Key Derivation (KDF)** | `Argon2id` (v0x13) | Memory `m = 19,456 KiB` (19 MiB), Iterations `t = 2`, Parallelism `p = 1`, Salt `>= 16 bytes`, Output `32 bytes`. |
| **Authenticated Encryption (AEAD)** | `XChaCha20-Poly1305` | 256-bit key, 192-bit random nonce (`24 bytes`), 128-bit Poly1305 authentication tag (`16 bytes`). |
| **Associated Data (AAD)** | Canonical UTF-8 binding | `fluxcast:v1:<entity_uuid>:<entity_kind>:<hlc_timestamp>` |
| **Blind Deduplication** | `HMAC-SHA256` | Subkey `K_dedup = HMAC-SHA256(MasterKey, "fluxcast-e2ee-dedup-subkey-v1")`; Digest `HMAC-SHA256(K_dedup, plaintext)`. |

---

## 3. Authenticated Associated Data (AAD) Prevented Attacks

Every encrypted blob binds its `ciphertext` to:
```text
fluxcast:v1:<entity_id>:<entity_kind>:<hlc_timestamp>
```
If a compromised relay server attempts to:
1. Swap the `ciphertext` of one entity into another `entity_id`,
2. Replay an older `ciphertext` under a newer `hlc_timestamp` to win Last-Write-Wins conflict resolution, or
3. Reclassify a `clipboard` blob (`entity_kind = 1`) as a `snippet` (`entity_kind = 2`),

Poly1305 tag verification fails immediately with `CryptoError::DecryptionFailed`.

---

## 4. Ephemeral QR Pairing Protocol (120-Second Single-Use)

1. **Fedora KDE** loads or creates the 32-byte `MasterKey` in **Plasma `ksecretd` / KWallet** via `oo7`.
2. Fedora requests a one-time pairing token from `POST /api/v1/devices/pair` (`expires_at = now + 120s`).
3. Fedora renders an SVG QR code locally containing:
   ```json
   {
     "version": 1,
     "server_url": "https://sync.example.com",
     "pairing_token": "pair_...",
     "master_key_b64": "<Base64URL-no-pad 32-byte key>",
     "expires_at": 1763850120
   }
   ```
   *(Note: `master_key_b64` is never sent to `/api/v1/devices/pair`; it exists only in the local QR code scanned optically by the user's phone.)*
4. **Android** scans the QR code with CameraX + ML Kit, validates expiration via `uniffi_parse_pairing_qr`, claims the single-use `pairing_token` at `POST /api/v1/devices/claim`, and wraps the 32-byte `MasterKey` with an `AES-256-GCM` hardware key inside **Android Keystore**.
