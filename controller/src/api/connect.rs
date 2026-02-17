use axum::{extract::State, extract::Json, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
pub struct ConnectRequest {
    pub url: String,
}

pub async fn connect(
    State(state): State<crate::AppState>,
    Json(payload): Json<ConnectRequest>,
) -> impl IntoResponse {
    let rtsp_url = payload.url;
    let mut workers = state.workers.lock().await;

    if workers.contains_key(&rtsp_url) {
        return StatusCode::CONFLICT;
    }

    match Command::new("./target/debug/rtspy-worker")
        .arg(&rtsp_url)
        .spawn()
    {
        Ok(child) => {
            workers.insert(rtsp_url, child);
            StatusCode::ACCEPTED
        }
        Err(e) => {
            eprintln!("Failed to spawn worker: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
