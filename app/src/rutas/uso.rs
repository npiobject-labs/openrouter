use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::{header::HeaderName, StatusCode},
    Json,
};
use serde_json::{json, Value};

use crate::{error::ErrorApi, Servicio};

/// Cabecera con la que el cliente sabe qué registro mirar después. Va en
/// `expose_headers` del CORS: sin eso el navegador no la deja leer.
pub const CABECERA_USO: HeaderName = HeaderName::from_static("x-uso-id");

const POR_DEFECTO: usize = 50;

/// `GET /v1/uso`: las últimas llamadas, de la más reciente a la más antigua.
pub async fn lista(
    State(servicio): State<Arc<Servicio>>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Json<Value> {
    let n = parametros
        .get("n")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(POR_DEFECTO)
        .clamp(1, 1000);

    Json(json!({
        "object": "list",
        "data": servicio.uso.ultimos(n),
        "total": servicio.uso.total(),
    }))
}

/// `GET /v1/uso/{id}`: una llamada concreta, la que devolvió `X-Uso-Id`.
pub async fn una(
    State(servicio): State<Arc<Servicio>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    servicio
        .uso
        .uno(&id)
        .map(|r| Json(json!(r)))
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::NOT_FOUND,
                "uso_desconocido",
                format!("No hay registro de uso con id {id}. Solo se guardan las últimas 1000 llamadas, y se pierden al reiniciar el servicio."),
            )
        })
}
