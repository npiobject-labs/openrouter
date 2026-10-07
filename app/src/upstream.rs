//! A quién se le compra. Hasta la etapa 9 todo iba a OpenRouter; desde
//! entonces el servicio puede comprar también en el router de Hugging Face,
//! que con una sola clave da acceso a una docena de hosts (Groq, Cerebras,
//! Together, Fireworks, DeepInfra, Novita, Scaleway, OVHcloud, Nscale...).
//!
//! La elección la hace el prefijo del id de modelo: `hf:<id>` va a Hugging
//! Face quitando el prefijo; todo lo demás sigue yendo a OpenRouter tal cual.
//! Añadir un upstream es añadir una variante aquí y su conexión en
//! `openrouter::Cliente`, que es el único sitio que conoce las claves.

use serde::Serialize;

/// Prefijo con el que una aplicación pide un modelo del router de Hugging Face.
pub const PREFIJO_HF: &str = "hf:";
/// Prefijo de Requesty, el segundo agregador: `rq:<id de Requesty>`
/// (`rq:vertex/gemini-3.1-flash-lite@eu`).
pub const PREFIJO_RQ: &str = "rq:";

/// Se serializa con el mismo nombre que da `nombre()`: `openrouter` y `hf`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Upstream {
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// El router de Hugging Face (`router.huggingface.co/v1`), compatible con
    /// la API de OpenAI. El sufijo `:<host>` del id elige quién sirve
    /// (`:groq`, `:cerebras`...), y `:cheapest` y `:fastest` dejan que elija él.
    #[serde(rename = "hf")]
    HuggingFace,
    /// Requesty (`router.eu.requesty.ai/v1`), compatible con la API de OpenAI.
    /// Sus ids van por proveedor y región, no por fabricante, y cada respuesta
    /// trae el coste real en `usage.cost`.
    #[serde(rename = "rq")]
    Requesty,
}

/// Cómo se sabe lo que costó una llamada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Medicion {
    /// Se estima con el catálogo y después se reconcilia con `/generation`.
    Reconciliada,
    /// Solo la estimación con el catálogo: el upstream no tiene con qué
    /// reconciliar.
    Estimada,
    /// El coste real llega en la propia respuesta (`usage.cost`) y no hay nada
    /// que reconciliar después.
    Directa,
}

impl Upstream {
    pub const TODOS: [Upstream; 3] = [
        Upstream::OpenRouter,
        Upstream::HuggingFace,
        Upstream::Requesty,
    ];

    /// El nombre que viaja en la API: en `upstream` del registro de uso, en
    /// `?upstream=` y en `upstreams` de `/salud`.
    pub fn nombre(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "openrouter",
            Upstream::HuggingFace => "hf",
            Upstream::Requesty => "rq",
        }
    }

    /// El inverso de `nombre`, para los parámetros de la API. `None` si el
    /// texto no es ninguno: quien llama decide si es error o "todos".
    pub fn desde_nombre(texto: &str) -> Option<Self> {
        match texto.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(Upstream::OpenRouter),
            "hf" | "huggingface" | "hugging-face" => Some(Upstream::HuggingFace),
            "rq" | "requesty" => Some(Upstream::Requesty),
            _ => None,
        }
    }

    /// Lo que se antepone a los ids de su catálogo para que vuelvan a él.
    pub fn prefijo(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "",
            Upstream::HuggingFace => PREFIJO_HF,
            Upstream::Requesty => PREFIJO_RQ,
        }
    }

    pub fn medicion(self) -> Medicion {
        match self {
            Upstream::OpenRouter => Medicion::Reconciliada,
            Upstream::HuggingFace => Medicion::Estimada,
            Upstream::Requesty => Medicion::Directa,
        }
    }

    /// Qué secreto hay que definir para que responda.
    pub fn secreto(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "OPENROUTER_API_KEY",
            Upstream::HuggingFace => "HF_TOKEN",
            Upstream::Requesty => "REQUESTY_API_KEY",
        }
    }

    /// Códigos de error estables, uno por upstream. Los de OpenRouter son los
    /// publicados desde la etapa 1 y no cambian; los de Hugging Face siguen el
    /// mismo patrón para que una app los reconozca por el prefijo.
    pub fn codigo_rechaza(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "openrouter_rechaza",
            Upstream::HuggingFace => "hf_rechaza",
            Upstream::Requesty => "rq_rechaza",
        }
    }

    pub fn codigo_inalcanzable(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "openrouter_inalcanzable",
            Upstream::HuggingFace => "hf_inalcanzable",
            Upstream::Requesty => "rq_inalcanzable",
        }
    }

    pub fn codigo_tardo_demasiado(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "openrouter_tardo_demasiado",
            Upstream::HuggingFace => "hf_tardo_demasiado",
            Upstream::Requesty => "rq_tardo_demasiado",
        }
    }

    /// Cómo se le llama en los mensajes para personas.
    pub fn titulo(self) -> &'static str {
        match self {
            Upstream::OpenRouter => "OpenRouter",
            Upstream::HuggingFace => "Hugging Face",
            Upstream::Requesty => "Requesty",
        }
    }
}

/// A dónde va una llamada: el upstream y el id del modelo tal como lo
/// entiende él, ya sin nuestro prefijo.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Destino {
    pub upstream: Upstream,
    pub modelo: String,
}

impl Destino {
    /// Decide por el prefijo del id que pidió la aplicación. Sin prefijo
    /// conocido, OpenRouter con el id intacto: es exactamente lo de siempre.
    pub fn de(id_pedido: &str) -> Self {
        for upstream in [Upstream::HuggingFace, Upstream::Requesty] {
            if let Some(resto) = id_pedido.strip_prefix(upstream.prefijo()) {
                return Self {
                    upstream,
                    modelo: resto.to_string(),
                };
            }
        }
        Self {
            upstream: Upstream::OpenRouter,
            modelo: id_pedido.to_string(),
        }
    }

    /// El host que eligió la aplicación con el sufijo `:<host>`, si lo hay.
    /// `:cheapest` y `:fastest` no son hosts sino políticas del router, así que
    /// no cuentan.
    pub fn host(&self) -> Option<&str> {
        if self.upstream != Upstream::HuggingFace {
            return None;
        }
        let (_, sufijo) = self.modelo.rsplit_once(':')?;
        (!sufijo.is_empty() && !matches!(sufijo, "cheapest" | "fastest")).then_some(sufijo)
    }
}

/// Los datos de conexión de un upstream. Vive dentro de `openrouter::Cliente`
/// y de ahí no sale: ni la clave ni la base se enseñan por la API.
pub struct Conexion {
    pub upstream: Upstream,
    pub base: String,
    pub clave: Option<String>,
}

impl Conexion {
    pub fn configurada(&self) -> bool {
        self.clave.is_some()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_prefijo_decide_el_upstream() {
        let d = Destino::de("hf:meta-llama/Llama-3.3-70B-Instruct:groq");
        assert_eq!(d.upstream, Upstream::HuggingFace);
        assert_eq!(d.modelo, "meta-llama/Llama-3.3-70B-Instruct:groq");
        assert_eq!(d.host(), Some("groq"));

        // Sin prefijo, OpenRouter y el id tal cual, con sus dos puntos y todo.
        let d = Destino::de("google/gemini-3.1-flash-lite:free");
        assert_eq!(d.upstream, Upstream::OpenRouter);
        assert_eq!(d.modelo, "google/gemini-3.1-flash-lite:free");
        assert_eq!(d.host(), None, "en OpenRouter el sufijo no es un host");
    }

    #[test]
    fn el_prefijo_rq_va_a_requesty_con_el_id_entero() {
        let d = Destino::de("rq:vertex/gemini-3.1-flash-lite@eu");
        assert_eq!(d.upstream, Upstream::Requesty);
        assert_eq!(d.modelo, "vertex/gemini-3.1-flash-lite@eu");
        assert_eq!(d.host(), None, "en Requesty no hay sufijo de host");
        assert_eq!(Upstream::Requesty.medicion(), Medicion::Directa);
    }

    #[test]
    fn cheapest_y_fastest_no_son_hosts() {
        assert_eq!(Destino::de("hf:openai/gpt-oss-120b:cheapest").host(), None);
        assert_eq!(Destino::de("hf:openai/gpt-oss-120b:fastest").host(), None);
        assert_eq!(Destino::de("hf:openai/gpt-oss-120b").host(), None);
        assert_eq!(
            Destino::de("hf:openai/gpt-oss-120b:cerebras").host(),
            Some("cerebras")
        );
    }

    #[test]
    fn el_nombre_va_y_vuelve() {
        for u in Upstream::TODOS {
            assert_eq!(Upstream::desde_nombre(u.nombre()), Some(u));
            assert_eq!(serde_json::to_value(u).unwrap(), u.nombre());
        }
        assert_eq!(Upstream::desde_nombre("HF"), Some(Upstream::HuggingFace));
        assert_eq!(Upstream::desde_nombre("otro"), None);
    }

    #[test]
    fn solo_openrouter_reconcilia() {
        assert_eq!(Upstream::OpenRouter.medicion(), Medicion::Reconciliada);
        assert_eq!(Upstream::HuggingFace.medicion(), Medicion::Estimada);
        // Los códigos publicados de OpenRouter no cambian.
        assert_eq!(Upstream::OpenRouter.codigo_rechaza(), "openrouter_rechaza");
        assert_eq!(Upstream::HuggingFace.codigo_rechaza(), "hf_rechaza");
    }
}
