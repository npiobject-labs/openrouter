//! Vídeo (etapa 11): generar y editar vídeo con OpenRouter y con Higgsfield.
//!
//! Las dos APIs son asíncronas: se encola un trabajo, se consulta su estado y,
//! al acabar, se descarga el vídeo. Este módulo traduce los dos catálogos a uno
//! solo, estima lo que va a costar un trabajo antes de encolarlo (el
//! presupuesto de la aplicación se comprueba con esa cifra) y guarda cada
//! trabajo en la tabla `videos`, junto al histórico de uso.
//!
//! Un id sin prefijo es de OpenRouter (`google/veo-3.1-fast`); con
//! `higgsfield:` es de Higgsfield (`higgsfield:kling-video/v3.0/std/text-to-video`).

use std::{
    collections::BTreeMap,
    sync::RwLock,
    time::{Duration, Instant},
};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::{json, Value};

/// El catálogo de vídeo cambia menos que el de texto, pero una hora de caché
/// es lo mismo que en `/v1/models`: nadie tiene que acordarse de dos cifras.
const VIGENCIA: Duration = Duration::from_secs(3600);

/// Higgsfield guarda los vídeos «al menos siete días».
pub const DIAS_HIGGSFIELD: u64 = 7;

/// Si no hay forma de estimar un trabajo, se reserva esto del presupuesto: un
/// clip de 8 s de un modelo caro ronda los 3 $, uno medio ~1 $.
pub const ESTIMACION_PRUDENTE: f64 = 1.0;

pub const PREFIJO_HIGGSFIELD: &str = "higgsfield:";

/// A quién se le encarga un vídeo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ProveedorVideo {
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "higgsfield")]
    Higgsfield,
}

impl ProveedorVideo {
    pub fn nombre(self) -> &'static str {
        match self {
            ProveedorVideo::OpenRouter => "openrouter",
            ProveedorVideo::Higgsfield => "higgsfield",
        }
    }

    pub fn desde_nombre(texto: &str) -> Option<Self> {
        match texto.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(ProveedorVideo::OpenRouter),
            "higgsfield" => Some(ProveedorVideo::Higgsfield),
            _ => None,
        }
    }

    /// El proveedor y el id tal como lo entiende él.
    pub fn de(id: &str) -> (Self, &str) {
        match id.strip_prefix(PREFIJO_HIGGSFIELD) {
            Some(resto) => (ProveedorVideo::Higgsfield, resto),
            None => (ProveedorVideo::OpenRouter, id),
        }
    }
}

/// Un modelo de vídeo, igual venga de donde venga.
#[derive(Clone, Debug, Serialize)]
pub struct ModeloVideo {
    pub id: String,
    pub object: &'static str,
    pub upstream: ProveedorVideo,
    pub nombre: String,
    pub descripcion: String,
    /// De una lista cerrada: `texto_a_video`, `imagen_a_video`,
    /// `video_a_video`, `alargar`, `ampliar`, `avatar`.
    pub tareas: Vec<String>,
    pub duraciones: Vec<u64>,
    pub resoluciones: Vec<String>,
    pub formatos: Vec<String>,
    /// Si puede generar audio. `None` cuando el proveedor no lo dice.
    pub audio: Option<bool>,
    /// Los precios tal como los publica el proveedor (OpenRouter los da por
    /// SKU: por segundo, por resolución, con o sin audio, por token de vídeo).
    /// Higgsfield no los publica en el catálogo: su precio sale de su
    /// estimador, trabajo a trabajo.
    pub precios: Option<Value>,
    /// Lo más barato por segundo de vídeo, en dólares, para ordenar y comparar.
    /// `None` si no se puede saber sin estimar el trabajo concreto.
    pub precio_segundo_desde: Option<f64>,
    pub creado: i64,
}

impl ModeloVideo {
    /// Un elemento de `GET /videos/models` de OpenRouter.
    pub fn desde_openrouter(bruto: &Value) -> Option<Self> {
        let id = bruto.get("id")?.as_str()?.to_string();
        let numeros = |campo: &str| -> Vec<u64> {
            bruto
                .get(campo)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default()
        };
        let textos = |campo: &str| -> Vec<String> {
            bruto
                .get(campo)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let fotogramas = textos("supported_frame_images");
        let minusculas = id.to_lowercase();
        let tareas: Vec<&str> = if minusculas.contains("upscale") {
            vec!["ampliar"]
        } else if minusculas.contains("edit") || minusculas.contains("aleph") {
            vec!["video_a_video"]
        } else if minusculas.contains("avatar") {
            vec!["avatar"]
        } else if fotogramas.iter().any(|f| f == "first_frame") {
            vec!["texto_a_video", "imagen_a_video"]
        } else {
            vec!["texto_a_video"]
        };
        let precios = bruto.get("pricing_skus").cloned().filter(Value::is_object);
        Some(Self {
            precio_segundo_desde: precios.as_ref().and_then(precio_segundo_desde),
            nombre: bruto
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string(),
            descripcion: bruto
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tareas: tareas.into_iter().map(str::to_string).collect(),
            duraciones: numeros("supported_durations"),
            resoluciones: textos("supported_resolutions"),
            formatos: textos("supported_aspect_ratios"),
            audio: bruto.get("generate_audio").and_then(Value::as_bool),
            precios,
            creado: bruto.get("created").and_then(Value::as_i64).unwrap_or(0),
            upstream: ProveedorVideo::OpenRouter,
            object: "model",
            id,
        })
    }

    /// Un elemento de `GET /models` de Higgsfield. Solo los de vídeo: los de
    /// imagen no son de esta etapa.
    pub fn desde_higgsfield(bruto: &Value) -> Option<Self> {
        if bruto.get("output_type").and_then(Value::as_str) != Some("video") {
            return None;
        }
        let slug = bruto.get("slug")?.as_str()?;
        let operaciones: Vec<&str> = bruto
            .get("operation_type")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut tareas = Vec::new();
        for op in operaciones {
            let tarea = match op {
                "text2video" => "texto_a_video",
                "image2video" => "imagen_a_video",
                "video2video" => "video_a_video",
                _ => continue,
            };
            if !tareas.contains(&tarea.to_string()) {
                tareas.push(tarea.to_string());
            }
        }
        // El slug dice más que `operation_type`, que Higgsfield rellena con
        // `image2video` también en algunos de texto y de edición.
        if slug.contains("text-to-video") && !tareas.iter().any(|t| t == "texto_a_video") {
            tareas.insert(0, "texto_a_video".into());
        }
        if slug.contains("extend") {
            tareas.push("alargar".into());
        }
        if (slug.contains("video-edit") || slug.contains("restyle") || slug.contains("object-swap"))
            && !tareas.iter().any(|t| t == "video_a_video")
        {
            tareas.push("video_a_video".into());
        }
        let titulo = bruto.get("title").and_then(Value::as_str).unwrap_or(slug);
        Some(Self {
            id: format!("{PREFIJO_HIGGSFIELD}{slug}"),
            object: "model",
            upstream: ProveedorVideo::Higgsfield,
            nombre: format!("{titulo} · {slug}"),
            descripcion: bruto
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tareas,
            duraciones: Vec::new(),
            resoluciones: Vec::new(),
            formatos: Vec::new(),
            audio: None,
            precios: None,
            precio_segundo_desde: None,
            creado: 0,
        })
    }
}

/// Píxeles por segundo de vídeo a 24 fps, para los modelos que cobran por
/// «token de vídeo» (Seedance): un token son 1024 píxeles de un fotograma.
/// [SUPUESTO] 24 fps y 1024 píxeles por token, como documenta BytePlus para
/// Seedance. Plan B: el coste real de `usage.cost` lo corrige al terminar.
fn tokens_por_segundo(resolucion: &str) -> f64 {
    let (ancho, alto) = match resolucion.to_ascii_lowercase().as_str() {
        "360p" => (640.0, 360.0),
        "480p" => (854.0, 480.0),
        "768p" => (1366.0, 768.0),
        "1080p" | "1k" => (1920.0, 1080.0),
        "2k" => (2560.0, 1440.0),
        "4k" => (3840.0, 2160.0),
        _ => (1280.0, 720.0),
    };
    ancho * alto * 24.0 / 1024.0
}

/// El valor de un SKU en dólares: OpenRouter mezcla céntimos (`cents_…`) y
/// dólares (`duration_seconds_…`, `video_tokens…`), y siempre como texto.
fn dolares(clave: &str, valor: &Value) -> Option<f64> {
    let n = valor
        .as_str()
        .and_then(|t| t.parse::<f64>().ok())
        .or_else(|| valor.as_f64())?;
    Some(if clave.contains("cents") {
        n / 100.0
    } else {
        n
    })
}

/// Si un SKU es un precio por segundo de vídeo generado (y no de referencia,
/// continuación, imagen de entrada o mínimo por trabajo).
fn por_segundo(clave: &str) -> bool {
    let k = clave.to_ascii_lowercase();
    (k.contains("second") || k.contains("duration_seconds"))
        && !k.contains("reference")
        && !k.contains("continuation")
        && !k.contains("image_input")
        && !k.contains("minimum")
        && !k.contains("megapixel")
}

/// Lo más barato por segundo que publica un modelo de OpenRouter.
pub fn precio_segundo_desde(skus: &Value) -> Option<f64> {
    let mapa = skus.as_object()?;
    let directos = mapa
        .iter()
        .filter(|(k, _)| por_segundo(k))
        .filter_map(|(k, v)| dolares(k, v));
    let por_tokens = mapa
        .iter()
        .filter(|(k, _)| k.starts_with("video_tokens") && !k.contains("with_video_input"))
        .filter_map(|(k, v)| dolares(k, v).map(|p| p * tokens_por_segundo("480p")));
    directos
        .chain(por_tokens)
        .filter(|p| *p > 0.0)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

/// Lo que costará un trabajo de OpenRouter, en dólares, a partir de sus SKUs y
/// de lo que pide el cuerpo (duración, resolución, audio, imagen de entrada).
/// `None` si no hay forma de saberlo.
pub fn estima_openrouter(modelo: &ModeloVideo, cuerpo: &Value) -> Option<f64> {
    let skus = modelo.precios.as_ref()?.as_object()?;
    let duracion = cuerpo
        .get("duration")
        .and_then(Value::as_u64)
        .or_else(|| modelo.duraciones.iter().max().copied())
        .unwrap_or(8) as f64;
    let resolucion = cuerpo
        .get("resolution")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            // Sin resolución, la más alta que no sea 4K: el proveedor suele
            // usar la suya por defecto y así no se estima por lo bajo.
            modelo
                .resoluciones
                .iter()
                .map(|r| r.to_ascii_lowercase())
                .rfind(|r| r != "4k")
        })
        .unwrap_or_else(|| "720p".into());
    let audio = cuerpo.get("generate_audio").and_then(Value::as_bool);
    let con_imagen = cuerpo
        .get("frame_images")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());

    // La puntuación premia el SKU que encaja con lo pedido; a igualdad, el más
    // caro, para no quedarse corto con el presupuesto.
    let puntua = |k: &str| -> i32 {
        let k = k.to_ascii_lowercase();
        let mut p = 0;
        if k.contains(&resolucion) {
            p += 4;
        } else if ["480p", "720p", "768p", "1080p", "2k", "4k"]
            .iter()
            .any(|r| k.contains(r))
        {
            p -= 4;
        }
        match audio {
            Some(false) if k.contains("without_audio") => p += 2,
            Some(false) if k.contains("with_audio") => p -= 2,
            _ if k.contains("without_audio") => p -= 1,
            _ if k.contains("with_audio") => p += 1,
            _ => {}
        }
        if k.starts_with("image_to_video") {
            p += if con_imagen { 2 } else { -2 };
        }
        if k.starts_with("text_to_video") {
            p += if con_imagen { -2 } else { 2 };
        }
        p
    };
    let mejor = |candidatos: Vec<(&String, f64)>| -> Option<f64> {
        candidatos
            .into_iter()
            .max_by(|(ka, pa), (kb, pb)| {
                puntua(ka)
                    .cmp(&puntua(kb))
                    .then(pa.partial_cmp(pb).unwrap_or(std::cmp::Ordering::Equal))
            })
            .map(|(_, p)| p)
    };

    let segundo: Vec<(&String, f64)> = skus
        .iter()
        .filter(|(k, _)| por_segundo(k))
        .filter_map(|(k, v)| dolares(k, v).map(|p| (k, p)))
        .collect();
    let mut total = if let Some(p) = mejor(segundo) {
        p * duracion
    } else {
        let tokens: Vec<(&String, f64)> = skus
            .iter()
            .filter(|(k, _)| k.starts_with("video_tokens") && !k.contains("with_video_input"))
            .filter_map(|(k, v)| dolares(k, v).map(|p| (k, p)))
            .collect();
        mejor(tokens)? * tokens_por_segundo(&resolucion) * duracion
    };
    if let Some(minimo) = skus
        .get("minimum_cents_per_generation")
        .and_then(|v| dolares("cents", v))
    {
        total = total.max(minimo);
    }
    if con_imagen {
        if let Some(por_imagen) = skus
            .get("cents_per_image_input")
            .and_then(|v| dolares("cents", v))
        {
            total += por_imagen;
        }
    }
    Some((total * 1_000_000.0).round() / 1_000_000.0)
}

/// El estado de un trabajo en nuestras palabras.
pub fn estado_normalizado(proveedor: ProveedorVideo, estado: &str) -> &'static str {
    match (proveedor, estado) {
        (_, "completed") => "completado",
        (_, "failed") | (ProveedorVideo::Higgsfield, "nsfw") => "fallido",
        (_, "cancelled" | "canceled") => "cancelado",
        (ProveedorVideo::OpenRouter, "expired") => "caducado",
        (_, "in_progress") => "en_curso",
        _ => "pendiente",
    }
}

pub fn terminado(estado: &str) -> bool {
    matches!(estado, "completado" | "fallido" | "cancelado" | "caducado")
}

/// Una copia del catálogo y cuándo se trajo.
type Copia = RwLock<Option<(Instant, Vec<ModeloVideo>)>>;

/// Copia del catálogo de vídeo de cada proveedor.
#[derive(Default)]
pub struct CatalogoVideo {
    copias: [Copia; 2],
}

impl CatalogoVideo {
    fn copia(&self, p: ProveedorVideo) -> &Copia {
        &self.copias[match p {
            ProveedorVideo::OpenRouter => 0,
            ProveedorVideo::Higgsfield => 1,
        }]
    }

    pub fn vigente(&self, p: ProveedorVideo) -> Option<Vec<ModeloVideo>> {
        let c = self.copia(p).read().ok()?;
        let (cuando, lista) = c.as_ref()?;
        (cuando.elapsed() < VIGENCIA).then(|| lista.clone())
    }

    pub fn caducada(&self, p: ProveedorVideo) -> Option<Vec<ModeloVideo>> {
        let c = self.copia(p).read().ok()?;
        Some(c.as_ref()?.1.clone())
    }

    /// Traduce la respuesta del proveedor y la guarda. OpenRouter la envuelve
    /// en `data`; Higgsfield, en `items`.
    pub fn guardar(&self, p: ProveedorVideo, bruto: &Value) -> Vec<ModeloVideo> {
        let lista: Vec<ModeloVideo> = match p {
            ProveedorVideo::OpenRouter => bruto
                .get("data")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(ModeloVideo::desde_openrouter).collect())
                .unwrap_or_default(),
            ProveedorVideo::Higgsfield => bruto
                .get("items")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(ModeloVideo::desde_higgsfield).collect())
                .unwrap_or_default(),
        };
        if let Ok(mut c) = self.copia(p).write() {
            *c = Some((Instant::now(), lista.clone()));
        }
        lista
    }
}

pub const ESQUEMA: &str = "
CREATE TABLE IF NOT EXISTS videos (
    id             TEXT PRIMARY KEY,
    uso_id         TEXT NOT NULL,
    upstream       TEXT NOT NULL,
    id_upstream    TEXT NOT NULL,
    modelo         TEXT NOT NULL,
    app_id         TEXT,
    estado         TEXT NOT NULL,
    estado_upstream TEXT,
    coste_estimado REAL,
    coste          REAL,
    urls           TEXT,
    error          TEXT,
    creado         TEXT NOT NULL,
    consultado     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS videos_app ON videos(app_id);
";

/// Un trabajo de vídeo, tal como se guarda y se devuelve.
#[derive(Clone, Debug, Serialize)]
pub struct Trabajo {
    pub id: String,
    pub object: &'static str,
    pub modelo: String,
    pub upstream: String,
    /// `pendiente`, `en_curso`, `completado`, `fallido`, `cancelado` o
    /// `caducado`.
    pub estado: String,
    /// El estado tal cual lo dice el proveedor.
    pub estado_upstream: Option<String>,
    pub coste_estimado: Option<f64>,
    /// El real si el proveedor lo da (OpenRouter); en Higgsfield, el de su
    /// estimador, que es lo que cobra. `0` si falló: ninguno de los dos cobra
    /// un trabajo fallido.
    pub coste: Option<f64>,
    /// Dónde descargar el vídeo. En OpenRouter, nuestra propia ruta
    /// (`/v1/videos/{id}/contenido`), porque la suya pide la clave; en
    /// Higgsfield, su URL pública.
    pub urls: Vec<String>,
    /// Hasta cuándo se podrán descargar, si se sabe.
    pub caduca: Option<String>,
    pub error: Option<String>,
    pub creado: String,
    pub uso_id: String,
    #[serde(skip)]
    pub id_upstream: String,
    #[serde(skip)]
    pub app_id: Option<String>,
    /// Último momento (segundos Unix) en que se preguntó al proveedor.
    #[serde(skip)]
    pub consultado: u64,
}

impl Trabajo {
    fn desde_fila(f: &Row) -> rusqlite::Result<Self> {
        let urls: Option<String> = f.get("urls")?;
        let upstream: String = f.get("upstream")?;
        let creado: String = f.get("creado")?;
        let caduca =
            (upstream == "higgsfield").then(|| crate::uso::mas_dias(&creado, DIAS_HIGGSFIELD));
        Ok(Self {
            id: f.get("id")?,
            object: "video",
            modelo: f.get("modelo")?,
            estado: f.get("estado")?,
            estado_upstream: f.get("estado_upstream")?,
            coste_estimado: f.get("coste_estimado")?,
            coste: f.get("coste")?,
            urls: urls
                .and_then(|u| serde_json::from_str(&u).ok())
                .unwrap_or_default(),
            caduca: caduca.flatten(),
            error: f.get("error")?,
            uso_id: f.get("uso_id")?,
            id_upstream: f.get("id_upstream")?,
            app_id: f.get("app_id")?,
            consultado: f.get::<_, i64>("consultado")? as u64,
            upstream,
            creado,
        })
    }
}

pub fn guarda(c: &Connection, t: &Trabajo) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR REPLACE INTO videos (id, uso_id, upstream, id_upstream, modelo, app_id,
            estado, estado_upstream, coste_estimado, coste, urls, error, creado, consultado)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            t.id,
            t.uso_id,
            t.upstream,
            t.id_upstream,
            t.modelo,
            t.app_id,
            t.estado,
            t.estado_upstream,
            t.coste_estimado,
            t.coste,
            serde_json::to_string(&t.urls).ok(),
            t.error,
            t.creado,
            t.consultado as i64
        ],
    )?;
    Ok(())
}

pub fn busca(c: &Connection, id: &str) -> Option<Trabajo> {
    c.query_row(
        "SELECT * FROM videos WHERE id = ?1",
        [id],
        Trabajo::desde_fila,
    )
    .optional()
    .ok()
    .flatten()
}

/// Lo que dice el proveedor de un trabajo, ya traducido.
pub struct Novedad {
    pub estado_upstream: String,
    pub urls: Vec<String>,
    pub coste: Option<f64>,
    pub error: Option<String>,
}

/// La respuesta de consulta de OpenRouter (`GET /videos/{id}`).
pub fn novedad_openrouter(cuerpo: &Value, id_nuestro: &str) -> Novedad {
    let n = cuerpo
        .get("unsigned_urls")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Novedad {
        estado_upstream: texto(cuerpo, "status").unwrap_or_else(|| "pending".into()),
        // Sus URL piden la clave de OpenRouter: se sirven por nuestra ruta.
        urls: (0..n)
            .map(|i| format!("/v1/videos/{id_nuestro}/contenido?index={i}"))
            .collect(),
        coste: cuerpo
            .get("usage")
            .and_then(|u| u.get("cost"))
            .and_then(Value::as_f64),
        error: texto(cuerpo, "error").or_else(|| {
            cuerpo
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }),
    }
}

/// La respuesta de consulta de Higgsfield (`GET /requests/{id}/status`).
pub fn novedad_higgsfield(cuerpo: &Value) -> Novedad {
    let mut urls: Vec<String> = cuerpo
        .get("video")
        .and_then(|v| v.get("url"))
        .and_then(Value::as_str)
        .map(|u| vec![u.to_string()])
        .unwrap_or_default();
    if urls.is_empty() {
        urls = cuerpo
            .get("images")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|i| i.get("url").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    }
    Novedad {
        estado_upstream: texto(cuerpo, "status").unwrap_or_else(|| "queued".into()),
        urls,
        coste: None,
        error: texto(cuerpo, "error"),
    }
}

fn texto(v: &Value, campo: &str) -> Option<String> {
    v.get(campo)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Filtros de `GET /v1/videos/models`.
#[derive(Default)]
pub struct FiltrosVideo {
    pub tarea: Option<String>,
    pub texto: Option<String>,
    pub max_precio_segundo: Option<f64>,
}

pub fn filtrar(lista: Vec<ModeloVideo>, f: &FiltrosVideo) -> Vec<ModeloVideo> {
    let mut lista: Vec<ModeloVideo> = lista
        .into_iter()
        .filter(|m| {
            f.tarea
                .as_ref()
                .is_none_or(|t| m.tareas.iter().any(|mt| mt.eq_ignore_ascii_case(t)))
        })
        .filter(|m| {
            f.texto.as_ref().is_none_or(|t| {
                let t = t.to_lowercase();
                m.id.to_lowercase().contains(&t) || m.nombre.to_lowercase().contains(&t)
            })
        })
        .filter(|m| {
            f.max_precio_segundo
                .is_none_or(|max| m.precio_segundo_desde.is_some_and(|p| p <= max))
        })
        .collect();
    // Con precio primero, del más barato al más caro; los de Higgsfield, que se
    // estiman trabajo a trabajo, detrás.
    lista.sort_by(|a, b| {
        let clave = |m: &ModeloVideo| m.precio_segundo_desde.unwrap_or(f64::MAX);
        clave(a)
            .partial_cmp(&clave(b))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    lista
}

/// Lo que se manda al proveedor: el cuerpo de la aplicación sin `model` en
/// Higgsfield (el modelo va en la URL) y con el id sin prefijo en OpenRouter.
pub fn cuerpo_para(proveedor: ProveedorVideo, modelo: &str, cuerpo: &Value) -> Value {
    let mut c: BTreeMap<String, Value> = cuerpo
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    match proveedor {
        ProveedorVideo::OpenRouter => {
            c.insert("model".into(), json!(modelo));
        }
        ProveedorVideo::Higgsfield => {
            c.remove("model");
        }
    }
    json!(c)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Recortes reales de los dos catálogos (07/10/2026).
    const OPENROUTER: &str = include_str!("../tests/fixtures/openrouter-videos-models.json");
    const HIGGSFIELD: &str = include_str!("../tests/fixtures/higgsfield-models.json");

    fn catalogo() -> Vec<ModeloVideo> {
        let c = CatalogoVideo::default();
        let mut l = c.guardar(
            ProveedorVideo::OpenRouter,
            &serde_json::from_str(OPENROUTER).unwrap(),
        );
        l.extend(c.guardar(
            ProveedorVideo::Higgsfield,
            &serde_json::from_str(HIGGSFIELD).unwrap(),
        ));
        l
    }

    fn modelo(id: &str) -> ModeloVideo {
        catalogo().into_iter().find(|m| m.id == id).unwrap()
    }

    #[test]
    fn el_prefijo_decide_el_proveedor() {
        assert_eq!(
            ProveedorVideo::de("higgsfield:kling-video/v3.0/std/text-to-video"),
            (
                ProveedorVideo::Higgsfield,
                "kling-video/v3.0/std/text-to-video"
            )
        );
        assert_eq!(
            ProveedorVideo::de("google/veo-3.1-fast"),
            (ProveedorVideo::OpenRouter, "google/veo-3.1-fast")
        );
    }

    #[test]
    fn traduce_los_dos_catalogos() {
        let veo = modelo("google/veo-3.1-fast");
        assert_eq!(veo.tareas, vec!["texto_a_video", "imagen_a_video"]);
        assert_eq!(veo.duraciones, vec![4, 6, 8]);
        // Lo más barato de Veo 3.1 Fast es sin audio a 720p: 0,10 $/s o menos.
        assert!(veo.precio_segundo_desde.unwrap() <= 0.10 + 1e-9);

        let edit = modelo("black-forest-labs/flux-video-edit");
        assert_eq!(edit.tareas, vec!["video_a_video"]);
        assert!(
            (edit.precio_segundo_desde.unwrap() - 0.03).abs() < 1e-9,
            "3 céntimos"
        );

        let up = modelo("black-forest-labs/flux-video-upscale");
        assert_eq!(up.tareas, vec!["ampliar"]);

        let kling = modelo("higgsfield:kling-video/v3.0/std/text-to-video");
        assert_eq!(kling.upstream, ProveedorVideo::Higgsfield);
        assert!(kling.tareas.contains(&"texto_a_video".to_string()));
        assert_eq!(
            kling.precio_segundo_desde, None,
            "Higgsfield se estima por trabajo"
        );
        let extend = modelo("higgsfield:bytedance/seedance-2.5/video-extend");
        assert!(extend.tareas.contains(&"alargar".to_string()));
        // Los de imagen de Higgsfield no entran.
        assert!(catalogo()
            .iter()
            .all(|m| m.upstream == ProveedorVideo::OpenRouter || m.id.starts_with("higgsfield:")));
    }

    #[test]
    fn estima_por_resolucion_audio_y_duracion() {
        let veo = modelo("google/veo-3.1");
        // 0,40 $/s con audio × 8 s.
        let con_audio =
            estima_openrouter(&veo, &json!({"duration": 8, "resolution": "1080p"})).unwrap();
        assert!((con_audio - 3.2).abs() < 1e-6, "{con_audio}");
        // Sin audio, 0,20 $/s × 4 s.
        let sin = estima_openrouter(
            &veo,
            &json!({"duration": 4, "resolution": "1080p", "generate_audio": false}),
        )
        .unwrap();
        assert!((sin - 0.8).abs() < 1e-6, "{sin}");

        // Runway Aleph cobra 28 céntimos/s con un mínimo de 56 por trabajo.
        let aleph = modelo("runway/aleph-2");
        let corto = estima_openrouter(&aleph, &json!({"duration": 1})).unwrap();
        assert!((corto - 0.56).abs() < 1e-6, "{corto}");

        // Seedance cobra por token de vídeo: 720p × 5 s.
        let seed = modelo("bytedance/seedance-2.0-mini");
        let s = estima_openrouter(&seed, &json!({"duration": 5, "resolution": "720p"})).unwrap();
        let esperado = 0.0000035 * 1280.0 * 720.0 * 24.0 / 1024.0 * 5.0;
        assert!((s - esperado).abs() < 1e-4, "{s} frente a {esperado}");
    }

    #[test]
    fn traduce_estados_y_respuestas() {
        assert_eq!(
            estado_normalizado(ProveedorVideo::Higgsfield, "queued"),
            "pendiente"
        );
        assert_eq!(
            estado_normalizado(ProveedorVideo::Higgsfield, "nsfw"),
            "fallido"
        );
        assert_eq!(
            estado_normalizado(ProveedorVideo::OpenRouter, "completed"),
            "completado"
        );
        assert!(terminado("completado") && !terminado("en_curso"));

        let n = novedad_openrouter(
            &json!({"status":"completed","unsigned_urls":["https://openrouter.ai/x?index=0"],"usage":{"cost":0.25}}),
            "v-1",
        );
        assert_eq!(n.urls, vec!["/v1/videos/v-1/contenido?index=0"]);
        assert_eq!(n.coste, Some(0.25));

        let h =
            novedad_higgsfield(&json!({"status":"completed","video":{"url":"https://cdn/x.mp4"}}));
        assert_eq!(h.urls, vec!["https://cdn/x.mp4"]);
        assert_eq!(h.estado_upstream, "completed");
    }

    #[test]
    fn filtra_por_tarea_y_precio() {
        let editar = filtrar(
            catalogo(),
            &FiltrosVideo {
                tarea: Some("video_a_video".into()),
                ..FiltrosVideo::default()
            },
        );
        assert!(editar
            .iter()
            .any(|m| m.id == "black-forest-labs/flux-video-edit"));
        assert!(editar.iter().any(|m| m.id.starts_with("higgsfield:")));
        let baratos = filtrar(
            catalogo(),
            &FiltrosVideo {
                max_precio_segundo: Some(0.05),
                ..FiltrosVideo::default()
            },
        );
        assert!(!baratos.is_empty());
        assert!(baratos
            .iter()
            .all(|m| m.precio_segundo_desde.unwrap() <= 0.05));
    }

    #[test]
    fn el_cuerpo_de_higgsfield_va_sin_model() {
        let c = cuerpo_para(
            ProveedorVideo::Higgsfield,
            "kling-video/v3.0/std/text-to-video",
            &json!({"model": "higgsfield:kling-video/v3.0/std/text-to-video", "prompt": "x"}),
        );
        assert_eq!(c, json!({"prompt": "x"}));
        let c = cuerpo_para(
            ProveedorVideo::OpenRouter,
            "google/veo-3.1",
            &json!({"model": "google/veo-3.1", "prompt": "x", "duration": 4}),
        );
        assert_eq!(c["model"], "google/veo-3.1");
        assert_eq!(c["duration"], 4);
    }
}
