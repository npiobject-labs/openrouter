use std::{sync::Arc, time::Duration, time::Instant};

use axum::{
    body::Body,
    extract::State,
    http::{
        header::{HeaderName, CONTENT_TYPE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::{
    apps::Identidad, error::ErrorApi, flujo::Medidor, guardia, rutas::uso::CABECERA_USO, Servicio,
};

/// Cuánto se espera entre intentos de reconciliación. OpenRouter tarda un poco
/// en dejar lista la contabilidad de una generación.
const REINTENTOS: [u64; 3] = [400, 1200, 3000];

/// Avisa de que el presupuesto se está acabando sin cortar la llamada.
const CABECERA_AVISO: HeaderName = HeaderName::from_static("x-presupuesto");

/// `POST /v1/chat/completions`: proxy fino con el contrato de OpenAI.
///
/// El cuerpo se reenvía tal cual salvo dos retoques: se rellena `model` si no
/// viene, y se rechaza `stream`, que es de la etapa 7. Toda llamada que llega a
/// salir queda anotada en el registro de uso, vaya bien o mal, y su id viaja de
/// vuelta en la cabecera `X-Uso-Id`.
pub async fn chat(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    cabeceras: HeaderMap,
    Json(mut cuerpo): Json<Value>,
) -> Result<Response, ErrorApi> {
    let objeto = cuerpo.as_object_mut().ok_or_else(|| {
        ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "cuerpo_invalido",
            "El cuerpo tiene que ser un objeto JSON.",
        )
    })?;

    let en_flujo = objeto.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if en_flujo {
        // Sin esto, el último evento no trae usage y la llamada no se puede
        // medir: el coste de un flujo se sabría solo por la reconciliación.
        objeto.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }

    // Sin modelo, el de la casa: así una app puede empezar a llamar sin elegir.
    if !objeto.contains_key("model") {
        objeto.insert(
            "model".to_string(),
            Value::String(servicio.config.modelo_defecto.clone()),
        );
    }

    let pedido = objeto
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // La huella identifica una consulta repetida; se calcula sobre el cuerpo ya
    // completado, para que dos peticiones sin modelo cuenten como la misma.
    let huella = crate::apps::hash(&cuerpo.to_string());
    let aviso = guardia::comprueba(&servicio, &quien, &huella)?;

    let mut registro = servicio.uso.abre(pedido.clone());
    registro.app_id = quien.app_id();
    registro.huella = Some(huella);
    // El trabajo de negocio al que pertenece la llamada: varias consultas de un
    // mismo presupuesto se agrupan luego por aqui.
    registro.operacion = cabeceras
        .get("x-operacion")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().chars().take(120).collect::<String>())
        .filter(|v| !v.is_empty());
    let id_uso = registro.id.clone();

    if en_flujo {
        return en_directo(servicio, cuerpo, registro, id_uso, aviso).await;
    }

    let reloj = Instant::now();
    let resultado = servicio.openrouter.chat(cuerpo).await;
    registro.latencia_ms = reloj.elapsed().as_millis() as i64;

    match resultado {
        Ok(respuesta) => {
            registro.estado = StatusCode::OK.as_u16();
            registro.desde_respuesta(&respuesta);
            // El modelo servido puede no ser el pedido: OpenRouter enruta.
            let servido = registro.modelo_servido.clone().unwrap_or(pedido);
            registro.estima(servicio.catalogo.precio(&servido));

            let pendiente = registro.id_openrouter.clone();
            servicio.uso.anota(registro);
            if let Some(generacion) = pendiente {
                reconcilia(servicio.clone(), id_uso.clone(), generacion);
            }

            Ok(con_uso(&id_uso, aviso.as_deref(), (StatusCode::OK, Json(respuesta))))
        }
        Err(fallo) => {
            // Un fallo también consumió tiempo, y a veces crédito: se anota.
            registro.estado = fallo.estado.as_u16();
            registro.motivo_fin = Some(fallo.codigo.to_string());
            servicio.uso.anota(registro);
            Err(fallo.con_uso(&id_uso))
        }
    }
}

fn con_uso(id: &str, aviso: Option<&str>, respuesta: impl IntoResponse) -> Response {
    let mut cabeceras = HeaderMap::new();
    if let Ok(valor) = HeaderValue::from_str(id) {
        cabeceras.insert(CABECERA_USO, valor);
    }
    // El aviso de presupuesto viaja en cabecera: no cambia el cuerpo, que es de
    // OpenRouter, y una aplicacion puede mirarlo sin parsear nada.
    if let Some(valor) = aviso.and_then(|a| HeaderValue::from_str(a).ok()) {
        cabeceras.insert(CABECERA_AVISO, valor);
    }
    (cabeceras, respuesta).into_response()
}

/// Pregunta a OpenRouter qué costó de verdad la generación y lo sustituye en el
/// registro. Va en segundo plano: el cliente ya tiene su respuesta y no debe
/// esperar por esto. Si no sale, se queda la estimación del catálogo.
pub fn reconcilia(servicio: Arc<Servicio>, id_uso: String, generacion: String) {
    tokio::spawn(async move {
        for espera in REINTENTOS {
            tokio::time::sleep(Duration::from_millis(espera)).await;

            let Ok(datos) = servicio.openrouter.generacion(&generacion).await else {
                continue;
            };
            let coste = datos.get("total_cost").and_then(Value::as_f64);
            if let Some(coste) = coste {
                let proveedor = datos
                    .get("provider_name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                servicio.uso.reconcilia(&id_uso, coste, proveedor);
                return;
            }
        }
    });
}

/// La misma consulta, servida según llega.
///
/// El cuerpo se reenvía tal cual: son eventos de OpenRouter y un cliente de la
/// API de OpenAI los entiende sin tocar nada. Lo único que añade el servicio es
/// un medidor que mira los eventos de pasada y, al cerrarse el flujo, escribe
/// el registro de uso.
async fn en_directo(
    servicio: Arc<Servicio>,
    cuerpo: Value,
    registro: crate::uso::Registro,
    id_uso: String,
    aviso: Option<String>,
) -> Result<Response, ErrorApi> {
    let respuesta = match servicio.openrouter.chat_en_flujo(cuerpo).await {
        Ok(r) => r,
        Err(fallo) => {
            // Un rechazo antes de abrir el flujo se anota como cualquier otro.
            let mut registro = registro;
            registro.estado = fallo.estado.as_u16();
            registro.motivo_fin = Some(fallo.codigo.to_string());
            servicio.uso.anota(registro);
            return Err(fallo.con_uso(&id_uso));
        }
    };

    let mut registro = registro;
    registro.estado = StatusCode::OK.as_u16();

    let mut medidor = Medidor::nuevo(servicio.clone(), registro);
    let eventos = respuesta.bytes_stream().map(move |trozo| {
        if let Ok(bytes) = &trozo {
            medidor.anota(bytes);
        }
        trozo
    });

    let mut cabeceras = HeaderMap::new();
    cabeceras.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    if let Ok(valor) = HeaderValue::from_str(&id_uso) {
        cabeceras.insert(CABECERA_USO, valor);
    }
    if let Some(valor) = aviso.as_deref().and_then(|a| HeaderValue::from_str(a).ok()) {
        cabeceras.insert(CABECERA_AVISO, valor);
    }

    Ok((cabeceras, Body::from_stream(eventos)).into_response())
}
