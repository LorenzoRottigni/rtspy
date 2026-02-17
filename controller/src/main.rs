use std::net::SocketAddr;

use axum::{
    routing::{get, post},
    Router,
};

mod api;

#[tokio::main]
async fn main() -> opencv::Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));

    let app = Router::new()
        .route("/", get(api::root))
        .route("/status", get(api::status))
        .route("/connect", post(api::connect::connect));

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
