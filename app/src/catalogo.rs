use std::{
    sync::RwLock,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;

use crate::upstream::Upstream;

/// El catálogo de OpenRouter cambia a diario, no a cada minuto: una hora de
/// caché ahorra una llamada por consulta sin servir precios viejos.
const VIGENCIA: Duration = Duration::from_secs(3600);

/// Precios de un modelo más allá de los tokens de entrada y salida. Todos en
/// dólares; los de tokens por millón, `imagen` por imagen, `peticion` por
/// llamada y `busqueda_web` por búsqueda. Cero cuando el modelo no lo cobra o
/// OpenRouter no lo informa.
#[derive(Clone, Default, Serialize)]
pub struct Precios {
    pub imagen: f64,
    pub peticion: f64,
    pub cache_lectura: f64,
    pub cache_escritura: f64,
    pub audio: f64,
    pub razonamiento: f64,
    pub busqueda_web: f64,
    /// Dólares por millón de tokens de imagen generada. Es el precio que
    /// define a los modelos que dibujan, que no cobran por `completion`.
    pub imagen_salida: f64,
}

/// Un modelo, con el formato de la API de OpenAI (`id`, `object`, `created`,
/// `owned_by`) más lo que hace falta para elegir: precio, contexto y qué sabe
/// hacer. Desde 0.6.2 lleva además lo que una aplicación necesita para
/// comparar modelos sin ir a OpenRouter: la descripción del proveedor, los
/// precios completos, las modalidades de salida y la lista de parámetros.
/// Desde la etapa 9 dice de qué upstream viene, conserva `canonical_slug` y
/// `hugging_face_id` de OpenRouter, y trae la latencia y el rendimiento que
/// publica el router de Hugging Face por host.
#[derive(Clone, Serialize)]
pub struct Modelo {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: String,
    pub nombre: String,
    pub contexto: u64,
    /// Dólares por millón de tokens.
    pub entrada: f64,
    pub salida: f64,
    pub gratis: bool,
    pub herramientas: bool,
    pub json: bool,
    pub modalidades: Vec<String>,
    /// Lo que escribe el proveedor sobre el modelo, tal cual lo da OpenRouter.
    pub descripcion: String,
    pub precios: Precios,
    /// Modalidades de salida (`text`, `image`, `audio`...).
    pub modalidades_salida: Vec<String>,
    /// Todos los parámetros que admite, para que el cliente decida sin
    /// depender de los dos booleanos de arriba.
    pub parametros: Vec<String>,
    /// Máximo de tokens de salida del proveedor principal, si lo informa.
    pub max_salida: Option<u64>,
    /// Si el proveedor principal modera las peticiones.
    pub moderado: bool,
    /// De qué upstream es el id: `openrouter` o `hf`. El id ya lleva el
    /// prefijo que hace falta para pedirlo en `model`.
    pub upstream: Upstream,
    /// El identificador estable de OpenRouter, con la fecha de la versión
    /// cuando el id corto apunta a la más reciente. Solo OpenRouter.
    pub canonical_slug: Option<String>,
    /// El repositorio de Hugging Face del modelo, si OpenRouter lo conoce. Es
    /// la llave para encontrar el mismo modelo en el otro upstream.
    pub hugging_face_id: Option<String>,
    /// Milisegundos hasta el primer token, medidos por el router de Hugging
    /// Face en ese host. Solo Hugging Face.
    pub primer_token_ms: Option<f64>,
    /// Tokens por segundo en ese host, idem.
    pub tokens_por_segundo: Option<f64>,
}

impl Modelo {
    /// Traduce un elemento del catálogo de OpenRouter. Devuelve `None` si le
    /// falta el id, que es lo único sin lo cual el modelo no sirve de nada.
    fn desde_openrouter(bruto: &Value) -> Option<Self> {
        let id = bruto.get("id")?.as_str()?.to_string();

        // OpenRouter da el precio por token y como texto ("0.0000001").
        let precio = |campo: &str| -> f64 {
            bruto
                .get("pricing")
                .and_then(|p| p.get(campo))
                .and_then(Value::as_str)
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(0.0)
                * 1_000_000.0
        };
        let entrada = precio("prompt");
        let salida = precio("completion");
        // Los que no van por token se dejan en su unidad: por imagen, por
        // petición y por búsqueda.
        let unitario = |campo: &str| -> f64 { precio(campo) / 1_000_000.0 };
        let precios = Precios {
            imagen: unitario("image"),
            peticion: unitario("request"),
            cache_lectura: precio("input_cache_read"),
            cache_escritura: precio("input_cache_write"),
            audio: precio("audio"),
            razonamiento: precio("internal_reasoning"),
            busqueda_web: unitario("web_search"),
            imagen_salida: precio("image_output"),
        };
        // Gratis es que no cobre nada, no solo los tokens: un modelo de imagen
        // tiene `prompt` y `completion` a cero y cobra por `image_output`.
        // Los de vídeo no publican ningún precio en el catálogo, así que salen
        // gratis aunque no lo sean: OpenRouter los factura por otra vía.
        let gratis = bruto
            .get("pricing")
            .and_then(Value::as_object)
            .map(|p| {
                p.values()
                    .filter_map(Value::as_str)
                    .all(|v| v.parse::<f64>().map(|n| n == 0.0).unwrap_or(true))
            })
            .unwrap_or(true);

        let parametros: Vec<&str> = bruto
            .get("supported_parameters")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        let arquitectura = bruto.get("architecture");
        let modalidades = lista(arquitectura.and_then(|a| a.get("input_modalities")));
        let modalidades_salida = lista(arquitectura.and_then(|a| a.get("output_modalities")));
        let proveedor_principal = bruto.get("top_provider");

        Some(Self {
            owned_by: id.split('/').next().unwrap_or("desconocido").to_string(),
            nombre: bruto
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string(),
            created: bruto.get("created").and_then(Value::as_i64).unwrap_or(0),
            contexto: bruto
                .get("context_length")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            gratis,
            herramientas: parametros.contains(&"tools"),
            json: parametros.contains(&"structured_outputs")
                || parametros.contains(&"response_format"),
            modalidades,
            descripcion: bruto
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            precios,
            modalidades_salida,
            parametros: parametros.iter().map(|p| p.to_string()).collect(),
            max_salida: proveedor_principal
                .and_then(|p| p.get("max_completion_tokens"))
                .and_then(Value::as_u64),
            moderado: proveedor_principal
                .and_then(|p| p.get("is_moderated"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            upstream: Upstream::OpenRouter,
            canonical_slug: texto(bruto.get("canonical_slug")),
            hugging_face_id: texto(bruto.get("hugging_face_id")),
            primer_token_ms: None,
            tokens_por_segundo: None,
            object: "model",
            id,
            entrada,
            salida,
        })
    }

    /// Traduce un elemento del catálogo del router de Hugging Face, que trae
    /// en `providers[]` cada host que sirve el modelo con su precio, contexto,
    /// latencia y rendimiento. Salen varias entradas: `hf:<id>`, con el precio
    /// del host más barato (que es con el que se estima una llamada sin
    /// sufijo), y una `hf:<id>:<host>` por cada host con precio.
    ///
    /// [SUPUESTO] un host sin `pricing` (Featherless, por ejemplo) no se puede
    /// comparar ni estimar, así que no sale en el catálogo aunque se le pueda
    /// llamar. Plan B si hace falta verlo: sacarlo con precio cero y
    /// `gratis: false`, que hoy no se puede expresar.
    fn desde_hf(bruto: &Value) -> Vec<Self> {
        let Some(id) = bruto.get("id").and_then(Value::as_str) else {
            return Vec::new();
        };
        let arquitectura = bruto.get("architecture");
        let modalidades = lista(arquitectura.and_then(|a| a.get("input_modalities")));
        let modalidades_salida = lista(arquitectura.and_then(|a| a.get("output_modalities")));
        let created = bruto.get("created").and_then(Value::as_i64).unwrap_or(0);
        let owned_by = bruto
            .get("owned_by")
            .and_then(Value::as_str)
            .or_else(|| id.split('/').next())
            .unwrap_or("desconocido")
            .to_string();

        let base = |id_completo: String, nombre: String| Self {
            id: id_completo,
            object: "model",
            created,
            owned_by: owned_by.clone(),
            nombre,
            contexto: 0,
            entrada: 0.0,
            salida: 0.0,
            gratis: false,
            herramientas: false,
            json: false,
            modalidades: modalidades.clone(),
            descripcion: String::new(),
            precios: Precios::default(),
            modalidades_salida: modalidades_salida.clone(),
            parametros: Vec::new(),
            max_salida: None,
            moderado: false,
            upstream: Upstream::HuggingFace,
            canonical_slug: None,
            hugging_face_id: Some(id.to_string()),
            primer_token_ms: None,
            tokens_por_segundo: None,
        };

        let prefijo = Upstream::HuggingFace.prefijo();
        let mut hosts: Vec<Self> = bruto
            .get("providers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let host = p.get("provider")?.as_str()?;
                let precios = p.get("pricing")?;
                let entrada = precios.get("input")?.as_f64()?;
                let salida = precios.get("output")?.as_f64()?;
                let mut m = base(format!("{prefijo}{id}:{host}"), format!("{id} · {host}"));
                m.entrada = entrada;
                m.salida = salida;
                m.gratis = p
                    .get("is_free")
                    .and_then(Value::as_bool)
                    .unwrap_or(entrada == 0.0 && salida == 0.0);
                m.contexto = p.get("context_length").and_then(Value::as_u64).unwrap_or(0);
                m.herramientas = p
                    .get("supports_tools")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                m.json = p
                    .get("supports_structured_output")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                m.primer_token_ms = p.get("first_token_latency_ms").and_then(Value::as_f64);
                m.tokens_por_segundo = p.get("throughput").and_then(Value::as_f64);
                Some(m)
            })
            .collect();

        if hosts.is_empty() {
            return hosts;
        }

        // La entrada sin host: lo que cobra el más barato, el contexto del
        // que más tiene, y herramientas o JSON si algún host las da.
        let mut general = base(format!("{prefijo}{id}"), id.to_string());
        let barato = hosts
            .iter()
            .min_by(|a, b| {
                (a.entrada + a.salida)
                    .partial_cmp(&(b.entrada + b.salida))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("hay al menos un host");
        general.entrada = barato.entrada;
        general.salida = barato.salida;
        general.gratis = hosts.iter().all(|h| h.gratis);
        general.contexto = hosts.iter().map(|h| h.contexto).max().unwrap_or(0);
        general.herramientas = hosts.iter().any(|h| h.herramientas);
        general.json = hosts.iter().any(|h| h.json);

        hosts.insert(0, general);
        hosts
    }
}

fn lista(valor: Option<&Value>) -> Vec<String> {
    valor
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn texto(valor: Option<&Value>) -> Option<String> {
    valor
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Catalogo {
    /// Precio de un modelo en dólares por millón, de la copia en memoria del
    /// upstream al que apunta su id. Se usa para estimar el coste de una
    /// llamada; devuelve `None` si ese catálogo aún no se ha traído o el
    /// modelo no está. Para `hf:<id>` sin host es el del host más barato.
    pub fn precio(&self, id: &str) -> Option<(f64, f64)> {
        let upstream = crate::upstream::Destino::de(id).upstream;
        let lista = self.caducada(upstream)?;
        lista
            .iter()
            .find(|m| m.id == id)
            .map(|m| (m.entrada, m.salida))
    }
}

/// Qué filtros admite `GET /v1/models`.
#[derive(Default)]
pub struct Filtros {
    pub texto: Option<String>,
    pub proveedor: Option<String>,
    pub contexto_min: Option<u64>,
    pub gratis: bool,
    /// Una modalidad de salida (`text`, `image`, `video`, `speech`,
    /// `transcription`, `embeddings`): solo los modelos que la producen.
    pub salida: Option<String>,
}

impl Filtros {
    fn pasa(&self, m: &Modelo) -> bool {
        if self.gratis && !m.gratis {
            return false;
        }
        if let Some(min) = self.contexto_min {
            if m.contexto < min {
                return false;
            }
        }
        if let Some(p) = &self.proveedor {
            if !m.owned_by.eq_ignore_ascii_case(p) {
                return false;
            }
        }
        if let Some(s) = &self.salida {
            if !m
                .modalidades_salida
                .iter()
                .any(|ms| ms.eq_ignore_ascii_case(s))
            {
                return false;
            }
        }
        if let Some(t) = &self.texto {
            let t = t.to_lowercase();
            if !m.id.to_lowercase().contains(&t) && !m.nombre.to_lowercase().contains(&t) {
                return false;
            }
        }
        true
    }
}

/// De dónde salió la lista que se devuelve. Viaja al cliente en `X-Cache`.
#[derive(Clone, Copy)]
pub enum Origen {
    /// Recién pedido al upstream.
    Fresco,
    /// Copia vigente.
    Cache,
    /// Copia caducada, porque el upstream no respondió. Mejor precios de ayer
    /// que ningún catálogo.
    Caducada,
}

impl Origen {
    pub fn etiqueta(self) -> &'static str {
        match self {
            Origen::Fresco => "miss",
            Origen::Cache => "hit",
            Origen::Caducada => "stale",
        }
    }
}

struct Copia {
    modelos: Vec<Modelo>,
    obtenida: Instant,
}

/// Una copia por upstream, cada una con su reloj: que caduque la de Hugging
/// Face no obliga a volver a pedir la de OpenRouter.
#[derive(Default)]
pub struct Catalogo {
    copias: [RwLock<Option<Copia>>; 2],
}

impl Catalogo {
    fn copia(&self, upstream: Upstream) -> &RwLock<Option<Copia>> {
        &self.copias[match upstream {
            Upstream::OpenRouter => 0,
            Upstream::HuggingFace => 1,
        }]
    }

    /// La copia guardada, solo si sigue vigente.
    pub fn vigente(&self, upstream: Upstream) -> Option<Vec<Modelo>> {
        let copia = self.copia(upstream).read().ok()?;
        let copia = copia.as_ref()?;
        (copia.obtenida.elapsed() < VIGENCIA).then(|| copia.modelos.clone())
    }

    /// La copia guardada aunque haya caducado. Último recurso.
    pub fn caducada(&self, upstream: Upstream) -> Option<Vec<Modelo>> {
        let copia = self.copia(upstream).read().ok()?;
        Some(copia.as_ref()?.modelos.clone())
    }

    /// Traduce la respuesta del upstream, la guarda y la devuelve. Los dos
    /// catálogos vienen como `{"data":[...]}`; cambia lo que hay dentro.
    pub fn guardar(&self, upstream: Upstream, bruto: &Value) -> Vec<Modelo> {
        let elementos = bruto.get("data").and_then(Value::as_array);
        let modelos: Vec<Modelo> = match upstream {
            Upstream::OpenRouter => elementos
                .map(|a| a.iter().filter_map(Modelo::desde_openrouter).collect())
                .unwrap_or_default(),
            Upstream::HuggingFace => elementos
                .map(|a| a.iter().flat_map(Modelo::desde_hf).collect())
                .unwrap_or_default(),
        };

        if let Ok(mut copia) = self.copia(upstream).write() {
            *copia = Some(Copia {
                modelos: modelos.clone(),
                obtenida: Instant::now(),
            });
        }
        modelos
    }

    /// Minutos desde que se trajo la copia, para enseñarlo en la consola.
    pub fn antiguedad_minutos(&self, upstream: Upstream) -> Option<u64> {
        let copia = self.copia(upstream).read().ok()?;
        Some(copia.as_ref()?.obtenida.elapsed().as_secs() / 60)
    }
}

pub fn filtrar(modelos: Vec<Modelo>, filtros: &Filtros) -> Vec<Modelo> {
    let mut lista: Vec<Modelo> = modelos.into_iter().filter(|m| filtros.pasa(m)).collect();
    // Los gratuitos primero y, dentro de cada grupo, del más barato al más caro.
    lista.sort_by(|a, b| {
        (a.entrada + a.salida)
            .partial_cmp(&(b.entrada + b.salida))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    lista
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    /// Recortes reales de las dos APIs, capturados el 06/10/2026: seis modelos
    /// de OpenRouter con `output_modalities=all` (texto, vídeo, imagen,
    /// transcripción y embeddings) y cuatro del router de Hugging Face.
    const OPENROUTER_TODO: &str = include_str!("../tests/fixtures/openrouter-models-all.json");
    const HF_ROUTER: &str = include_str!("../tests/fixtures/hf-router-models.json");

    fn bruto_completo() -> Value {
        json!({
            "id": "google/gemini-2.5-flash-lite",
            "canonical_slug": "google/gemini-2.5-flash-lite-20250617",
            "hugging_face_id": null,
            "name": "Google: Gemini 2.5 Flash Lite",
            "created": 1752000000,
            "description": "Modelo rápido y barato para volumen.",
            "context_length": 1048576,
            "architecture": {
                "input_modalities": ["text", "image", "file"],
                "output_modalities": ["text"]
            },
            "pricing": {
                "prompt": "0.0000001",
                "completion": "0.0000004",
                "image": "0.0004",
                "request": "0",
                "input_cache_read": "0.000000025",
                "web_search": "0.004"
            },
            "top_provider": { "max_completion_tokens": 65535, "is_moderated": false },
            "supported_parameters": ["tools", "structured_outputs", "temperature"]
        })
    }

    #[test]
    fn traduce_los_campos_ampliados() {
        let m = Modelo::desde_openrouter(&bruto_completo()).unwrap();
        assert_eq!(m.descripcion, "Modelo rápido y barato para volumen.");
        assert_eq!(m.modalidades_salida, vec!["text"]);
        assert_eq!(
            m.parametros,
            vec!["tools", "structured_outputs", "temperature"]
        );
        assert_eq!(m.max_salida, Some(65535));
        assert!(!m.moderado);
        // Por millón los de tokens; por unidad los demás.
        assert!((m.entrada - 0.1).abs() < 1e-9);
        assert!((m.precios.cache_lectura - 0.025).abs() < 1e-9);
        assert!((m.precios.imagen - 0.0004).abs() < 1e-12);
        assert!((m.precios.busqueda_web - 0.004).abs() < 1e-12);
        assert_eq!(m.precios.audio, 0.0);
        // Lo de la etapa 9.
        assert_eq!(m.upstream, Upstream::OpenRouter);
        assert_eq!(
            m.canonical_slug.as_deref(),
            Some("google/gemini-2.5-flash-lite-20250617")
        );
        assert_eq!(m.hugging_face_id, None, "null se queda en None");
    }

    #[test]
    fn sin_los_campos_nuevos_sigue_valiendo() {
        // Un elemento mínimo del catálogo: lo nuevo es opcional y queda vacío.
        let m = Modelo::desde_openrouter(&json!({ "id": "x/y" })).unwrap();
        assert_eq!(m.descripcion, "");
        assert!(m.modalidades_salida.is_empty());
        assert!(m.parametros.is_empty());
        assert_eq!(m.max_salida, None);
        assert_eq!(m.precios.imagen, 0.0);
        assert!(m.gratis);
        assert_eq!(m.canonical_slug, None);
        assert_eq!(m.hugging_face_id, None);
    }

    #[test]
    fn sin_id_no_hay_modelo() {
        assert!(Modelo::desde_openrouter(&json!({ "name": "sin id" })).is_none());
    }

    #[test]
    fn serializa_los_campos_nuevos() {
        let m = Modelo::desde_openrouter(&bruto_completo()).unwrap();
        let v = serde_json::to_value(&m).unwrap();
        for campo in [
            "descripcion",
            "precios",
            "modalidades_salida",
            "parametros",
            "max_salida",
            "moderado",
            "upstream",
            "canonical_slug",
            "hugging_face_id",
            "primer_token_ms",
            "tokens_por_segundo",
        ] {
            assert!(v.get(campo).is_some(), "falta {campo}");
        }
        let cache = v["precios"]["cache_lectura"].as_f64().unwrap();
        assert!((cache - 0.025).abs() < 1e-9, "{cache}");
        assert_eq!(v["upstream"], "openrouter");
        assert!(v["precios"].get("imagen_salida").is_some());
    }

    #[test]
    fn el_catalogo_completo_conserva_video_imagen_slug_y_hugging_face_id() {
        let bruto: Value = serde_json::from_str(OPENROUTER_TODO).unwrap();
        let catalogo = Catalogo::default();
        let modelos = catalogo.guardar(Upstream::OpenRouter, &bruto);
        assert_eq!(modelos.len(), 6);

        let por_id = |id: &str| modelos.iter().find(|m| m.id == id).unwrap();

        let video = por_id("alibaba/wan-3.0");
        assert_eq!(video.modalidades_salida, vec!["video"]);
        assert_eq!(
            video.canonical_slug.as_deref(),
            Some("alibaba/wan-3.0-20260824")
        );
        assert_eq!(video.hugging_face_id, None);

        let imagen = por_id("black-forest-labs/flux-3-image");
        assert_eq!(imagen.modalidades_salida, vec!["image"]);
        assert!(
            !imagen.gratis,
            "cobra por imagen generada aunque prompt y completion sean cero"
        );
        assert!((imagen.precios.imagen_salida - 4.91017964071857).abs() < 1e-6);

        let llama = por_id("meta-llama/llama-3.3-70b-instruct");
        assert_eq!(
            llama.hugging_face_id.as_deref(),
            Some("meta-llama/Llama-3.3-70B-Instruct"),
            "la llave hacia el otro upstream"
        );
        assert_eq!(
            llama.canonical_slug.as_deref(),
            Some("meta-llama/llama-3.3-70b-instruct")
        );

        // El filtro de salida deja solo lo pedido; sin él, todo.
        let solo_video = filtrar(
            modelos.clone(),
            &Filtros {
                salida: Some("video".into()),
                ..Filtros::default()
            },
        );
        assert_eq!(solo_video.len(), 1);
        assert_eq!(solo_video[0].id, "alibaba/wan-3.0");
        let texto = filtrar(
            modelos.clone(),
            &Filtros {
                salida: Some("TEXT".into()),
                ..Filtros::default()
            },
        );
        assert_eq!(texto.len(), 2);
        assert_eq!(filtrar(modelos.clone(), &Filtros::default()).len(), 6);

        // El modelo por defecto del servicio sigue encontrándose con su precio.
        assert!(catalogo.precio("openai/gpt-oss-120b").is_some());
        assert_eq!(catalogo.precio("hf:openai/gpt-oss-120b"), None);
    }

    #[test]
    fn el_catalogo_de_hugging_face_sale_con_una_entrada_por_host() {
        let bruto: Value = serde_json::from_str(HF_ROUTER).unwrap();
        let catalogo = Catalogo::default();
        let modelos = catalogo.guardar(Upstream::HuggingFace, &bruto);

        // gpt-oss-120b: 11 hosts, 10 con precio. Llama 3.3: 5, 4 con precio.
        // Qwen3.8-27B: 5, 4 con precio. gemma-3-4b-it: 2.
        let general = modelos
            .iter()
            .find(|m| m.id == "hf:openai/gpt-oss-120b")
            .expect("la entrada sin host");
        assert_eq!(general.upstream, Upstream::HuggingFace);
        assert_eq!(general.owned_by, "openai");
        assert_eq!(
            general.hugging_face_id.as_deref(),
            Some("openai/gpt-oss-120b")
        );
        assert_eq!(general.canonical_slug, None);
        assert_eq!(general.modalidades_salida, vec!["text"]);
        assert!(general.herramientas);
        assert_eq!(general.contexto, 131072);

        let por_host: Vec<&Modelo> = modelos
            .iter()
            .filter(|m| m.id.starts_with("hf:openai/gpt-oss-120b:"))
            .collect();
        assert_eq!(por_host.len(), 10, "un host sin precio no sale");
        assert!(!modelos
            .iter()
            .any(|m| m.id == "hf:openai/gpt-oss-120b:featherless-ai"));

        let groq = modelos
            .iter()
            .find(|m| m.id == "hf:openai/gpt-oss-120b:groq")
            .unwrap();
        assert_eq!(groq.entrada, 0.15);
        assert_eq!(groq.salida, 0.75);
        assert_eq!(groq.nombre, "openai/gpt-oss-120b · groq");
        assert_eq!(groq.primer_token_ms, Some(228.0));
        assert!(groq.tokens_por_segundo.unwrap() > 400.0);
        assert!(groq.herramientas && groq.json);

        // La general lleva el precio del más barato: deepinfra a 0.037 / 0.17.
        assert_eq!(general.entrada, 0.037);
        assert!((general.salida - 0.17).abs() < 1e-9, "{}", general.salida);
        let (entrada, salida) = catalogo.precio("hf:openai/gpt-oss-120b").unwrap();
        assert_eq!(entrada, 0.037);
        assert!((salida - 0.17).abs() < 1e-9);
        assert_eq!(
            catalogo.precio("hf:openai/gpt-oss-120b:groq"),
            Some((0.15, 0.75))
        );
        assert_eq!(
            catalogo.precio("hf:openai/gpt-oss-120b:featherless-ai"),
            None
        );

        // La imagen de entrada de Qwen se conserva.
        let qwen = modelos
            .iter()
            .find(|m| m.id == "hf:Qwen/Qwen3.8-27B")
            .unwrap();
        assert_eq!(qwen.modalidades, vec!["text", "image"]);
        assert_eq!(qwen.owned_by, "Qwen");

        // Las dos cachés son independientes.
        assert!(catalogo.vigente(Upstream::HuggingFace).is_some());
        assert!(catalogo.vigente(Upstream::OpenRouter).is_none());
        assert_eq!(catalogo.precio("openai/gpt-oss-120b"), None);
    }

    #[test]
    fn un_modelo_de_hugging_face_sin_hosts_con_precio_no_sale() {
        let sin_precio = json!({
            "id": "x/y",
            "providers": [{ "provider": "featherless-ai", "status": "live" }]
        });
        assert!(Modelo::desde_hf(&sin_precio).is_empty());
        assert!(Modelo::desde_hf(&json!({ "providers": [] })).is_empty());
    }
}
