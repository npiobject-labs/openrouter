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
    alias, apps::Identidad, error::ErrorApi, flujo::Medidor, guardia, rutas::uso::CABECERA_USO,
    Servicio,
};

/// Cuánto se espera entre intentos de reconciliación. OpenRouter tarda un poco
/// en dejar lista la contabilidad de una generación.
const REINTENTOS: [u64; 3] = [400, 1200, 3000];

/// Avisa de que el presupuesto se está acabando sin cortar la llamada.
const CABECERA_AVISO: HeaderName = HeaderName::from_static("x-presupuesto");

/// `POST /v1/chat/completions`: proxy fino con el contrato de OpenAI.
///
/// El cuerpo se reenvía tal cual salvo tres retoques: se rellena `model` si no
/// viene, se resuelve si pide un alias, y con `stream` se fuerza el `usage` del
/// último evento. Toda llamada que llega a salir queda anotada en el registro de
/// uso, vaya bien o mal, y su id viaja de vuelta en la cabecera `X-Uso-Id`.
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
    let pedido = objeto
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| servicio.config.modelo_defecto.clone());

    // Un alias no es un modelo: detrás hay una cadena de modelos que se
    // intentan en orden, y unos parámetros por defecto que no pisan lo que
    // mande quien llama.
    let (nombre_alias, cadena) = match alias::pedido(&pedido) {
        Some(nombre) => {
            let a = servicio
                .uso
                .con(|c| alias::una(c, nombre))
                .ok_or_else(|| {
                    ErrorApi::nuevo(
                        StatusCode::NOT_FOUND,
                        "alias_desconocido",
                        format!("No hay ningún alias llamado {nombre}."),
                    )
                })?;
            a.aplica(objeto);
            (Some(nombre.to_string()), a.cadena())
        }
        None => (None, vec![pedido]),
    };

    // El cuerpo sale ya con un modelo de verdad: OpenRouter no sabe de alias.
    objeto.insert("model".to_string(), Value::String(cadena[0].clone()));

    // La huella identifica una consulta repetida; se calcula sobre el cuerpo ya
    // completado, para que dos peticiones sin modelo cuenten como la misma.
    let huella = crate::apps::hash(&cuerpo.to_string());
    let aviso = guardia::comprueba(&servicio, &quien, &huella)?;

    let mut registro = servicio.uso.abre(cadena[0].clone());
    registro.app_id = quien.app_id();
    registro.alias = nombre_alias;
    registro.huella = Some(huella);
    // El trabajo de negocio al que pertenece la llamada: varias consultas de un
    // mismo presupuesto se agrupan luego por aqui.
    registro.operacion = cabeceras
        .get("x-operacion")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().chars().take(120).collect::<String>())
        .filter(|v| !v.is_empty());
    let id_uso = registro.id.clone();

    // Un solo reloj para los dos caminos, y arranca aquí: lo que tarda el
    // primer token es sobre todo lo que tarda OpenRouter en contestar.
    let reloj = Instant::now();

    if en_flujo {
        return en_directo(servicio, cuerpo, cadena, registro, id_uso, aviso, reloj).await;
    }

    let (intentado, resultado) = por_la_cadena(&cadena, |modelo| {
        let mut cuerpo = cuerpo.clone();
        cuerpo["model"] = Value::String(modelo.to_string());
        let servicio = servicio.clone();
        async move { servicio.openrouter.chat(cuerpo).await }
    })
    .await;
    registro.latencia_ms = reloj.elapsed().as_millis() as i64;
    registro.modelo_pedido = intentado;

    match resultado {
        Ok(respuesta) => {
            registro.estado = StatusCode::OK.as_u16();
            registro.desde_respuesta(&respuesta);
            // El modelo servido puede no ser el pedido: OpenRouter enruta.
            let servido = registro
                .modelo_servido
                .clone()
                .unwrap_or_else(|| registro.modelo_pedido.clone());
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
    cadena: Vec<String>,
    registro: crate::uso::Registro,
    id_uso: String,
    aviso: Option<String>,
    reloj: Instant,
) -> Result<Response, ErrorApi> {
    // El respaldo solo cabe aquí, antes de abrir el flujo: una vez ha salido la
    // primera cabecera no hay forma de cambiar de modelo sin mentir al cliente.
    let (intentado, resultado) = por_la_cadena(&cadena, |modelo| {
        let mut cuerpo = cuerpo.clone();
        cuerpo["model"] = Value::String(modelo.to_string());
        let servicio = servicio.clone();
        async move { servicio.openrouter.chat_en_flujo(cuerpo).await }
    })
    .await;

    let mut registro = registro;
    registro.modelo_pedido = intentado;

    let respuesta = match resultado {
        Ok(r) => r,
        Err(fallo) => {
            // Un rechazo antes de abrir el flujo se anota como cualquier otro.
            registro.latencia_ms = reloj.elapsed().as_millis() as i64;
            registro.estado = fallo.estado.as_u16();
            registro.motivo_fin = Some(fallo.codigo.to_string());
            servicio.uso.anota(registro);
            return Err(fallo.con_uso(&id_uso));
        }
    };

    registro.estado = StatusCode::OK.as_u16();

    let mut medidor = Medidor::nuevo(servicio.clone(), registro, reloj);
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

/// Cuándo tiene sentido probar el siguiente modelo de la cadena: cuando el fallo
/// es del proveedor o del modelo, no de la consulta. Un cuerpo mal formado o una
/// clave sin crédito fallan igual en todos, y reintentarlos solo gasta tiempo.
fn se_reintenta(estado: StatusCode) -> bool {
    estado == StatusCode::TOO_MANY_REQUESTS
        || estado == StatusCode::NOT_FOUND
        || estado == StatusCode::REQUEST_TIMEOUT
        || estado.is_server_error()
}

/// Intenta la cadena de modelos en orden y devuelve el que se llegó a usar junto
/// con su resultado. El primero que responde manda; si ninguno responde, vuelve
/// el fallo del último, que es el que más se acerca a la verdad.
async fn por_la_cadena<T, F, Fut>(cadena: &[String], mut intento: F) -> (String, Result<T, ErrorApi>)
where
    F: FnMut(&str) -> Fut,
    Fut: std::future::Future<Output = Result<T, ErrorApi>>,
{
    let mut ultimo = None;
    for (orden, modelo) in cadena.iter().enumerate() {
        match intento(modelo).await {
            Ok(valor) => return (modelo.clone(), Ok(valor)),
            Err(fallo) => {
                let hay_mas = orden + 1 < cadena.len();
                if !hay_mas || !se_reintenta(fallo.estado) {
                    return (modelo.clone(), Err(fallo));
                }
                eprintln!(
                    "{modelo} respondió {} ({}); se prueba el respaldo",
                    fallo.estado, fallo.codigo
                );
                ultimo = Some(modelo.clone());
            }
        }
    }
    // Una cadena vacía no se construye en ninguna parte, pero el tipo lo admite.
    (
        ultimo.unwrap_or_default(),
        Err(ErrorApi::nuevo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cadena_vacia",
            "El alias no tiene ningún modelo al que salir.",
        )),
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cadena(modelos: &[&str]) -> Vec<String> {
        modelos.iter().map(|m| m.to_string()).collect()
    }

    fn falla(estado: StatusCode) -> ErrorApi {
        ErrorApi::nuevo(estado, "openrouter_rechaza", "el proveedor dijo que no")
    }

    #[tokio::test]
    async fn el_primero_que_responde_manda_y_nadie_mas_se_intenta() {
        let intentos = AtomicUsize::new(0);
        let (modelo, salida) = por_la_cadena(&cadena(&["bueno", "respaldo"]), |m| {
            intentos.fetch_add(1, Ordering::Relaxed);
            let m = m.to_string();
            async move { Ok::<_, ErrorApi>(m) }
        })
        .await;

        assert_eq!(modelo, "bueno");
        assert_eq!(salida.unwrap(), "bueno");
        assert_eq!(intentos.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn un_fallo_del_proveedor_pasa_al_respaldo() {
        let (modelo, salida) = por_la_cadena(&cadena(&["caido", "respaldo"]), |m| {
            let m = m.to_string();
            async move {
                if m == "caido" {
                    return Err(falla(StatusCode::SERVICE_UNAVAILABLE));
                }
                Ok(m)
            }
        })
        .await;

        assert_eq!(modelo, "respaldo");
        assert_eq!(salida.unwrap(), "respaldo");
    }

    #[tokio::test]
    async fn una_consulta_mal_formada_no_se_reintenta_en_otro_modelo() {
        let intentos = AtomicUsize::new(0);
        let (modelo, salida) = por_la_cadena(&cadena(&["primero", "respaldo"]), |_| {
            intentos.fetch_add(1, Ordering::Relaxed);
            async { Err::<String, _>(falla(StatusCode::BAD_REQUEST)) }
        })
        .await;

        // Fallaría igual en el respaldo: probarlo solo gastaría tiempo.
        assert_eq!(modelo, "primero");
        assert_eq!(intentos.load(Ordering::Relaxed), 1);
        assert_eq!(salida.unwrap_err().estado, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn si_cae_la_cadena_entera_vuelve_el_fallo_del_ultimo() {
        let (modelo, salida) = por_la_cadena(&cadena(&["uno", "dos"]), |m| {
            let m = m.to_string();
            async move {
                Err::<String, _>(falla(if m == "uno" {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::TOO_MANY_REQUESTS
                }))
            }
        })
        .await;

        assert_eq!(modelo, "dos");
        assert_eq!(salida.unwrap_err().estado, StatusCode::TOO_MANY_REQUESTS);
    }
}
