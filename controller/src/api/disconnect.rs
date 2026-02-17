use axum::{extract::Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct DisconnectRequest {
    pub url: String,
}

pub async fn disconnect(
    State(state): State<crate::AppState>,
    Json(payload): Json<DisconnectRequest>,
) -> impl IntoResponse {
    let mut workers = state.workers.lock().await;
    let child = workers.remove(&payload.url);
    drop(workers);

    match child {
        Some(mut child) => match child.kill().await {
            Ok(_) => StatusCode::OK,
            Err(e) => {
                eprintln!("Failed to stop worker for {}: {e:?}", payload.url);
                StatusCode::INTERNAL_SERVER_ERROR
            }
        },
        None => StatusCode::NOT_FOUND,
    }
}
