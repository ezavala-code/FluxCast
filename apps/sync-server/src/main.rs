use fluxcast_sync_server::{
    auth::JwtSigner,
    routes::{router, AppState, Storage},
    ws::WsHub,
};
use std::{env, sync::Arc};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr = env::var("FLUXCAST_BIND").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let jwt_secret = env::var("FLUXCAST_JWT_SECRET")
        .unwrap_or_else(|_| "fluxcast-dev-jwt-secret-change-in-prod".into());

    let store = match env::var("DATABASE_URL") {
        Ok(url) if url.starts_with("postgres://") || url.starts_with("postgresql://") => {
            println!("FluxCast Sync Server · Conectando a PostgreSQL...");
            Storage::connect_postgres(&url).await?
        }
        _ => {
            println!("FluxCast Sync Server · Modo almacenamiento en memoria (Zero-Knowledge)");
            Storage::memory()
        }
    };

    let state = AppState {
        store,
        jwt: Arc::new(JwtSigner::new(jwt_secret.into_bytes())),
        hub: Arc::new(WsHub::default()),
    };

    let listener = TcpListener::bind(&bind_addr).await?;
    println!("FluxCast Sync Relay Server escuchando en http://{bind_addr}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
