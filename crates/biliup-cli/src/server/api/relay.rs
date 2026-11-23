use crate::server::infrastructure::service_register::ServiceRegister;
use axum::Json;
use axum::extract::State;
use biliup_relay::RelayHealthSnapshot;

pub async fn health(State(services): State<ServiceRegister>) -> Json<RelayHealthSnapshot> {
    Json(services.relay.health())
}
