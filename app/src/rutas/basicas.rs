use std::sync::Arc;

use axum::{extract::State, http::{StatusCode, Uri}, Json};
use serde_json::{json, Value};

use crate::{error::ErrorApi, Servicio};

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

/// Cualquier ruta que no existe. Sin esto axum devuelve un 404 con el cuerpo
/// vacío y el cliente solo puede decir "respuesta ilegible"; con esto, una
/// consola más nueva que el backend sabe exactamente qué le pasa.
pub async fn desconocida(uri: Uri) -> ErrorApi {
    ErrorApi::nuevo(
        StatusCode::NOT_FOUND,
        "ruta_desconocida",
        format!("El servicio no conoce {uri}. Si la consola es más nueva que el backend, espera al despliegue."),
    )
}
