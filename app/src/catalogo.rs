use std::{
    sync::RwLock,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;

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
}

/// Un modelo, con el formato de la API de OpenAI (`id`, `object`, `created`,
/// `owned_by`) más lo que hace falta para elegir: precio, contexto y qué sabe
/// hacer. Desde 0.6.2 lleva además lo que una aplicación necesita para
/// comparar modelos sin ir a OpenRouter: la descripción del proveedor, los
/// precios completos, las modalidades de salida y la lista de parámetros.
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
        };

        let parametros: Vec<&str> = bruto
            .get("supported_parameters")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        let lista = |valor: Option<&Value>| -> Vec<String> {
            valor
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
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
            gratis: entrada == 0.0 && salida == 0.0,
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
            object: "model",
            id,
            entrada,
            salida,
        })
    }
}

impl Catalogo {
    /// Precio de un modelo en dólares por millón, de la copia en memoria. Se
    /// usa para estimar el coste de una llamada; devuelve `None` si el
    /// catálogo aún no se ha traído o el modelo no está.
    pub fn precio(&self, id: &str) -> Option<(f64, f64)> {
        let lista = self.caducada()?;
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
    /// Recién pedido a OpenRouter.
    Fresco,
    /// Copia vigente.
    Cache,
    /// Copia caducada, porque OpenRouter no respondió. Mejor precios de ayer
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

#[derive(Default)]
pub struct Catalogo {
    copia: RwLock<Option<Copia>>,
}

impl Catalogo {
    /// La copia guardada, solo si sigue vigente.
    pub fn vigente(&self) -> Option<Vec<Modelo>> {
        let copia = self.copia.read().ok()?;
        let copia = copia.as_ref()?;
        (copia.obtenida.elapsed() < VIGENCIA).then(|| copia.modelos.clone())
    }

    /// La copia guardada aunque haya caducado. Último recurso.
    pub fn caducada(&self) -> Option<Vec<Modelo>> {
        let copia = self.copia.read().ok()?;
        Some(copia.as_ref()?.modelos.clone())
    }

    /// Traduce la respuesta de OpenRouter, la guarda y la devuelve.
    pub fn guardar(&self, bruto: &Value) -> Vec<Modelo> {
        let modelos: Vec<Modelo> = bruto
            .get("data")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Modelo::desde_openrouter).collect())
            .unwrap_or_default();

        if let Ok(mut copia) = self.copia.write() {
            *copia = Some(Copia {
                modelos: modelos.clone(),
                obtenida: Instant::now(),
            });
        }
        modelos
    }

    /// Minutos desde que se trajo la copia, para enseñarlo en la consola.
    pub fn antiguedad_minutos(&self) -> Option<u64> {
        let copia = self.copia.read().ok()?;
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

    fn bruto_completo() -> Value {
        json!({
            "id": "google/gemini-2.5-flash-lite",
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
        ] {
            assert!(v.get(campo).is_some(), "falta {campo}");
        }
        let cache = v["precios"]["cache_lectura"].as_f64().unwrap();
        assert!((cache - 0.025).abs() < 1e-9, "{cache}");
    }
}
