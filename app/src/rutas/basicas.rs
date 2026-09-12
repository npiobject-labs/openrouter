use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::Servicio;

pub async fn raiz() -> &'static str {
    "openrouter backend"
}

/// Prueba "hola mundo" que consume docs/holamundo.html desde Pages.
pub async fn holamundo() -> &'static str {
    "holamundo"
}

/// Salud del proceso. No llama a OpenRouter: dice si el servicio está en pie y
/// con qué build, nada más.
pub async fn salud(State(servicio): State<Arc<Servicio>>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "build": servicio.config.build,
        "openrouter": servicio.config.clave_openrouter.is_some(),
        "clave_servicio": servicio.config.clave_servicio.is_some(),
    }))
}
