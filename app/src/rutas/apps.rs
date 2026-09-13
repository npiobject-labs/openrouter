use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde_json::{json, Value};

use crate::{
    apps::{self, Identidad, Limites},
    error::ErrorApi,
    guardia,
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

/// `GET /v1/presupuesto`: cuánto le queda a quien llama, para que decida antes
/// de gastar. Con clave de administración, `?app=` dice de cuál.
pub async fn presupuesto(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ErrorApi> {
    let app = match (&quien.app, parametros.get("app")) {
        (Some(propia), _) => Some(propia.clone()),
        (None, Some(id)) => servicio.uso.con(|c| apps::una(c, id)),
        (None, None) => None,
    };

    let Some(app) = app else {
        // La administración no tiene presupuesto: es la dueña del servicio.
        return Ok(Json(json!({
            "administracion": true,
            "aviso": "La clave de administración no tiene límites. Pide ?app=<id> para ver el de una aplicación.",
        })));
    };

    let margen = guardia::margen_de(&servicio, &app);
    Ok(Json(guardia::como_json(&app, &margen)))
}

/// `PUT /v1/apps/{id}/presupuesto`: fija o quita los topes de una aplicación.
///
/// Lo que no venga en el cuerpo se borra: es un reemplazo, no un parche, para
/// que quitar un límite sea tan fácil como ponerlo.
pub async fn limites(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(id): Path<String>,
    Json(cuerpo): Json<Value>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;

    let periodo = cuerpo.get("periodo").and_then(Value::as_str).map(str::to_string);
    if let Some(p) = &periodo {
        if p != "dia" && p != "mes" {
            return Err(ErrorApi::nuevo(
                StatusCode::BAD_REQUEST,
                "periodo_invalido",
                "El periodo tiene que ser \"dia\" o \"mes\".",
            ));
        }
    }

    let nuevos = Limites {
        periodo,
        limite: cuerpo.get("limite").and_then(Value::as_f64),
        aviso: cuerpo.get("aviso").and_then(Value::as_f64),
        cuota_minuto: cuerpo.get("cuota_minuto").and_then(Value::as_i64),
    };

    if nuevos.limite.is_some() && nuevos.periodo.is_none() {
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "periodo_invalido",
            "Un presupuesto sin periodo no se puede aplicar: manda también \"periodo\".",
        ));
    }

    let hecho = servicio
        .uso
        .con(|c| apps::limita(c, &id, &nuevos))
        .unwrap_or(false);
    if !hecho {
        return Err(ErrorApi::nuevo(
            StatusCode::NOT_FOUND,
            "app_desconocida",
            format!("No hay ninguna aplicación con id {id}."),
        ));
    }

    let app = servicio.uso.con(|c| apps::una(c, &id)).ok_or_else(|| {
        ErrorApi::nuevo(
            StatusCode::NOT_FOUND,
            "app_desconocida",
            format!("No hay ninguna aplicación con id {id}."),
        )
    })?;
    let margen = guardia::margen_de(&servicio, &app);
    Ok(Json(guardia::como_json(&app, &margen)))
}
