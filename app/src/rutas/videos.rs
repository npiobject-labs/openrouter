use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};

use crate::{
    apps::Identidad,
    error::ErrorApi,
    guardia,
    rutas::uso::CABECERA_USO,
    uso::ahora_unix,
    video::{
        self, cuerpo_para, estado_normalizado, estima_openrouter, filtrar, terminado, FiltrosVideo,
        ModeloVideo, ProveedorVideo, Trabajo, ESTIMACION_PRUDENTE,
    },
    Servicio,
};

const CABECERA_AVISO: HeaderName = HeaderName::from_static("x-presupuesto");

/// Cada cuánto, como mucho, se pregunta al proveedor por un trabajo: una app
/// que consulta en bucle no debe multiplicar las llamadas hacia fuera.
const SEGUNDOS_ENTRE_CONSULTAS: u64 = 3;

/// `GET /v1/videos/models`: los modelos de vídeo de OpenRouter y de
/// Higgsfield en un solo formato. `?upstream=openrouter|higgsfield` deja uno
/// solo; `tarea`, `texto` y `max_precio_segundo` filtran. Los de Higgsfield
/// llevan el prefijo `higgsfield:` en el id.
pub async fn modelos(
    State(servicio): State<Arc<Servicio>>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, ErrorApi> {
    let pedido = parametros
        .get("upstream")
        .map(String::as_str)
        .filter(|v| !v.is_empty() && *v != "todos");
    let proveedores: Vec<ProveedorVideo> = match pedido {
        None => vec![ProveedorVideo::OpenRouter, ProveedorVideo::Higgsfield],
        Some(v) => vec![ProveedorVideo::desde_nombre(v).ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::BAD_REQUEST,
                "upstream_desconocido",
                format!("No hay proveedor de vídeo «{v}»: usa openrouter o higgsfield."),
            )
        })?],
    };

    let mut lista = Vec::new();
    let mut por_proveedor = serde_json::Map::new();
    for p in proveedores {
        // Sin clave, ese proveedor no está; si se pidió expresamente, 503.
        if !servicio.openrouter.video_configurado(p) && pedido.is_none() {
            por_proveedor.insert(p.nombre().into(), Value::Null);
            continue;
        }
        let modelos = catalogo_video_de(&servicio, p).await?;
        por_proveedor.insert(p.nombre().into(), json!(modelos.len()));
        lista.extend(modelos);
    }

    let filtros = FiltrosVideo {
        tarea: parametros.get("tarea").filter(|t| !t.is_empty()).cloned(),
        texto: parametros.get("texto").filter(|t| !t.is_empty()).cloned(),
        max_precio_segundo: parametros
            .get("max_precio_segundo")
            .and_then(|v| v.parse().ok()),
    };
    let total = lista.len();
    let data = filtrar(lista, &filtros);
    Ok(Json(json!({
        "object": "list",
        "data": data,
        "total": total,
        "upstreams": por_proveedor,
    })))
}

/// El catálogo de un proveedor: la copia vigente, o se pide y se guarda; si
/// el proveedor no responde, la caducada antes que un error.
pub async fn catalogo_video_de(
    servicio: &Servicio,
    p: ProveedorVideo,
) -> Result<Vec<ModeloVideo>, ErrorApi> {
    if let Some(lista) = servicio.catalogo_video.vigente(p) {
        return Ok(lista);
    }
    match servicio.openrouter.video_modelos(p).await {
        Ok(bruto) => Ok(servicio.catalogo_video.guardar(p, &bruto)),
        Err(e) => servicio.catalogo_video.caducada(p).ok_or(e),
    }
}

/// Un id de modelo que va a acabar en una URL del proveedor: solo letras,
/// números y `._-/`, y nada de `..`.
fn id_valido(id: &str) -> bool {
    !id.is_empty()
        && !id.contains("..")
        && !id.starts_with('/')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// `POST /v1/videos`: encola un vídeo y responde `202` al momento con el
/// trabajo. El cuerpo es el del proveedor (`prompt`, `duration`,
/// `resolution`... en OpenRouter; el del modelo en Higgsfield) más `model`.
///
/// Antes de encolar se estima el coste y se comprueba que cabe en el
/// presupuesto de la aplicación: un vídeo cuesta lo que cien consultas de
/// texto, y mejor un `402` antes que una sorpresa después.
pub async fn crea(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    cabeceras: HeaderMap,
    Json(cuerpo): Json<Value>,
) -> Result<Response, ErrorApi> {
    let pedido = cuerpo
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::BAD_REQUEST,
                "cuerpo_invalido",
                "Falta \"model\": elige uno de /v1/videos/models.",
            )
        })?
        .to_string();
    let (proveedor, modelo) = ProveedorVideo::de(&pedido);
    if !id_valido(modelo) {
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "cuerpo_invalido",
            "El id del modelo solo puede llevar letras, números y . _ - /.",
        ));
    }
    let modelo = modelo.to_string();
    let enviado = cuerpo_para(proveedor, &modelo, &cuerpo);

    let huella = crate::apps::hash(&cuerpo.to_string());
    let aviso = guardia::comprueba(&servicio, &quien, &huella)?;

    // Cuánto va a costar, antes de gastar nada.
    let (estimado, _) = estima(&servicio, proveedor, &modelo, &cuerpo, &enviado).await?;
    if let Some(app) = &quien.app {
        let margen = guardia::margen_de(&servicio, app);
        if let Some(restante) = margen.restante {
            if estimado > restante {
                return Err(ErrorApi::nuevo(
                    StatusCode::PAYMENT_REQUIRED,
                    "presupuesto_insuficiente",
                    format!(
                        "Este vídeo costaría unos {estimado:.2} $ y a la aplicación le quedan \
                         {restante:.2} $ de presupuesto en el periodo."
                    ),
                ));
            }
        }
    }

    let mut registro = servicio.uso.abre(pedido.clone());
    registro.app_id = quien.app_id();
    registro.huella = Some(huella);
    registro.upstream = proveedor.nombre().to_string();
    registro.operacion = cabeceras
        .get("x-operacion")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().chars().take(120).collect::<String>())
        .filter(|v| !v.is_empty());
    // Mientras el vídeo se hace, cuenta en el presupuesto lo estimado.
    registro.coste = Some(estimado);
    registro.coste_origen = "estimado".into();
    let id_uso = registro.id.clone();

    let reloj = std::time::Instant::now();
    let resultado = servicio
        .openrouter
        .video_crea(proveedor, &modelo, &enviado)
        .await;
    registro.latencia_ms = reloj.elapsed().as_millis() as i64;

    let respuesta = match resultado {
        Ok(r) => r,
        Err(fallo) => {
            // No se encoló nada: no cuesta.
            registro.estado = fallo.estado.as_u16();
            registro.motivo_fin = Some(fallo.codigo.to_string());
            registro.coste = Some(0.0);
            servicio.uso.anota(registro);
            return Err(fallo.con_uso(&id_uso));
        }
    };
    let id_upstream = respuesta
        .get(match proveedor {
            ProveedorVideo::OpenRouter => "id",
            ProveedorVideo::Higgsfield => "request_id",
        })
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(id_upstream) = id_upstream else {
        registro.estado = StatusCode::BAD_GATEWAY.as_u16();
        registro.motivo_fin = Some("respuesta_ilegible".into());
        servicio.uso.anota(registro);
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_GATEWAY,
            "respuesta_ilegible",
            "El proveedor aceptó el vídeo pero no devolvió el id del trabajo.",
        )
        .con_upstream(respuesta)
        .con_uso(&id_uso));
    };

    let estado_upstream = respuesta
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending")
        .to_string();
    registro.estado = StatusCode::ACCEPTED.as_u16();
    registro.motivo_fin = Some("video_pendiente".into());
    registro.id_openrouter = None;
    let fecha = registro.fecha.clone();
    servicio.uso.anota(registro);

    let trabajo = Trabajo {
        id: format!("vid{}", id_uso.trim_start_matches('u')),
        object: "video",
        modelo: pedido,
        upstream: proveedor.nombre().to_string(),
        estado: estado_normalizado(proveedor, &estado_upstream).to_string(),
        estado_upstream: Some(estado_upstream),
        coste_estimado: Some(estimado),
        coste: None,
        urls: Vec::new(),
        caduca: None,
        error: None,
        creado: fecha,
        uso_id: id_uso.clone(),
        id_upstream,
        app_id: quien.app_id(),
        consultado: ahora_unix(),
    };
    servicio
        .uso
        .con(|c| video::guarda(c, &trabajo))
        .map_err(|e| {
            ErrorApi::nuevo(
                StatusCode::INTERNAL_SERVER_ERROR,
                "almacen",
                format!("El vídeo se encoló pero no se pudo guardar el trabajo: {e}"),
            )
        })?;

    let mut salida = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&id_uso) {
        salida.insert(CABECERA_USO, v);
    }
    if let Some(v) = aviso.as_deref().and_then(|a| HeaderValue::from_str(a).ok()) {
        salida.insert(CABECERA_AVISO, v);
    }
    Ok((StatusCode::ACCEPTED, salida, Json(trabajo)).into_response())
}

/// Lo que costaría un vídeo y de dónde sale la cifra: `precios` (los SKU de
/// OpenRouter), `higgsfield` (su estimador) o `prudente` (sin forma de saberlo).
async fn estima(
    servicio: &Servicio,
    proveedor: ProveedorVideo,
    modelo: &str,
    cuerpo: &Value,
    enviado: &Value,
) -> Result<(f64, &'static str), ErrorApi> {
    match proveedor {
        ProveedorVideo::OpenRouter => Ok(catalogo_video_de(servicio, proveedor)
            .await
            .ok()
            .and_then(|l| l.into_iter().find(|m| m.id == modelo))
            .and_then(|m| estima_openrouter(&m, cuerpo))
            .map(|e| (e, "precios"))
            .unwrap_or((ESTIMACION_PRUDENTE, "prudente"))),
        ProveedorVideo::Higgsfield => servicio
            .openrouter
            .video_estima_higgsfield(modelo, enviado)
            .await
            .map(|e| (e, "higgsfield")),
    }
}

/// `POST /v1/videos/estimar`: lo que costaría el mismo cuerpo de
/// `POST /v1/videos`, sin encolar nada ni gastar. Con clave de aplicación dice
/// además si cabe en lo que le queda de presupuesto.
pub async fn estimar(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Json(cuerpo): Json<Value>,
) -> Result<Json<Value>, ErrorApi> {
    let pedido = cuerpo
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::BAD_REQUEST,
                "cuerpo_invalido",
                "Falta \"model\": elige uno de /v1/videos/models.",
            )
        })?
        .to_string();
    let (proveedor, modelo) = ProveedorVideo::de(&pedido);
    if !id_valido(modelo) {
        return Err(ErrorApi::nuevo(
            StatusCode::BAD_REQUEST,
            "cuerpo_invalido",
            "El id del modelo solo puede llevar letras, números y . _ - /.",
        ));
    }
    let enviado = cuerpo_para(proveedor, modelo, &cuerpo);
    let (coste, origen) = estima(&servicio, proveedor, modelo, &cuerpo, &enviado).await?;
    let restante = quien
        .app
        .as_ref()
        .and_then(|app| guardia::margen_de(&servicio, app).restante);
    Ok(Json(json!({
        "modelo": pedido,
        "upstream": proveedor,
        "coste_estimado": coste,
        "origen": origen,
        "restante": restante,
        "cabe": restante.is_none_or(|r| coste <= r),
    })))
}

/// El trabajo de quien pregunta. Uno de otra aplicación responde como si no
/// existiera, igual que en `/v1/uso/{id}`.
fn trabajo_de(servicio: &Servicio, quien: &Identidad, id: &str) -> Result<Trabajo, ErrorApi> {
    servicio
        .uso
        .con(|c| video::busca(c, id))
        .filter(|t| quien.admin || t.app_id == quien.app_id())
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::NOT_FOUND,
                "video_desconocido",
                format!("No hay ningún vídeo «{id}»."),
            )
        })
}

/// `GET /v1/videos/{id}`: el estado del trabajo. Si no ha terminado, se
/// pregunta al proveedor (como mucho cada pocos segundos) y, al terminar, se
/// cierra el coste: el real de OpenRouter, el estimado de Higgsfield (que es
/// lo que cobra) o cero si falló.
pub async fn uno(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(id): Path<String>,
) -> Result<Json<Trabajo>, ErrorApi> {
    let mut t = trabajo_de(&servicio, &quien, &id)?;
    let ahora = ahora_unix();
    if terminado(&t.estado) || ahora < t.consultado + SEGUNDOS_ENTRE_CONSULTAS {
        return Ok(Json(t));
    }
    let proveedor = ProveedorVideo::desde_nombre(&t.upstream).unwrap_or(ProveedorVideo::OpenRouter);
    let Ok(cuerpo) = servicio
        .openrouter
        .video_estado(proveedor, &t.id_upstream)
        .await
    else {
        // Si el proveedor no contesta, se enseña lo último que se sabía.
        return Ok(Json(t));
    };
    let novedad = match proveedor {
        ProveedorVideo::OpenRouter => video::novedad_openrouter(&cuerpo, &t.id),
        ProveedorVideo::Higgsfield => video::novedad_higgsfield(&cuerpo),
    };
    t.estado = estado_normalizado(proveedor, &novedad.estado_upstream).to_string();
    t.estado_upstream = Some(novedad.estado_upstream);
    if !novedad.urls.is_empty() {
        t.urls = novedad.urls;
    }
    t.error = novedad.error.or(t.error);
    t.consultado = ahora;
    if terminado(&t.estado) {
        let (coste, origen) = match (t.estado.as_str(), proveedor, novedad.coste) {
            ("completado", ProveedorVideo::OpenRouter, Some(c)) => (c, "openrouter"),
            ("completado", ProveedorVideo::Higgsfield, _) => {
                (t.coste_estimado.unwrap_or(0.0), "higgsfield")
            }
            ("completado", _, None) => (t.coste_estimado.unwrap_or(0.0), "estimado"),
            // Ni OpenRouter ni Higgsfield cobran un trabajo que falla.
            (_, p, c) => (c.unwrap_or(0.0), p.nombre()),
        };
        t.coste = Some(coste);
        let estado_http = if t.estado == "completado" { 200 } else { 502 };
        servicio.uso.cierra_video(
            &t.uso_id,
            coste,
            origen,
            estado_http,
            &format!("video_{}", t.estado),
        );
    }
    let _ = servicio.uso.con(|c| video::guarda(c, &t));
    if proveedor == ProveedorVideo::Higgsfield {
        t.caduca = crate::uso::mas_dias(&t.creado, video::DIAS_HIGGSFIELD);
    }
    Ok(Json(t))
}

/// `GET /v1/videos/{id}/contenido?index=0`: el vídeo terminado. Los de
/// OpenRouter se sirven a través del servicio, porque su URL pide la clave de
/// OpenRouter; los de Higgsfield redirigen a su URL pública.
pub async fn contenido(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(id): Path<String>,
    Query(parametros): Query<HashMap<String, String>>,
) -> Result<Response, ErrorApi> {
    let t = trabajo_de(&servicio, &quien, &id)?;
    if t.estado != "completado" {
        return Err(ErrorApi::nuevo(
            StatusCode::CONFLICT,
            "video_no_listo",
            format!(
                "El vídeo está «{}»: consulta /v1/videos/{id} hasta que esté completado.",
                t.estado
            ),
        ));
    }
    let indice: u32 = parametros
        .get("index")
        .and_then(|i| i.parse().ok())
        .unwrap_or(0);
    if t.upstream == "higgsfield" {
        let url = t.urls.get(indice as usize).ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::NOT_FOUND,
                "video_desconocido",
                "No hay vídeo con ese índice.",
            )
        })?;
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                HeaderValue::from_str(url).map_err(|_| {
                    ErrorApi::nuevo(
                        StatusCode::BAD_GATEWAY,
                        "respuesta_ilegible",
                        "URL de vídeo no válida.",
                    )
                })?,
            )],
        )
            .into_response());
    }
    let respuesta = servicio
        .openrouter
        .video_contenido(&t.id_upstream, indice)
        .await?;
    let tipo = respuesta
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("video/mp4"));
    let largo = respuesta.headers().get(header::CONTENT_LENGTH).cloned();
    let mut salida = Response::new(Body::from_stream(respuesta.bytes_stream()));
    salida.headers_mut().insert(header::CONTENT_TYPE, tipo);
    if let Some(l) = largo {
        salida.headers_mut().insert(header::CONTENT_LENGTH, l);
    }
    Ok(salida)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::{
        apps::{App, Limites},
        catalogo::Catalogo,
        openrouter::{pruebas as falso, Cliente},
        uso::Uso,
        video::CatalogoVideo,
    };
    use axum::body::to_bytes;

    const VIDEOS: &str = include_str!("../../tests/fixtures/openrouter-videos-models.json");

    fn servicio(base_openrouter: &str) -> Arc<Servicio> {
        let config = falso::config(base_openrouter, "http://127.0.0.1:1", None);
        let servicio = Arc::new(Servicio {
            openrouter: Cliente::nuevo(&config),
            catalogo: Catalogo::default(),
            catalogo_video: CatalogoVideo::default(),
            uso: Uso::nuevo(Some("/no/existe/uso.db")),
            config,
        });
        // Catálogo en caché: el servidor falso solo atiende la creación.
        servicio.catalogo_video.guardar(
            ProveedorVideo::OpenRouter,
            &serde_json::from_str(VIDEOS).unwrap(),
        );
        servicio
    }

    fn app_con_presupuesto(limite: f64) -> Identidad {
        Identidad {
            app: Some(App {
                id: "app_prueba".into(),
                nombre: "prueba".into(),
                activa: true,
                creada: "2026-10-07T00:00:00Z".into(),
                limites: Limites {
                    periodo: Some("mes".into()),
                    limite: Some(limite),
                    aviso: None,
                    cuota_minuto: None,
                },
            }),
            admin: false,
        }
    }

    #[tokio::test]
    async fn encola_en_openrouter_y_anota_lo_estimado() {
        let (base, peticiones) = falso::servidor_falso(
            202,
            r#"{"id":"gen-vid-1789493115-a1B2c3D4e5F6g7H8i9J0","polling_url":"https://openrouter.ai/api/v1/videos/x","status":"pending"}"#,
        );
        let servicio = servicio(&base);
        let respuesta = crea(
            State(servicio.clone()),
            Extension(app_con_presupuesto(5.0)),
            HeaderMap::new(),
            Json(json!({"model": "google/veo-3.1-fast", "prompt": "un perro", "duration": 4, "resolution": "720p", "generate_audio": false})),
        )
        .await
        .unwrap();
        assert_eq!(respuesta.status(), StatusCode::ACCEPTED);
        let id_uso = respuesta.headers()["x-uso-id"]
            .to_str()
            .unwrap()
            .to_string();
        let cuerpo: Value =
            serde_json::from_slice(&to_bytes(respuesta.into_body(), 1 << 20).await.unwrap())
                .unwrap();
        assert_eq!(cuerpo["estado"], "pendiente");
        assert_eq!(cuerpo["upstream"], "openrouter");
        // Veo 3.1 Fast sin audio a 720p: 0,10 $/s × 4 s = 0,40 $ o menos.
        let estimado = cuerpo["coste_estimado"].as_f64().unwrap();
        assert!(estimado > 0.0 && estimado <= 0.4 + 1e-9, "{estimado}");
        assert!(cuerpo["id"].as_str().unwrap().starts_with("vid-"));

        let peticion = peticiones.recv().unwrap();
        assert!(peticion.starts_with("POST /videos HTTP/1.1"), "{peticion}");
        assert!(peticion.contains(r#""model":"google/veo-3.1-fast""#));

        // En el histórico cuenta ya lo estimado, con la aplicación.
        let r = servicio.uso.uno(&id_uso).unwrap();
        assert_eq!(r.coste, Some(estimado));
        assert_eq!(r.app_id.as_deref(), Some("app_prueba"));
        assert_eq!(r.estado, 202);

        // Y el trabajo se puede leer, pero no desde otra aplicación.
        let id = cuerpo["id"].as_str().unwrap().to_string();
        assert!(trabajo_de(&servicio, &app_con_presupuesto(5.0), &id).is_ok());
        let mut otra = app_con_presupuesto(5.0);
        otra.app.as_mut().unwrap().id = "otra".into();
        assert_eq!(
            trabajo_de(&servicio, &otra, &id).unwrap_err().codigo,
            "video_desconocido"
        );
    }

    #[tokio::test]
    async fn un_video_que_no_cabe_en_el_presupuesto_se_corta_antes_de_encolar() {
        // Nadie escucha: si se intentara encolar, fallaría de otra manera.
        let servicio = servicio("http://127.0.0.1:1");
        let fallo = crea(
            State(servicio.clone()),
            Extension(app_con_presupuesto(1.0)),
            HeaderMap::new(),
            Json(json!({"model": "google/veo-3.1", "prompt": "x", "duration": 8, "resolution": "1080p"})),
        )
        .await
        .unwrap_err();
        assert_eq!(fallo.estado, StatusCode::PAYMENT_REQUIRED);
        assert_eq!(fallo.codigo, "presupuesto_insuficiente");
        assert_eq!(servicio.uso.total(), 0, "no salió nada hacia fuera");
    }

    #[test]
    fn el_id_no_puede_salirse_de_la_ruta() {
        assert!(id_valido("kling-video/v3.0/std/text-to-video"));
        assert!(id_valido("google/veo-3.1"));
        assert!(!id_valido("../requests/x/cancel"));
        assert!(!id_valido("/estimate/x"));
        assert!(!id_valido("a b"));
        assert!(!id_valido("x?y=1"));
        assert!(!id_valido(""));
    }
}
