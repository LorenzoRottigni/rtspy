use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    routing::{get, post},
    Router,
};
use tokio::{process::Child, sync::Mutex};

mod api;

#[derive(Clone)]
pub struct AppState {
    pub workers: Arc<Mutex<HashMap<String, Child>>>,
}

#[tokio::main]
async fn main() -> opencv::Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let state = AppState {
        workers: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(api::root))
        .route("/status", get(api::status))
        .route("/connect", post(api::connect::connect))
        .route("/workers", get(api::workers::workers))
        .route("/disconnect", post(api::disconnect::disconnect))
        .with_state(state);

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("Failed to bind listener");
        println!("Axum server running on {}", addr);

        axum::serve(listener, app).await.unwrap();
    });

    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");

    Ok(())
}
