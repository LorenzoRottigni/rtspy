use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use std::{process::Command, thread};

#[derive(Deserialize)]
pub struct ConnectRequest {
    pub url: String,
}

pub async fn connect(Json(payload): Json<ConnectRequest>) -> impl IntoResponse {
    let rtsp_url = payload.url.clone();

    thread::spawn(move || {
        // Launch the worker binary
        // Make sure to use the correct path to your binary
        let result = Command::new("./target/debug/rtspy-worker") // adjust path if needed
            .arg(rtsp_url)
            .spawn();

        match result {
            Ok(_child) => {
                println!("Worker process launched successfully");
                // _child is a Child handle, you can optionally store it
            }
            Err(e) => eprintln!("Failed to spawn worker: {:?}", e),
        }
    });

    StatusCode::OK
}
