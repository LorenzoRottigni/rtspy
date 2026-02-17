use axum::{
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use std::{net::SocketAddr, thread};

/// Start the Axum server in a new Tokio task
pub fn start_server(addr: SocketAddr) {
    // build app
    let app = Router::new()
        .route("/", get(root))
        .route("/status", get(status))
        .route("/connect", post(connect_rtsp));

    // run server in background
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("Failed to bind listener");
        println!("Axum server running on {}", addr);

        axum::serve(listener, app).await.unwrap();
    });
}

async fn root() -> impl IntoResponse {
    "Controller is online"
}

async fn status() -> impl IntoResponse {
    "Status OK"
}

async fn connect_rtsp(body: String) -> impl IntoResponse {
    let rtsp_url = body.trim().to_string();
    println!("Connect requested to RTSP URL: {}", rtsp_url);

    // Spawn a thread to start viewing
    thread::spawn(move || {
        // Call your existing viewer logic
        crate::viewer::start_viewer(&rtsp_url, "Controller View").unwrap();
    });
}
