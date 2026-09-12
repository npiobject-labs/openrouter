use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::{error::ErrorApi, Servicio};

/// Exige `Authorization: Bearer <clave de servicio>` en todo /v1.
///
/// Sin `SERVICIO_CLAVE` configurada el servicio no atiende a nadie: un proxy
/// abierto en una URL pública es crédito de OpenRouter regalado.
pub async fn exigir_clave(
    State(servicio): State<Arc<Servicio>>,
    peticion: Request,
    siguiente: Next,
) -> Result<Response, ErrorApi> {
    let esperada = servicio
        .config
        .clave_servicio
        .as_deref()
        .ok_or_else(|| ErrorApi::sin_configurar("clave de servicio"))?;

    let recibida = peticion
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::UNAUTHORIZED,
                "sin_clave",
                "Falta la cabecera Authorization: Bearer <clave de servicio>.",
            )
        })?;

    if !iguales(recibida.as_bytes(), esperada.as_bytes()) {
        return Err(ErrorApi::nuevo(
            StatusCode::UNAUTHORIZED,
            "clave_invalida",
            "La clave de servicio no es válida.",
        ));
    }

    Ok(siguiente.run(peticion).await)
}

/// Comparación en tiempo constante: no queremos que el tiempo de respuesta
/// diga cuántos caracteres de la clave se han acertado.
fn iguales(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
