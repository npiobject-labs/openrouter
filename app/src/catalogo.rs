use std::{
    sync::RwLock,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;

/// El catálogo de OpenRouter cambia a diario, no a cada minuto: una hora de
/// caché ahorra una llamada por consulta sin servir precios viejos.
const VIGENCIA: Duration = Duration::from_secs(3600);

/// Un modelo, con el formato de la API de OpenAI (`id`, `object`, `created`,
/// `owned_by`) más lo que hace falta para elegir: precio, contexto y qué sabe
/// hacer.
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

        let parametros: Vec<&str> = bruto
            .get("supported_parameters")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        let modalidades = bruto
            .get("architecture")
            .and_then(|a| a.get("input_modalities"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

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
            object: "model",
            id,
            entrada,
            salida,
        })
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
