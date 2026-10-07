use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Query, State},
    http::{header::HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::{
    catalogo::{filtrar, Filtros, Modelo, Origen},
    error::ErrorApi,
    rutas::uso::upstream_desconocido,
    upstream::Upstream,
    Servicio,
};

const CABECERA_CACHE: HeaderName = HeaderName::from_static("x-cache");

/// `GET /v1/models`: el catálogo, con el formato de la API de OpenAI más los
/// campos que hacen falta para elegir modelo.
///
/// Filtros por query string: `texto`, `proveedor`, `contexto_min`, `gratis=1`
/// y, desde la etapa 9, `salida=<modalidad>`. Con `upstream=hf` se devuelve el
/// catálogo del router de Hugging Face en vez del de OpenRouter, con ids
/// `hf:...` listos para mandar en `model`. Con `refrescar=1` se salta la caché.
pub async fn modelos(
    State(servicio): State<Arc<Servicio>>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, ErrorApi> {
    let refrescar = verdadero(parametros.get("refrescar"));
    let upstream = match parametros
        .get("upstream")
        .map(String::as_str)
        .filter(|v| !v.is_empty())
    {
        None => Upstream::OpenRouter,
        Some(valor) => Upstream::desde_nombre(valor).ok_or_else(|| upstream_desconocido(valor))?,
    };

    let (lista, origen) = catalogo_de(&servicio, upstream, refrescar).await?;

    let filtros = Filtros {
        texto: parametros.get("texto").filter(|t| !t.is_empty()).cloned(),
        proveedor: parametros
            .get("proveedor")
            .filter(|p| !p.is_empty())
            .cloned(),
        contexto_min: parametros.get("contexto_min").and_then(|c| c.parse().ok()),
        gratis: verdadero(parametros.get("gratis")),
        salida: parametros.get("salida").filter(|s| !s.is_empty()).cloned(),
    };

    let total = lista.len();
    let data = filtrar(lista, &filtros);

    let cuerpo = json!({
        "object": "list",
        "data": data,
        "total": total,
        "upstream": upstream,
        "antiguedad_minutos": servicio.catalogo.antiguedad_minutos(upstream),
    });

    let cabecera = HeaderValue::from_static(origen.etiqueta());
    Ok((StatusCode::OK, [(CABECERA_CACHE, cabecera)], Json(cuerpo)))
}

/// El catálogo de un upstream: de la caché si está vigente, si no se pide y se
/// guarda, y si el upstream no responde se sirve la copia caducada antes que
/// un error. Lo usan `/v1/models` y la estimación de coste de una consulta
/// `hf:`, que necesita el catálogo de Hugging Face aunque nadie lo haya
/// pedido todavía.
pub async fn catalogo_de(
    servicio: &Servicio,
    upstream: Upstream,
    refrescar: bool,
) -> Result<(Vec<Modelo>, Origen), ErrorApi> {
    if let Some(lista) = servicio.catalogo.vigente(upstream).filter(|_| !refrescar) {
        return Ok((lista, Origen::Cache));
    }
    match servicio.openrouter.modelos(upstream).await {
        Ok(bruto) => Ok((servicio.catalogo.guardar(upstream, &bruto), Origen::Fresco)),
        // Sin catálogo nuevo, mejor el de ayer que ninguno.
        Err(e) => match servicio.catalogo.caducada(upstream) {
            Some(lista) => Ok((lista, Origen::Caducada)),
            None => Err(e),
        },
    }
}

/// `1`, `true` y `si` valen; el resto, no. Que un `?gratis=0` no filtre nada.
fn verdadero(valor: Option<&String>) -> bool {
    matches!(valor.map(String::as_str), Some("1" | "true" | "si"))
}
