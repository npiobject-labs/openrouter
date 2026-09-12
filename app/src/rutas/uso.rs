use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::{
        header::{HeaderName, CONTENT_DISPOSITION, CONTENT_TYPE},
        StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::{error::ErrorApi, uso::Agrupacion, Servicio};

/// Cabecera con la que el cliente sabe qué registro mirar después. Va en
/// `expose_headers` del CORS: sin eso el navegador no la deja leer.
pub const CABECERA_USO: HeaderName = HeaderName::from_static("x-uso-id");

const POR_DEFECTO: usize = 50;

/// `GET /v1/uso`: las últimas llamadas, de la más reciente a la más antigua.
///
/// Desde la etapa 4 salen del histórico en disco, así que sobreviven a un
/// reinicio. Para ventanas de tiempo y totales está `/v1/uso/resumen`.
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
                format!("No hay registro de uso con id {id}."),
            )
        })
}

/// `GET /v1/uso/resumen`: totales por día o por modelo.
///
/// `desde` y `hasta` se comparan como texto contra la fecha ISO, así que valen
/// tanto `2026-09-12` como `2026-09-12T14:00:00Z`. Sin ellos, todo el histórico.
pub async fn resumen(
    State(servicio): State<Arc<Servicio>>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Json<Value> {
    let agrupar = Agrupacion::desde(parametros.get("agrupar").map(String::as_str));
    let filas = servicio.uso.resumen(
        intervalo(&parametros, "desde").as_deref(),
        intervalo(&parametros, "hasta").as_deref(),
        &agrupar,
    );

    // Los contadores se suman como enteros: un "llamadas: 3.0" en el JSON
    // obliga a la app que lee a redondear sin motivo.
    let cuenta = |campo: &str| -> i64 {
        filas
            .iter()
            .filter_map(|f| f.get(campo).and_then(Value::as_i64))
            .sum()
    };
    let coste: f64 = filas
        .iter()
        .filter_map(|f| f.get("coste").and_then(Value::as_f64))
        .sum();

    Json(json!({
        "object": "list",
        "agrupar": match agrupar { Agrupacion::Modelo => "modelo", Agrupacion::Dia => "dia" },
        "data": filas,
        "totales": {
            "llamadas": cuenta("llamadas"),
            "fallos": cuenta("fallos"),
            "tokens_entrada": cuenta("tokens_entrada"),
            "tokens_salida": cuenta("tokens_salida"),
            "coste": coste,
        },
    }))
}

/// `GET /v1/uso/exportar`: el histórico de un intervalo en JSON o en CSV.
///
/// El CSV va con `Content-Disposition` para que el navegador lo guarde con un
/// nombre decente en vez de pintarlo en pantalla.
pub async fn exportar(
    State(servicio): State<Arc<Servicio>>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<Response, ErrorApi> {
    let formato = parametros
        .get("formato")
        .map(String::as_str)
        .unwrap_or("json");

    let registros = servicio.uso.intervalo(
        intervalo(&parametros, "desde").as_deref(),
        intervalo(&parametros, "hasta").as_deref(),
    );

    match formato {
        "json" => Ok(Json(json!({ "object": "list", "data": registros })).into_response()),
        "csv" => {
            let mut csv = String::from(
                "id,fecha,modelo_pedido,modelo_servido,proveedor,tokens_entrada,tokens_salida,\
tokens_razonamiento,tokens_cache,coste,coste_origen,latencia_ms,motivo_fin,estado\n",
            );
            for r in &registros {
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    campo(&r.id),
                    campo(&r.fecha),
                    campo(&r.modelo_pedido),
                    campo(r.modelo_servido.as_deref().unwrap_or("")),
                    campo(r.proveedor.as_deref().unwrap_or("")),
                    r.tokens_entrada,
                    r.tokens_salida,
                    r.tokens_razonamiento,
                    r.tokens_cache,
                    r.coste.map(|c| format!("{c:.8}")).unwrap_or_default(),
                    campo(&r.coste_origen),
                    r.latencia_ms,
                    campo(r.motivo_fin.as_deref().unwrap_or("")),
                    r.estado,
                ));
            }
            Ok((
                [
                    (CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                    (
                        CONTENT_DISPOSITION,
                        "attachment; filename=\"uso.csv\"".to_string(),
                    ),
                ],
                csv,
            )
                .into_response())
        }
        otro => Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "formato_invalido",
            format!("Formato {otro} desconocido: usa csv o json."),
        )),
    }
}

/// Un parámetro de fecha, descartando el vacío para que `?desde=` no filtre.
///
/// Las fechas se comparan como texto contra la fecha ISO del registro, así que
/// un `hasta=2026-09-12` a secas dejaría fuera ese mismo día entero: se
/// completa hasta su último segundo, que es lo que espera cualquiera.
fn intervalo(parametros: &HashMap<String, String>, nombre: &str) -> Option<String> {
    let valor = parametros
        .get(nombre)
        .map(String::as_str)
        .filter(|v| !v.is_empty())?;

    Some(if nombre == "hasta" && valor.len() == 10 {
        format!("{valor}T23:59:59Z")
    } else {
        valor.to_string()
    })
}

/// Escapa un campo de CSV: comillas dobladas y todo entrecomillado si hace
/// falta. Los identificadores de modelo llevan barras y puntos, no comas, pero
/// el motivo de fin viene de fuera y no se controla.
fn campo(valor: &str) -> String {
    if valor.contains([',', '"', '\n']) {
        format!("\"{}\"", valor.replace('"', "\"\""))
    } else {
        valor.to_string()
    }
}
