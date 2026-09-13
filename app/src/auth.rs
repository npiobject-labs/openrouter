use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::{
    apps::{self, Identidad},
    error::ErrorApi,
    Servicio,
};

/// Exige `Authorization: Bearer <clave>` en todo /v1 y averigua quién llama.
///
/// La clave del despliegue (`SERVICIO_CLAVE`) es la de administración; las
/// demás son de aplicaciones dadas de alta, y pueden revocarse una a una sin
/// tocar el despliegue. Sin secreto configurado el servicio no atiende a nadie:
/// un proxy abierto en una URL pública es crédito de OpenRouter regalado.
pub async fn exigir_clave(
    State(servicio): State<Arc<Servicio>>,
    mut peticion: Request,
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

    let identidad = if iguales(recibida.as_bytes(), esperada.as_bytes()) {
        Identidad::administracion()
    } else {
        match servicio.uso.con(|c| apps::por_clave(c, recibida)) {
            Some(app) => Identidad { app: Some(app), admin: false },
            None => {
                return Err(ErrorApi::nuevo(
                    StatusCode::UNAUTHORIZED,
                    "clave_invalida",
                    "La clave no es válida o la aplicación está desactivada.",
                ))
            }
        }
    };

    // Quien llama viaja con la peticion: los handlers deciden con ella qué
    // puede ver y a quien se le anota el gasto.
    peticion.extensions_mut().insert(identidad);
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
