use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::Value;

use crate::{error::ErrorApi, Servicio};

/// `POST /v1/chat/completions`: proxy fino con el contrato de OpenAI.
///
/// El cuerpo se reenvía tal cual salvo dos retoques: se rellena `model` si no
/// viene, y se rechaza `stream`, que es de la etapa 7.
pub async fn chat(
    State(servicio): State<Arc<Servicio>>,
    Json(mut cuerpo): Json<Value>,
) -> Result<impl IntoResponse, ErrorApi> {
    let objeto = cuerpo.as_object_mut().ok_or_else(|| {
        ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "cuerpo_invalido",
            "El cuerpo tiene que ser un objeto JSON.",
        )
    })?;

    if objeto.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "streaming_no_disponible",
            "El streaming llega en la etapa 7; manda la consulta sin \"stream\".",
        ));
    }

    // Sin modelo, el de la casa: así una app puede empezar a llamar sin elegir.
    if !objeto.contains_key("model") {
        objeto.insert(
            "model".to_string(),
            Value::String(servicio.config.modelo_defecto.clone()),
        );
    }

    let (estado, respuesta) = servicio.openrouter.chat(cuerpo).await?;
    Ok((estado, Json(respuesta)))
}
