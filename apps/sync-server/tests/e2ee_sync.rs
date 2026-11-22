use fluxcast_crypto::{decode_key_b64, encode_key_b64, generate_master_key, open, seal};
use fluxcast_protocol::{
    blob_associated_data, AuthSessionResponse, ClaimPairingRequest, PairingQrPayload,
    PairingTokenResponse, PullResponse, PushBlob, PushRequest, PushResponse, RegisterDeviceRequest,
    WsFrame,
};
use fluxcast_sync_server::{
    auth::JwtSigner,
    routes::{router, AppState, Storage},
    ws::WsHub,
};
use futures_util::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;
use uuid::Uuid;

#[tokio::test]
async fn zero_knowledge_qr_pairing_e2ee_push_pull_lww_and_websocket_broadcast() {
    let state = AppState {
        store: Storage::memory(),
        jwt: Arc::new(JwtSigner::new(b"test-jwt-secret-key-stage3".to_vec())),
        hub: Arc::new(WsHub::default()),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });

    let client = reqwest::Client::new();

    // 1. Fedora registra su sesión inicial.
    let fedora: AuthSessionResponse = client
        .post(format!("{base_url}/api/v1/auth/register"))
        .json(&RegisterDeviceRequest {
            user_id: None,
            device_name: "Fedora KDE Laptop".into(),
            platform: "fedora-kde-wayland".into(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // 2. Fedora solicita un token efímero (120 s) y construye el payload QR con la Master Key.
    let pair_token: PairingTokenResponse = client
        .post(format!("{base_url}/api/v1/devices/pair"))
        .bearer_auth(&fedora.access_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let master_key = generate_master_key().unwrap();
    let qr_payload = PairingQrPayload::new(
        &base_url,
        &pair_token.pairing_token,
        encode_key_b64(&master_key),
        pair_token.expires_at,
    );
    let qr_json = serde_json::to_string(&qr_payload).unwrap();

    // 3. Android escanea el QR y reclama el token de un solo uso.
    let parsed_qr: PairingQrPayload = serde_json::from_str(&qr_json).unwrap();
    let android_key = decode_key_b64(&parsed_qr.master_key_b64).unwrap();
    assert_eq!(android_key, master_key);

    let android: AuthSessionResponse = client
        .post(format!("{}/api/v1/devices/claim", parsed_qr.server_url))
        .json(&ClaimPairingRequest {
            pairing_token: parsed_qr.pairing_token.clone(),
            device_name: "Pixel Android".into(),
            platform: "android".into(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(android.user_id, fedora.user_id);

    // Reutilizar el mismo token de emparejamiento debe rechazarse.
    let replay = client
        .post(format!("{base_url}/api/v1/devices/claim"))
        .json(&ClaimPairingRequest {
            pairing_token: parsed_qr.pairing_token,
            device_name: "Intruso".into(),
            platform: "android".into(),
        })
        .send()
        .await
        .unwrap();
    assert!(!replay.status().is_success());

    // 4. Android abre el WebSocket en tiempo real.
    let ws_url = format!("ws://{addr}/api/v1/ws?token={}", android.access_token);
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // 5. Fedora cifra un ítem con XChaCha20-Poly1305 y hace push.
    let entity_id = Uuid::now_v7();
    let hlc_new = "001700000000500-0001-fedora-kde".to_owned();
    let plaintext = br#"{"content_type":"code","content":"cargo test --workspace"}"#;
    let aad = blob_associated_data(&entity_id, 1, &hlc_new);
    let sealed = seal(&master_key, plaintext, &aad).unwrap();

    let push_resp: PushResponse = client
        .post(format!("{base_url}/api/v1/sync/push"))
        .bearer_auth(&fedora.access_token)
        .json(&PushRequest {
            blobs: vec![PushBlob {
                entity_id,
                entity_kind: 1,
                device_id: fedora.device_id,
                hlc_timestamp: hlc_new.clone(),
                dedup_hmac: Some(sealed.dedup_hmac.clone()),
                nonce: sealed.nonce.clone(),
                ciphertext: sealed.ciphertext.clone(),
                is_deleted: false,
            }],
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(push_resp.accepted, 1);

    // 6. Android recibe el broadcast por WebSocket y lo descifra con la Master Key.
    let ws_msg = tokio::time::timeout(Duration::from_secs(3), ws_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsFrame::Broadcast { blob } = serde_json::from_str(&ws_msg.into_text().unwrap()).unwrap()
    else {
        panic!("Se esperaba WsFrame::Broadcast");
    };
    let rx_aad = blob_associated_data(&blob.entity_id, blob.entity_kind, &blob.hlc_timestamp);
    let decrypted = open(&android_key, &blob.nonce, &blob.ciphertext, &rx_aad).unwrap();
    assert_eq!(decrypted, plaintext);

    // 7. Resolución Last-Write-Wins (HLC): un push con HLC más antiguo es ignorado.
    let stale_push: PushResponse = client
        .post(format!("{base_url}/api/v1/sync/push"))
        .bearer_auth(&fedora.access_token)
        .json(&PushRequest {
            blobs: vec![PushBlob {
                entity_id,
                entity_kind: 1,
                device_id: fedora.device_id,
                hlc_timestamp: "001700000000100-0000-fedora-kde".into(),
                dedup_hmac: None,
                nonce: sealed.nonce,
                ciphertext: vec![1, 2, 3],
                is_deleted: false,
            }],
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stale_push.accepted, 0);

    // 8. Pull delta desde cursor 0 devuelve exactamente el blob ganador.
    let pull: PullResponse = client
        .get(format!("{base_url}/api/v1/sync/pull?since_seq=0"))
        .bearer_auth(&android.access_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pull.blobs.len(), 1);
    assert_eq!(pull.blobs[0].hlc_timestamp, hlc_new);
}
