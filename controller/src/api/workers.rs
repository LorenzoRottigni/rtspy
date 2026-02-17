use axum::{extract::State, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct WorkerInfo {
    pub url: String,
    pub pid: Option<u32>,
}

pub async fn workers(State(state): State<crate::AppState>) -> Json<Vec<WorkerInfo>> {
    let workers = state.workers.lock().await;
    let active_workers = workers
        .iter()
        .map(|(url, child)| WorkerInfo {
            url: url.clone(),
            pid: child.id(),
        })
        .collect();

    Json(active_workers)
}
