use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use serde_json::{json, Value};

use crate::{
    apps::{self, Identidad},
    error::ErrorApi,
    Servicio,
};

/// Las rutas de aplicaciones son de administración: con una clave de aplicación
/// no se pueden crear ni listar otras.
fn solo_administracion(quien: &Identidad) -> Result<(), ErrorApi> {
    if quien.admin {
        return Ok(());
    }
    Err(ErrorApi::nuevo(
        StatusCode::FORBIDDEN,
        "solo_administracion",
        "Esta ruta exige la clave de administración del servicio.",
    ))
}

/// `POST /v1/apps`: da de alta una aplicación y devuelve su clave.
///
/// La clave se enseña **una sola vez**: el servicio guarda solo su hash, así
/// que no hay forma de recuperarla después. Si se pierde, se da de baja esa
/// aplicación y se crea otra.
pub async fn alta(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Json(cuerpo): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ErrorApi> {
    solo_administracion(&quien)?;

    let nombre = cuerpo
        .get("nombre")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::BAD_REQUEST,
                "cuerpo_invalido",
                "Hace falta un \"nombre\" para la aplicación.",
            )
        })?;

    let fecha = crate::uso::ahora_iso();
    let (app, clave) = servicio
        .uso
        .con(|c| apps::crea(c, nombre, &fecha))
        .map_err(|e| {
            ErrorApi::nuevo(
                StatusCode::INTERNAL_SERVER_ERROR,
                "alta_fallida",
                format!("No se pudo dar de alta la aplicación: {e}"),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "app": app,
            "clave": clave,
            "aviso": "Guarda la clave ahora: no se puede volver a consultar.",
        })),
    ))
}

/// `GET /v1/apps`: las aplicaciones dadas de alta, con o sin clave activa.
pub async fn lista(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;
    let apps = servicio.uso.con(apps::lista);
    Ok(Json(json!({ "object": "list", "data": apps, "total": apps.len() })))
}

/// `DELETE /v1/apps/{id}`: desactiva la aplicación. No la borra, porque el
/// histórico de uso la sigue referenciando.
pub async fn baja(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;

    let hecho = servicio
        .uso
        .con(|c| apps::desactiva(c, &id))
        .unwrap_or(false);

    if !hecho {
        return Err(ErrorApi::nuevo(
            StatusCode::NOT_FOUND,
            "app_desconocida",
            format!("No hay ninguna aplicación con id {id}."),
        ));
    }
    Ok(Json(json!({ "ok": true, "id": id, "activa": false })))
}
