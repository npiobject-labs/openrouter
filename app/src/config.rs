use std::env;

/// Nombre y URL que se mandan a OpenRouter para identificar de dónde vienen las
/// llamadas. Aparecen en su panel de actividad.
pub const TITULO: &str = "openrouter (npiobject-labs)";
pub const REFERER: &str = "https://npiobject-labs.github.io/openrouter/";

const MODELO_DEFECTO: &str = "google/gemini-2.5-flash-lite";
const BASE_OPENROUTER: &str = "https://openrouter.ai/api/v1";

#[derive(Clone)]
pub struct Config {
    /// Clave de OpenRouter. Nunca sale del backend.
    pub clave_openrouter: Option<String>,
    /// Clave que exigimos a quien llama a /v1. Sin ella el servicio no atiende.
    pub clave_servicio: Option<String>,
    pub modelo_defecto: String,
    pub base_openrouter: String,
    pub build: String,
    pub puerto: u16,
    /// Fichero SQLite del histórico. Por defecto, el volumen de Fly.
    pub bd: Option<String>,
}

impl Config {
    pub fn del_entorno() -> Self {
        Self {
            clave_openrouter: variable("OPENROUTER_API_KEY"),
            clave_servicio: variable("SERVICIO_CLAVE"),
            modelo_defecto: variable("MODELO_DEFECTO")
                .unwrap_or_else(|| MODELO_DEFECTO.to_string()),
            base_openrouter: variable("OPENROUTER_BASE")
                .unwrap_or_else(|| BASE_OPENROUTER.to_string())
                .trim_end_matches('/')
                .to_string(),
            build: variable("BUILD_ID").unwrap_or_else(|| "dev".to_string()),
            // Fly siempre usa 8080; PUERTO solo lo fija tools/arrancar.ps1 al probar en el PC.
            puerto: variable("PUERTO")
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            bd: variable("BD_RUTA"),
        }
    }
}

/// Una variable vacía cuenta como no definida: `flyctl secrets` y los formularios
/// de GitHub dejan cadenas vacías con demasiada facilidad.
fn variable(nombre: &str) -> Option<String> {
    env::var(nombre)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
