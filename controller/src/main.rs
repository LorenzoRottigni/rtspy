use std::net::SocketAddr;
use std::thread;

mod http;
mod types;
mod viewer;

use http::start_server;
use types::StreamerUrl;
use viewer::start_viewer;

#[tokio::main]
async fn main() -> opencv::Result<()> {
    // --- Start Axum server ---
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    start_server(addr); // runs via tokio::spawn inside http.rs

    // --- Start viewer on a separate thread ---
    // let rtsp_url: StreamerUrl = "rtsp://127.0.0.1:8554/live".to_string();
    // thread::spawn(move || {
    //     start_viewer(&rtsp_url, "Controller View").unwrap();
    // });

    // Keep main alive while Axum is running
    // Could use tokio::signal::ctrl_c() or just sleep forever
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");

    Ok(())
}
