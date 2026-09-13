use std::{sync::Arc, time::Duration, time::Instant};

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::Value;

use crate::{apps::Identidad, error::ErrorApi, rutas::uso::CABECERA_USO, Servicio};

/// Cuánto se espera entre intentos de reconciliación. OpenRouter tarda un poco
/// en dejar lista la contabilidad de una generación.
const REINTENTOS: [u64; 3] = [400, 1200, 3000];

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

    if objeto.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "streaming_no_disponible",
            "El streaming llega en la etapa 7; manda la consulta sin \"stream\".",
        ));
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

    let mut registro = servicio.uso.abre(pedido.clone());
    registro.app_id = quien.app_id();
    // El trabajo de negocio al que pertenece la llamada: varias consultas de un
    // mismo presupuesto se agrupan luego por aqui.
    registro.operacion = cabeceras
        .get("x-operacion")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().chars().take(120).collect::<String>())
        .filter(|v| !v.is_empty());
    let id_uso = registro.id.clone();

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

            Ok(con_uso(&id_uso, (StatusCode::OK, Json(respuesta))))
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

fn con_uso(id: &str, respuesta: impl IntoResponse) -> Response {
    match HeaderValue::from_str(id) {
        Ok(valor) => ([(CABECERA_USO, valor)], respuesta).into_response(),
        Err(_) => respuesta.into_response(),
    }
}

/// Pregunta a OpenRouter qué costó de verdad la generación y lo sustituye en el
/// registro. Va en segundo plano: el cliente ya tiene su respuesta y no debe
/// esperar por esto. Si no sale, se queda la estimación del catálogo.
fn reconcilia(servicio: Arc<Servicio>, id_uso: String, generacion: String) {
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
