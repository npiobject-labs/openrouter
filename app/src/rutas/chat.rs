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
    catalogo::{nombre_normalizado, Modelo},
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
/// Quién respondió: `openrouter`, `hf` o `rq`.
const CABECERA_UPSTREAM: HeaderName = HeaderName::from_static("x-upstream");
/// En la petición, `no` apaga el respaldo. En la respuesta, el upstream que
/// falló y obligó a usarlo.
const CABECERA_RESPALDO: HeaderName = HeaderName::from_static("x-respaldo");

/// `POST /v1/chat/completions`: proxy fino con el contrato de OpenAI.
///
/// El cuerpo se reenvía tal cual salvo tres retoques: se rellena `model` si no
/// viene, se rechaza `stream`, que es de la etapa 7, y si el modelo lleva el
/// prefijo `hf:` o `rq:` se le quita y la llamada va al router de Hugging Face
/// o a Requesty en vez de a OpenRouter. Si OpenRouter no está, la consulta se
/// repite con el mismo modelo en esos dos (respaldo). Toda llamada que llega a
/// salir queda anotada en el registro de uso, vaya bien o mal, y su id viaja de
/// vuelta en la cabecera `X-Uso-Id`; quién respondió, en `X-Upstream`.
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

    let operacion = cabeceras
        .get("x-operacion")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().chars().take(120).collect::<String>())
        .filter(|v| !v.is_empty());
    let llamada = Llamada {
        servicio: &servicio,
        quien: &quien,
        operacion,
    };

    // A quién se le compra lo decide el prefijo del modelo.
    let destino = Destino::de(&pedido);
    let fallo = match llamada
        .intenta(&pedido, &destino, &cuerpo, Some(huella))
        .await
    {
        Ok(hecha) => return Ok(hecha.respuesta(aviso.as_deref(), None)),
        Err(fallo) => fallo,
    };

    // Respaldo: si OpenRouter no está (caído, saturado, sin crédito), la misma
    // consulta va al mismo modelo en otro upstream. Un 4xx se devuelve tal
    // cual: un esquema que no vale fallaría igual en cualquier sitio.
    if destino.upstream != Upstream::OpenRouter
        || !conmutable(&fallo)
        || !respaldo_permitido(&cabeceras)
    {
        return Err(fallo);
    }
    for alternativa in equivalentes(&servicio, &pedido, pide_esquema(&cuerpo)).await {
        let destino = Destino::de(&alternativa);
        // Sin huella: un respaldo no es una repetición, y no debe disparar el
        // cortacircuitos de bucles de la aplicación.
        if let Ok(hecha) = llamada.intenta(&alternativa, &destino, &cuerpo, None).await {
            return Ok(hecha.respuesta(aviso.as_deref(), Some(Upstream::OpenRouter)));
        }
    }
    Err(fallo)
}

/// Lo que comparten todos los intentos de una misma consulta.
struct Llamada<'a> {
    servicio: &'a Arc<Servicio>,
    quien: &'a Identidad,
    operacion: Option<String>,
}

/// Un intento que salió bien.
struct Hecha {
    cuerpo: Value,
    id_uso: String,
    upstream: Upstream,
}

impl Hecha {
    fn respuesta(self, aviso: Option<&str>, respaldo_de: Option<Upstream>) -> Response {
        let mut cabeceras = HeaderMap::new();
        if let Ok(valor) = HeaderValue::from_str(&self.id_uso) {
            cabeceras.insert(CABECERA_USO, valor);
        }
        // El aviso de presupuesto viaja en cabecera: no cambia el cuerpo, que
        // es del upstream, y una aplicacion puede mirarlo sin parsear nada.
        if let Some(valor) = aviso.and_then(|a| HeaderValue::from_str(a).ok()) {
            cabeceras.insert(CABECERA_AVISO, valor);
        }
        cabeceras.insert(
            CABECERA_UPSTREAM,
            HeaderValue::from_static(self.upstream.nombre()),
        );
        if let Some(caido) = respaldo_de {
            cabeceras.insert(CABECERA_RESPALDO, HeaderValue::from_static(caido.nombre()));
        }
        (cabeceras, (StatusCode::OK, Json(self.cuerpo))).into_response()
    }
}

impl Llamada<'_> {
    /// Un intento contra un upstream, medido y anotado vaya bien o mal.
    async fn intenta(
        &self,
        pedido: &str,
        destino: &Destino,
        cuerpo: &Value,
        huella: Option<String>,
    ) -> Result<Hecha, ErrorApi> {
        let servicio = self.servicio;
        // El upstream recibe el id como él lo entiende; el registro guarda el
        // que pidió la aplicación.
        let mut cuerpo = cuerpo.clone();
        if let Some(objeto) = cuerpo.as_object_mut() {
            objeto.insert("model".to_string(), Value::String(destino.modelo.clone()));
        }

        let mut registro = servicio.uso.abre(pedido.to_string());
        registro.app_id = self.quien.app_id();
        registro.huella = huella;
        registro.upstream = destino.upstream.nombre().to_string();
        // El trabajo de negocio al que pertenece la llamada: varias consultas
        // de un mismo presupuesto se agrupan luego por aqui.
        registro.operacion = self.operacion.clone();
        let id_uso = registro.id.clone();

        let reloj = Instant::now();
        let resultado = servicio.openrouter.chat(destino.upstream, cuerpo).await;
        registro.latencia_ms = reloj.elapsed().as_millis() as i64;

        match resultado {
            Ok(respuesta) => {
                registro.estado = StatusCode::OK.as_u16();
                registro.desde_respuesta(&respuesta);
                anota_destino(&mut registro, destino);
                let referencia = referencia_de_precio(&registro, destino);
                let precio = match servicio.catalogo.precio(&referencia) {
                    Some(p) => Some(p),
                    // Sin reconciliación, la estimación puede ser el único
                    // coste que va a haber: merece traer el catálogo si aún no
                    // está. Una vez por hora, lo que dura la caché.
                    None if destino.upstream.medicion() != Medicion::Reconciliada => {
                        let _ = catalogo_de(servicio, destino.upstream, false).await;
                        servicio.catalogo.precio(&referencia)
                    }
                    None => None,
                };
                registro.estima(precio);

                let pendiente = match destino.upstream.medicion() {
                    Medicion::Reconciliada => registro.id_openrouter.clone(),
                    Medicion::Estimada | Medicion::Directa => None,
                };
                servicio.uso.anota(registro);
                if let Some(generacion) = pendiente {
                    reconcilia(servicio.clone(), id_uso.clone(), generacion);
                }
                Ok(Hecha {
                    cuerpo: respuesta,
                    id_uso,
                    upstream: destino.upstream,
                })
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
}

/// Si un fallo de OpenRouter justifica probar en otro upstream: no
/// contestó, tardó demasiado, falló por dentro (5xx), está saturado (429) o
/// la cuenta no tiene crédito (402).
fn conmutable(fallo: &ErrorApi) -> bool {
    fallo.estado.is_server_error()
        || fallo.estado == StatusCode::TOO_MANY_REQUESTS
        || fallo.estado == StatusCode::PAYMENT_REQUIRED
}

/// `X-Respaldo: no` lo apaga para una consulta: la manda quien prefiere un
/// error a que sus datos salgan hacia otro proveedor.
fn respaldo_permitido(cabeceras: &HeaderMap) -> bool {
    !cabeceras
        .get(CABECERA_RESPALDO)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("no"))
}

/// Si la consulta pide salida con esquema: el respaldo tiene que admitirlo.
fn pide_esquema(cuerpo: &Value) -> bool {
    cuerpo
        .get("response_format")
        .and_then(|f| f.get("type"))
        .and_then(Value::as_str)
        == Some("json_schema")
}

/// El mismo modelo en los otros upstreams, en el orden en que se prueban:
/// primero Hugging Face, por el `hugging_face_id` que publica OpenRouter;
/// después Requesty, por el nombre canónico. Solo los upstreams con clave.
async fn equivalentes(servicio: &Servicio, pedido: &str, con_esquema: bool) -> Vec<String> {
    let mut lista = Vec::new();
    if servicio.openrouter.configurado(Upstream::HuggingFace) {
        let repo = servicio
            .catalogo
            .caducada(Upstream::OpenRouter)
            .and_then(|l| l.into_iter().find(|m| m.id == pedido))
            .and_then(|m| m.hugging_face_id);
        if let Some(repo) = repo {
            if let Ok((hf, _)) = catalogo_de(servicio, Upstream::HuggingFace, false).await {
                lista.extend(elige_hf(&hf, &repo, con_esquema));
            }
        }
    }
    if servicio.openrouter.configurado(Upstream::Requesty) {
        if let Ok((rq, _)) = catalogo_de(servicio, Upstream::Requesty, false).await {
            lista.extend(elige_requesty(&rq, pedido, con_esquema));
        }
    }
    lista
}

/// En Hugging Face, la entrada general del repositorio (`hf:<repo>`), que deja
/// elegir host al router.
fn elige_hf(catalogo: &[Modelo], repo: &str, con_esquema: bool) -> Option<String> {
    let id = format!("{}{repo}", Upstream::HuggingFace.prefijo());
    catalogo
        .iter()
        .find(|m| m.id == id && (!con_esquema || m.json))
        .map(|m| m.id.clone())
}

/// En Requesty, entre las variantes del mismo modelo que ni guardan ni
/// entrenan: primero las que procesan en la UE y, dentro, la más barata.
fn elige_requesty(catalogo: &[Modelo], pedido: &str, con_esquema: bool) -> Option<String> {
    let nombre = nombre_normalizado(pedido);
    catalogo
        .iter()
        .filter(|m| {
            nombre_normalizado(&m.id) == nombre
                || m.canonical_slug.as_deref().map(nombre_normalizado) == Some(nombre.clone())
        })
        .filter(|m| m.retencion != Some(true) && m.entrena != Some(true))
        .filter(|m| !con_esquema || m.json)
        .min_by(|a, b| {
            let fuera = |m: &Modelo| m.region.as_deref() != Some("eu");
            fuera(a).cmp(&fuera(b)).then_with(|| {
                (a.entrada + a.salida)
                    .partial_cmp(&(b.entrada + b.salida))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        })
        .map(|m| m.id.clone())
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
    // `usage.cost` lo da también Requesty: es su coste real, no el de OpenRouter.
    if destino.upstream.medicion() == Medicion::Directa && registro.coste.is_some() {
        registro.coste_origen = destino.upstream.nombre().to_string();
    }
}

/// Con qué id se busca el precio en el catálogo. En OpenRouter, el modelo
/// servido, que puede no ser el pedido porque enruta. En Hugging Face, el
/// pedido: lleva el host (`hf:...:groq`) que fija el precio, y el servido
/// solo dice el modelo. En Requesty, también el pedido: el id lleva la región.
fn referencia_de_precio(registro: &Registro, destino: &Destino) -> String {
    match destino.upstream {
        Upstream::OpenRouter => registro
            .modelo_servido
            .clone()
            .unwrap_or_else(|| registro.modelo_pedido.clone()),
        Upstream::HuggingFace | Upstream::Requesty => registro.modelo_pedido.clone(),
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
    use super::*;
    use crate::{
        catalogo::Catalogo,
        openrouter::{pruebas as falso, Cliente},
        uso::Uso,
    };
    use axum::body::to_bytes;
    use serde_json::json;

    const REQUESTY: &str = include_str!("../../tests/fixtures/requesty-models.json");

    fn catalogo_requesty() -> Vec<Modelo> {
        Catalogo::default().guardar(Upstream::Requesty, &serde_json::from_str(REQUESTY).unwrap())
    }

    #[test]
    fn en_requesty_se_elige_la_variante_ue_mas_barata_que_no_retiene() {
        let rq = catalogo_requesty();
        // gpt-oss-120b: runware es el más barato pero global; scaleway y nebius
        // son UE, y nebius es más barato.
        assert_eq!(
            elige_requesty(&rq, "openai/gpt-oss-120b", false).as_deref(),
            Some("rq:nebius/openai/gpt-oss-120b")
        );
        assert_eq!(
            elige_requesty(&rq, "google/gemini-3.1-flash-lite", true).as_deref(),
            Some("rq:vertex/gemini-3.1-flash-lite@eu")
        );
        assert_eq!(elige_requesty(&rq, "no/existe", false), None);
        // Un modelo que solo está con retención no sirve de respaldo.
        assert_eq!(
            elige_requesty(&rq, "anthropic/claude-fable-5.1", false),
            None
        );
    }

    #[test]
    fn solo_conmuta_cuando_el_upstream_no_esta() {
        let con = |estado: StatusCode| ErrorApi::nuevo(estado, "x", "x");
        assert!(conmutable(&con(StatusCode::BAD_GATEWAY)));
        assert!(conmutable(&con(StatusCode::SERVICE_UNAVAILABLE)));
        assert!(conmutable(&con(StatusCode::TOO_MANY_REQUESTS)));
        assert!(conmutable(&con(StatusCode::PAYMENT_REQUIRED)));
        assert!(!conmutable(&con(StatusCode::BAD_REQUEST)));
        assert!(!conmutable(&con(StatusCode::NOT_FOUND)));

        let mut cabeceras = HeaderMap::new();
        assert!(respaldo_permitido(&cabeceras));
        cabeceras.insert(CABECERA_RESPALDO, HeaderValue::from_static("No"));
        assert!(!respaldo_permitido(&cabeceras));
    }

    fn servicio(base_openrouter: &str, base_rq: &str) -> Arc<Servicio> {
        let mut config = falso::config(base_openrouter, "http://127.0.0.1:1", None);
        config.clave_rq = Some("rq-prueba".into());
        config.base_rq = base_rq.into();
        let servicio = Arc::new(Servicio {
            openrouter: Cliente::nuevo(&config),
            catalogo: Catalogo::default(),
            catalogo_video: crate::video::CatalogoVideo::default(),
            uso: Uso::nuevo(Some("/no/existe/uso.db")),
            config,
        });
        // El catálogo de Requesty ya en caché: el servidor falso solo atiende
        // una petición, que tiene que ser la consulta.
        servicio
            .catalogo
            .guardar(Upstream::Requesty, &serde_json::from_str(REQUESTY).unwrap());
        servicio
    }

    async fn pide(
        servicio: Arc<Servicio>,
        cabeceras: HeaderMap,
        cuerpo: Value,
    ) -> Result<Response, ErrorApi> {
        chat(
            State(servicio),
            Extension(Identidad::administracion()),
            cabeceras,
            Json(cuerpo),
        )
        .await
    }

    #[tokio::test]
    async fn si_openrouter_no_contesta_responde_requesty_con_su_coste() {
        let (base_rq, peticiones) = falso::servidor_falso(
            200,
            r#"{"id":"chatcmpl-rq","model":"nebius/openai/gpt-oss-120b","choices":[{"finish_reason":"stop","message":{"content":"hola"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"cost":0.0000042}}"#,
        );
        // Nadie escucha en el puerto 1: OpenRouter «caído».
        let servicio = servicio("http://127.0.0.1:1", &base_rq);
        let respuesta = pide(
            servicio.clone(),
            HeaderMap::new(),
            json!({ "model": "openai/gpt-oss-120b", "messages": [{"role": "user", "content": "hola"}] }),
        )
        .await
        .unwrap();

        assert_eq!(respuesta.status(), StatusCode::OK);
        assert_eq!(respuesta.headers()["x-upstream"], "rq");
        assert_eq!(respuesta.headers()["x-respaldo"], "openrouter");
        let id_uso = respuesta.headers()["x-uso-id"]
            .to_str()
            .unwrap()
            .to_string();
        let cuerpo = to_bytes(respuesta.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&cuerpo).contains("chatcmpl-rq"));

        // A Requesty le llega su id, sin prefijo, con su clave y sin la de OpenRouter.
        let peticion = peticiones.recv().unwrap();
        assert!(
            peticion.contains(r#""model":"nebius/openai/gpt-oss-120b""#),
            "{peticion}"
        );
        assert!(peticion.contains("authorization: Bearer rq-prueba"));
        assert!(!peticion.contains("sk-or-prueba"));

        // Dos registros: el fallo de OpenRouter y el acierto de Requesty, con
        // el coste que dio Requesty y sin huella (no cuenta como bucle).
        let bueno = servicio.uso.uno(&id_uso).unwrap();
        assert_eq!(bueno.upstream, "rq");
        assert_eq!(bueno.modelo_pedido, "rq:nebius/openai/gpt-oss-120b");
        assert_eq!(bueno.coste_origen, "rq");
        assert_eq!(bueno.coste, Some(0.0000042));
        assert_eq!(bueno.huella, None);
        assert_eq!(servicio.uso.total(), 2);
    }

    #[tokio::test]
    async fn con_x_respaldo_no_el_fallo_de_openrouter_se_devuelve_tal_cual() {
        let servicio = servicio("http://127.0.0.1:1", "http://127.0.0.1:1");
        let mut cabeceras = HeaderMap::new();
        cabeceras.insert(CABECERA_RESPALDO, HeaderValue::from_static("no"));
        let fallo = pide(
            servicio.clone(),
            cabeceras,
            json!({ "model": "openai/gpt-oss-120b", "messages": [] }),
        )
        .await
        .unwrap_err();
        assert_eq!(fallo.codigo, "openrouter_inalcanzable");
        assert_eq!(servicio.uso.total(), 1, "un solo intento");
    }

    #[tokio::test]
    async fn una_llamada_rq_va_directa_a_requesty() {
        let (base_rq, peticiones) = falso::servidor_falso(
            200,
            r#"{"id":"c","model":"vertex/gemini-3.1-flash-lite@eu","usage":{"prompt_tokens":1,"completion_tokens":1,"cost":0.001}}"#,
        );
        let servicio = servicio("http://127.0.0.1:1", &base_rq);
        let respuesta = pide(
            servicio.clone(),
            HeaderMap::new(),
            json!({ "model": "rq:vertex/gemini-3.1-flash-lite@eu", "messages": [] }),
        )
        .await
        .unwrap();
        assert_eq!(respuesta.headers()["x-upstream"], "rq");
        assert!(respuesta.headers().get("x-respaldo").is_none());
        assert!(peticiones
            .recv()
            .unwrap()
            .contains(r#""model":"vertex/gemini-3.1-flash-lite@eu""#));
        assert_eq!(servicio.uso.total(), 1);
    }

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
