use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{error::ErrorApi, Servicio};

/// `GET /v1/estado`: comprueba de verdad que hablamos con OpenRouter y con qué
/// clave. Es la ruta que verifica el despliegue, por eso no gasta crédito.
pub async fn estado(State(servicio): State<Arc<Servicio>>) -> Result<Json<Value>, ErrorApi> {
    let clave = servicio.openrouter.info_clave().await?;

    Ok(Json(json!({
        "ok": true,
        "build": servicio.config.build,
        "modelo_defecto": servicio.config.modelo_defecto,
        "clave": clave,
    })))
}
