use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde_json::{json, Map, Value};

use crate::{alias, apps::Identidad, error::ErrorApi, Servicio};

/// Leer los alias lo puede hacer cualquiera con clave: una aplicación necesita
/// saber qué nombres tiene a mano. Escribirlos es de administración, porque
/// cambiar un alias cambia a qué modelo salen todas las llamadas que lo usan.
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

fn desconocido(nombre: &str) -> ErrorApi {
    ErrorApi::nuevo(
        StatusCode::NOT_FOUND,
        "alias_desconocido",
        format!("No hay ningún alias llamado {nombre}."),
    )
}

fn invalido(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::nuevo(StatusCode::BAD_REQUEST, "cuerpo_invalido", mensaje)
}

/// `GET /v1/alias`: los alias dados de alta, con su cadena y sus parámetros.
pub async fn lista(State(servicio): State<Arc<Servicio>>) -> Json<Value> {
    let alias = servicio.uso.con(alias::lista);
    Json(json!({ "object": "list", "data": alias, "total": alias.len() }))
}

/// `PUT /v1/alias/{nombre}`: crea o reemplaza un alias entero.
///
/// Es un reemplazo, no un parche: un alias es corto y se entiende mejor
/// mandándolo completo que parcheándolo a trozos. Lo que no venga, se queda sin
/// poner.
pub async fn guarda(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(nombre): Path<String>,
    Json(cuerpo): Json<Value>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;

    if !alias::nombre_valido(&nombre) {
        return Err(invalido(
            "El nombre de un alias son minúsculas, dígitos y guiones, hasta 40 caracteres.",
        ));
    }

    let modelo = cuerpo
        .get("modelo")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| invalido("Hace falta un \"modelo\" al que apunte el alias."))?
        .to_string();

    let respaldos = match cuerpo.get("respaldos") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(lista)) => {
            let mut salida = Vec::new();
            for v in lista {
                let m = v
                    .as_str()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .ok_or_else(|| invalido("Los \"respaldos\" son nombres de modelo."))?;
                salida.push(m.to_string());
            }
            salida
        }
        Some(_) => return Err(invalido("\"respaldos\" tiene que ser una lista.")),
    };

    let parametros = match cuerpo.get("parametros") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(o)) => {
            // Un alias fija parámetros del modelo, no de la llamada: dejarle
            // tocar el cuerpo entero rompería la consulta o la medición.
            for reservado in alias::RESERVADOS {
                if o.contains_key(reservado) {
                    return Err(invalido(format!(
                        "Un alias no puede fijar \"{reservado}\": eso lo decide cada llamada."
                    )));
                }
            }
            o.clone()
        }
        Some(_) => return Err(invalido("\"parametros\" tiene que ser un objeto.")),
    };

    let nuevo = alias::Alias {
        nombre: nombre.clone(),
        modelo,
        respaldos,
        parametros,
        nota: cuerpo
            .get("nota")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| n.chars().take(200).collect()),
        actualizado: crate::uso::ahora_iso(),
    };

    servicio
        .uso
        .con(|c| alias::guarda(c, &nuevo))
        .map_err(|e| {
            ErrorApi::nuevo(
                StatusCode::INTERNAL_SERVER_ERROR,
                "alias_no_guardado",
                format!("No se pudo guardar el alias: {e}"),
            )
        })?;

    Ok(Json(json!({ "ok": true, "alias": nuevo })))
}

/// `DELETE /v1/alias/{nombre}`: lo borra de verdad.
///
/// Al revés que una aplicación, que se desactiva: el histórico guarda el nombre
/// del alias como texto, así que un informe viejo sigue diciendo por dónde salió
/// la llamada aunque el alias ya no exista.
pub async fn borra(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(nombre): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;

    let hecho = servicio
        .uso
        .con(|c| alias::borra(c, &nombre))
        .unwrap_or(false);
    if !hecho {
        return Err(desconocido(&nombre));
    }
    Ok(Json(json!({ "ok": true, "nombre": nombre, "borrado": true })))
}

/// `GET /v1/alias/{nombre}/simular`: a qué saldría una llamada por este alias,
/// sin llamar a nadie ni gastar un céntimo.
///
/// Devuelve la cadena entera con el precio de cada escalón, para poder ver de
/// un vistazo cuánto encarece el respaldo si el primero no responde.
pub async fn simula(
    State(servicio): State<Arc<Servicio>>,
    Path(nombre): Path<String>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ErrorApi> {
    let Some(a) = servicio.uso.con(|c| alias::una(c, &nombre)) else {
        return Err(desconocido(&nombre));
    };

    // Un tamaño de ejemplo para poder comparar: mil de entrada y quinientos de
    // salida es una consulta corriente. Se puede cambiar con ?entrada= y ?salida=.
    let entrada: f64 = parametros
        .get("entrada")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000.0);
    let salida: f64 = parametros
        .get("salida")
        .and_then(|v| v.parse().ok())
        .unwrap_or(500.0);

    let cadena: Vec<Value> = a
        .cadena()
        .iter()
        .enumerate()
        .map(|(orden, modelo)| {
            let precios = servicio.catalogo.precio(modelo);
            json!({
                "orden": orden,
                "modelo": modelo,
                "papel": if orden == 0 { "principal" } else { "respaldo" },
                // Sin catálogo cargado no se sabe el precio; no es un error,
                // es que todavía no se ha pedido la lista a OpenRouter.
                "conocido": precios.is_some(),
                "precio_entrada": precios.map(|(e, _)| e),
                "precio_salida": precios.map(|(_, s)| s),
                "coste_estimado": precios
                    .map(|(e, s)| (entrada / 1_000_000.0) * e + (salida / 1_000_000.0) * s),
            })
        })
        .collect();

    Ok(Json(json!({
        "alias": a,
        "muestra": { "tokens_entrada": entrada, "tokens_salida": salida },
        "cadena": cadena,
    })))
}
