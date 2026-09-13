use std::sync::Arc;

use axum::{extract::State, Extension, Json};
use serde_json::{json, Value};

use crate::{apps::Identidad, error::ErrorApi, Servicio};

/// `GET /v1/estado`: comprueba de verdad que hablamos con OpenRouter y con qué
/// clave. Es la ruta que verifica el despliegue, por eso no gasta crédito.
pub async fn estado(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
) -> Result<Json<Value>, ErrorApi> {
    let mut clave = servicio.openrouter.info_clave().await?;
    // El uso y el límite son de la cuenta, no de la aplicación que pregunta:
    // una aplicación solo ve que la clave existe y cómo se llama.
    if !quien.admin {
        clave = json!({ "label": clave.get("label").cloned().unwrap_or(Value::Null) });
    }

    Ok(Json(json!({
        "ok": true,
        "build": servicio.config.build,
        "modelo_defecto": servicio.config.modelo_defecto,
        "identidad": {
            "nombre": quien.nombre(),
            "app_id": quien.app_id(),
            "administracion": quien.admin,
        },
        "almacen": servicio.uso.almacen(),
        "registros": servicio.uso.total(),
        "clave": clave,
    })))
}
