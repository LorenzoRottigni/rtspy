use axum::response::IntoResponse;

pub mod connect;

pub async fn root() -> impl IntoResponse {
    "Controller is online"
}

pub async fn status() -> impl IntoResponse {
    "Status OK"
}
