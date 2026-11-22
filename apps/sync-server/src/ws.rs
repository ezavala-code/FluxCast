use crate::{auth::Claims, routes::AppState};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
};
use fluxcast_protocol::{EncryptedBlob, WsFrame};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{collections::HashMap, sync::RwLock};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Default)]
pub struct WsHub {
    channels: RwLock<HashMap<Uuid, broadcast::Sender<EncryptedBlob>>>,
}

impl WsHub {
    pub fn subscribe(&self, user_id: Uuid) -> broadcast::Receiver<EncryptedBlob> {
        if let Some(tx) = self.channels.read().unwrap().get(&user_id) {
            return tx.subscribe();
        }
        let mut guard = self.channels.write().unwrap();
        let tx = guard
            .entry(user_id)
            .or_insert_with(|| broadcast::channel(256).0);
        tx.subscribe()
    }

    pub fn broadcast(&self, blob: &EncryptedBlob) {
        if let Some(tx) = self.channels.read().unwrap().get(&blob.user_id) {
            let _ = tx.send(blob.clone());
        }
    }
}

#[derive(Deserialize)]
pub struct WsAuthQuery {
    pub token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let claims = state
        .jwt
        .verify(&query.token)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_owned()))?;
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, claims)))
}

async fn handle_socket(socket: WebSocket, state: AppState, claims: Claims) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.hub.subscribe(claims.sub);
    let user_id = claims.sub;
    let device_id = claims.dev;

    loop {
        tokio::select! {
            Ok(blob) = rx.recv() => {
                if blob.device_id == device_id {
                    continue;
                }
                if let Ok(json) = serde_json::to_string(&WsFrame::Broadcast { blob }) {
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
            Some(Ok(msg)) = receiver.next() => {
                let Message::Text(text) = msg else {
                    if matches!(msg, Message::Close(_)) {
                        break;
                    }
                    continue;
                };
                let Ok(frame) = serde_json::from_str::<WsFrame>(&text) else {
                    continue;
                };
                match frame {
                    WsFrame::Ping => {
                        if let Ok(pong) = serde_json::to_string(&WsFrame::Pong) {
                            let _ = sender.send(Message::Text(pong.into())).await;
                        }
                    }
                    WsFrame::Push { blobs } => {
                        match state.store.push_blobs(user_id, &blobs, &state.hub).await {
                            Ok(resp) => {
                                if let Ok(ack) = serde_json::to_string(&WsFrame::Ack { seq: resp.latest_seq }) {
                                    let _ = sender.send(Message::Text(ack.into())).await;
                                }
                            }
                            Err(err) => {
                                if let Ok(frame) = serde_json::to_string(&WsFrame::Error { message: err }) {
                                    let _ = sender.send(Message::Text(frame.into())).await;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            else => break,
        }
    }
}
