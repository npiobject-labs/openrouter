use std::{sync::Arc, time::Duration, time::Instant};

use axum::{
    extract::State,
    http::{header::HeaderName, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};

use crate::{
    apps::Identidad,
    error::ErrorApi,
    guardia,
    rutas::{modelos::catalogo_de, uso::CABECERA_USO},
    upstream::{Destino, Medicion, Upstream},
    uso::Registro,
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
/// viene, se rechaza `stream`, que es de la etapa 7, y si el modelo lleva el
/// prefijo `hf:` se le quita y la llamada va al router de Hugging Face en vez
/// de a OpenRouter. Toda llamada que llega a salir queda anotada en el registro
/// de uso, vaya bien o mal, y su id viaja de vuelta en la cabecera `X-Uso-Id`.
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

    if objeto
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "streaming_no_disponible",
            "El streaming llega en la etapa 7; manda la consulta sin \"stream\".",
        ));
    }

    // Sin modelo, el de la casa: así una app puede empezar a llamar sin elegir.
    // El de texto razona: si la app no dice cuánto, poco, para que no se coma
    // el `max_tokens` pensando y devuelva la respuesta vacía o cortada.
    if !objeto.contains_key("model") {
        let modelo = if lleva_adjuntos(objeto.get("messages")) {
            servicio.config.modelo_multimodal.clone()
        } else {
            objeto
                .entry("reasoning")
                .or_insert_with(|| json!({ "effort": "low" }));
            servicio.config.modelo_defecto.clone()
        };
        objeto.insert("model".to_string(), Value::String(modelo));
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

    // A quién se le compra lo decide el prefijo del modelo. El upstream recibe
    // el id como él lo entiende; el registro guarda el que pidió la aplicación.
    let destino = Destino::de(&pedido);
    if destino.modelo != pedido {
        if let Some(objeto) = cuerpo.as_object_mut() {
            objeto.insert("model".to_string(), Value::String(destino.modelo.clone()));
        }
    }

    let mut registro = servicio.uso.abre(pedido.clone());
    registro.app_id = quien.app_id();
    registro.huella = Some(huella);
    registro.upstream = destino.upstream.nombre().to_string();
    // El trabajo de negocio al que pertenece la llamada: varias consultas de un
    // mismo presupuesto se agrupan luego por aqui.
    registro.operacion = cabeceras
        .get("x-operacion")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().chars().take(120).collect::<String>())
        .filter(|v| !v.is_empty());
    let id_uso = registro.id.clone();

    let reloj = Instant::now();
    let resultado = servicio.openrouter.chat(destino.upstream, cuerpo).await;
    registro.latencia_ms = reloj.elapsed().as_millis() as i64;

    match resultado {
        Ok(respuesta) => {
            registro.estado = StatusCode::OK.as_u16();
            registro.desde_respuesta(&respuesta);
            anota_destino(&mut registro, &destino);
            let referencia = referencia_de_precio(&registro, &destino);
            let precio = match servicio.catalogo.precio(&referencia) {
                Some(p) => Some(p),
                // Sin reconciliación, la estimación es el único coste que va a
                // haber: merece traer el catálogo si aún no está. Una vez por
                // hora, lo que dura la caché.
                None if destino.upstream.medicion() == Medicion::Estimada => {
                    let _ = catalogo_de(&servicio, destino.upstream, false).await;
                    servicio.catalogo.precio(&referencia)
                }
                None => None,
            };
            registro.estima(precio);

            let pendiente = match destino.upstream.medicion() {
                Medicion::Reconciliada => registro.id_openrouter.clone(),
                Medicion::Estimada => None,
            };
            servicio.uso.anota(registro);
            if let Some(generacion) = pendiente {
                reconcilia(servicio.clone(), id_uso.clone(), generacion);
            }

            Ok(con_uso(
                &id_uso,
                aviso.as_deref(),
                (StatusCode::OK, Json(respuesta)),
            ))
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

/// Deja el registro de una llamada a otro upstream coherente con el resto del
/// histórico: el modelo servido con su prefijo, para que `agrupar=modelo` no
/// mezcle el `openai/gpt-oss-120b` de Hugging Face con el de OpenRouter; el
/// host elegido como proveedor, porque el router no lo dice en la respuesta;
/// y sin `id_openrouter`, que es de quien lo reconcilia.
///
/// [SUPUESTO] la respuesta del router de Hugging Face no trae `provider` ni
/// un `model` con el sufijo `:<host>`. Plan B si lo trae: `desde_respuesta`
/// ya lo habrá leído y aquí no se pisa nada que venga informado.
fn anota_destino(registro: &mut Registro, destino: &Destino) {
    if destino.upstream == Upstream::OpenRouter {
        return;
    }
    let prefijo = destino.upstream.prefijo();
    if let Some(servido) = &registro.modelo_servido {
        if !servido.starts_with(prefijo) {
            registro.modelo_servido = Some(format!("{prefijo}{servido}"));
        }
    }
    if registro.proveedor.is_none() {
        registro.proveedor = destino.host().map(str::to_string);
    }
    registro.id_openrouter = None;
}

/// Con qué id se busca el precio en el catálogo. En OpenRouter, el modelo
/// servido, que puede no ser el pedido porque enruta. En Hugging Face, el
/// pedido: lleva el host (`hf:...:groq`) que fija el precio, y el servido
/// solo dice el modelo.
fn referencia_de_precio(registro: &Registro, destino: &Destino) -> String {
    match destino.upstream {
        Upstream::OpenRouter => registro
            .modelo_servido
            .clone()
            .unwrap_or_else(|| registro.modelo_pedido.clone()),
        Upstream::HuggingFace => registro.modelo_pedido.clone(),
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

/// Si algún mensaje trae una parte que no es texto (`image_url`, `file`,
/// `input_audio`, `video_url`...). Con contenido en texto plano, no.
fn lleva_adjuntos(mensajes: Option<&Value>) -> bool {
    mensajes
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("content").and_then(Value::as_array))
        .flatten()
        .any(|parte| parte.get("type").and_then(Value::as_str) != Some("text"))
}

#[cfg(test)]
mod pruebas {
    use super::{anota_destino, lleva_adjuntos, referencia_de_precio};
    use crate::{upstream::Destino, uso::Uso};
    use serde_json::json;

    #[test]
    fn una_llamada_hf_se_anota_con_prefijo_host_y_sin_reconciliar() {
        let uso = Uso::nuevo(Some("/no/existe/uso.db"));
        let destino = Destino::de("hf:openai/gpt-oss-120b:groq");
        let mut r = uso.abre("hf:openai/gpt-oss-120b:groq".into());
        r.desde_respuesta(&json!({
            "id": "chatcmpl-1", "model": "openai/gpt-oss-120b",
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        }));
        anota_destino(&mut r, &destino);
        assert_eq!(r.modelo_servido.as_deref(), Some("hf:openai/gpt-oss-120b"));
        assert_eq!(r.proveedor.as_deref(), Some("groq"));
        assert_eq!(r.id_openrouter, None, "no hay /generation que consultar");
        // El precio se busca por lo pedido, que es lo que lleva el host.
        assert_eq!(
            referencia_de_precio(&r, &destino),
            "hf:openai/gpt-oss-120b:groq"
        );

        // Sin host, el proveedor queda sin informar y el precio es el general.
        let destino = Destino::de("hf:openai/gpt-oss-120b:cheapest");
        let mut r = uso.abre("hf:openai/gpt-oss-120b:cheapest".into());
        r.desde_respuesta(&json!({ "model": "openai/gpt-oss-120b" }));
        anota_destino(&mut r, &destino);
        assert_eq!(r.proveedor, None);
    }

    #[test]
    fn en_openrouter_no_se_toca_nada_y_el_precio_es_el_del_servido() {
        let uso = Uso::nuevo(Some("/no/existe/uso.db"));
        let destino = Destino::de("openai/gpt-oss-120b");
        let mut r = uso.abre("openai/gpt-oss-120b".into());
        r.desde_respuesta(&json!({
            "id": "gen-1", "model": "openai/gpt-oss-120b", "provider": "Groq"
        }));
        anota_destino(&mut r, &destino);
        assert_eq!(r.modelo_servido.as_deref(), Some("openai/gpt-oss-120b"));
        assert_eq!(r.proveedor.as_deref(), Some("Groq"));
        assert_eq!(r.id_openrouter.as_deref(), Some("gen-1"));
        assert_eq!(referencia_de_precio(&r, &destino), "openai/gpt-oss-120b");
    }

    #[test]
    fn distingue_texto_de_adjuntos() {
        assert!(!lleva_adjuntos(None));
        assert!(!lleva_adjuntos(Some(
            &json!([{"role": "user", "content": "hola"}])
        )));
        assert!(!lleva_adjuntos(Some(&json!([
            {"role": "user", "content": [{"type": "text", "text": "hola"}]}
        ]))));
        assert!(lleva_adjuntos(Some(&json!([
            {"role": "system", "content": "eres útil"},
            {"role": "user", "content": [
                {"type": "text", "text": "¿qué es?"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}
            ]}
        ]))));
        assert!(lleva_adjuntos(Some(&json!([
            {"role": "user", "content": [{"type": "file", "file": {"filename": "a.pdf"}}]}
        ]))));
    }
}
