use axum::response::IntoResponse;

pub mod connect;
pub mod disconnect;
pub mod workers;

pub async fn root() -> impl IntoResponse {
    "Controller is online"
}

pub async fn status() -> impl IntoResponse {
    "Status OK"
}
