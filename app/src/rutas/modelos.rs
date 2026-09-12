use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Query, State},
    http::{header::HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::{
    catalogo::{filtrar, Filtros, Origen},
    error::ErrorApi,
    Servicio,
};

const CABECERA_CACHE: HeaderName = HeaderName::from_static("x-cache");

/// `GET /v1/models`: el catálogo, con el formato de la API de OpenAI más los
/// campos que hacen falta para elegir modelo.
///
/// Filtros por query string: `texto`, `proveedor`, `contexto_min`, `gratis=1`.
/// Con `refrescar=1` se salta la caché.
pub async fn modelos(
    State(servicio): State<Arc<Servicio>>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, ErrorApi> {
    let refrescar = verdadero(parametros.get("refrescar"));

    let (lista, origen) = match servicio.catalogo.vigente().filter(|_| !refrescar) {
        Some(lista) => (lista, Origen::Cache),
        None => match servicio.openrouter.modelos().await {
            Ok(bruto) => (servicio.catalogo.guardar(&bruto), Origen::Fresco),
            // Sin catálogo nuevo, mejor el de ayer que ninguno.
            Err(e) => match servicio.catalogo.caducada() {
                Some(lista) => (lista, Origen::Caducada),
                None => return Err(e),
            },
        },
    };

    let filtros = Filtros {
        texto: parametros.get("texto").filter(|t| !t.is_empty()).cloned(),
        proveedor: parametros.get("proveedor").filter(|p| !p.is_empty()).cloned(),
        contexto_min: parametros.get("contexto_min").and_then(|c| c.parse().ok()),
        gratis: verdadero(parametros.get("gratis")),
    };

    let total = lista.len();
    let data = filtrar(lista, &filtros);

    let cuerpo = json!({
        "object": "list",
        "data": data,
        "total": total,
        "antiguedad_minutos": servicio.catalogo.antiguedad_minutos(),
    });

    let cabecera = HeaderValue::from_static(origen.etiqueta());
    Ok((StatusCode::OK, [(CABECERA_CACHE, cabecera)], Json(cuerpo)))
}

/// `1`, `true` y `si` valen; el resto, no. Que un `?gratis=0` no filtre nada.
fn verdadero(valor: Option<&String>) -> bool {
    matches!(valor.map(String::as_str), Some("1" | "true" | "si"))
}
