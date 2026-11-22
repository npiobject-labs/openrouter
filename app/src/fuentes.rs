//! Fuentes declarativas (etapa 12; B1 del plan de la Plataforma). Además de
//! los tres upstreams con código propio (OpenRouter, Hugging Face y Requesty),
//! cualquier API compatible con OpenAI se da de alta **en caliente**, desde la
//! consola y con la clave de administración, sin desplegar: base, clave, ruta
//! del catálogo, región, retención y cómo se sabe el coste. Un modelo de una
//! fuente se pide como `<fuente>:<modelo>` (`groq:llama-3.3-70b-versatile`).
//!
//! La clave de cada fuente se guarda **cifrada** (ChaCha20-Poly1305) con una
//! clave maestra que vive solo en el entorno (`FUENTES_CLAVE_MAESTRA`), nunca
//! se devuelve por la API y solo se descifra para hacer la llamada.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::ErrorApi;

pub const ESQUEMA: &str = "
CREATE TABLE IF NOT EXISTS fuentes (
    id            TEXT PRIMARY KEY,
    nombre        TEXT NOT NULL,
    tipo          TEXT NOT NULL,
    base          TEXT NOT NULL,
    clave         TEXT,
    ruta_modelos  TEXT NOT NULL DEFAULT '/models',
    region        TEXT,
    retencion     TEXT,
    coste         TEXT NOT NULL DEFAULT 'usage',
    precio_entrada REAL,
    precio_salida  REAL,
    activa        INTEGER NOT NULL DEFAULT 1,
    creada        TEXT NOT NULL,
    actualizada   TEXT NOT NULL,
    salud         TEXT
);
";

/// Ids que no puede tomar una fuente: son prefijos o nombres ya usados.
pub const RESERVADOS: [&str; 9] = [
    "openrouter",
    "hf",
    "huggingface",
    "rq",
    "requesty",
    "higgsfield",
    "auto",
    "alias",
    "todas",
];
pub const TIPOS: [&str; 4] = ["agregador", "proveedor", "local", "openai_compatible"];
/// Cuánto dura el catálogo de una fuente en caché.
const CACHE: Duration = Duration::from_secs(3600);
const TIEMPO_CHAT: Duration = Duration::from_secs(180);
const TIEMPO_CATALOGO: Duration = Duration::from_secs(20);

/// Una fuente dada de alta. La clave va cifrada y no se serializa nunca.
#[derive(Clone, Debug, Serialize)]
pub struct Fuente {
    pub id: String,
    pub nombre: String,
    pub tipo: String,
    pub base: String,
    pub ruta_modelos: String,
    pub region: Option<String>,
    pub retencion: Option<String>,
    /// `usage`: el coste llega en `usage.cost`; `catalogo`: se estima con los
    /// precios del catálogo de la fuente o, si no los da, con los declarados.
    pub coste: String,
    pub precio_entrada: Option<f64>,
    pub precio_salida: Option<f64>,
    pub activa: bool,
    pub creada: String,
    pub actualizada: String,
    pub salud: Option<Value>,
    #[serde(skip)]
    pub clave_cifrada: Option<String>,
}

impl Fuente {
    pub fn prefijo(&self) -> String {
        format!("{}:", self.id)
    }

    /// Lo que se enseña por `GET /v1/fuentes`. Una app no ve la base (puede
    /// ser un túnel privado); la administración sí. La clave, nadie.
    pub fn publica(&self, admin: bool) -> Value {
        let mut v = json!({
            "id": self.id, "nombre": self.nombre, "tipo": self.tipo, "prefijo": self.prefijo(),
            "declarativa": true, "configurada": self.activa, "region": self.region,
            "retencion": self.retencion,
            "medicion": if self.coste == "usage" { "directa" } else { "estimada" },
            "salud": self.salud,
        });
        if admin {
            v["base"] = json!(self.base);
            v["ruta_modelos"] = json!(self.ruta_modelos);
            v["coste"] = json!(self.coste);
            v["precio_entrada"] = json!(self.precio_entrada);
            v["precio_salida"] = json!(self.precio_salida);
            v["activa"] = json!(self.activa);
            v["con_clave"] = json!(self.clave_cifrada.is_some());
            v["creada"] = json!(self.creada);
            v["actualizada"] = json!(self.actualizada);
        }
        v
    }
}

/// `[a-z][a-z0-9-]{1,23}` y no reservado: se usa como prefijo del modelo.
pub fn id_valido(id: &str) -> bool {
    let mut c = id.chars();
    matches!(c.next(), Some('a'..='z'))
        && id.len() >= 2
        && id.len() <= 24
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !RESERVADOS.contains(&id)
}

// --- Cifrado de las claves ---

fn llave(maestra: &str) -> Key {
    let h = Sha256::digest(maestra.as_bytes());
    *Key::from_slice(&h)
}

/// `v1:` + hex(nonce ‖ cifrado). El nonce (12 bytes) lo da quien llama:
/// el `randomblob` del propio SQLite, como las claves de aplicación.
pub fn cifrar(maestra: &str, nonce: &[u8; 12], texto: &str) -> Result<String, String> {
    let c = ChaCha20Poly1305::new(&llave(maestra));
    let cifrado = c
        .encrypt(Nonce::from_slice(nonce), texto.as_bytes())
        .map_err(|_| "no se pudo cifrar la clave".to_string())?;
    let mut todo = nonce.to_vec();
    todo.extend(cifrado);
    Ok(format!("v1:{}", hex(&todo)))
}

pub fn descifrar(maestra: &str, guardado: &str) -> Option<String> {
    let bytes = de_hex(guardado.strip_prefix("v1:")?)?;
    if bytes.len() < 12 + 16 {
        return None;
    }
    let (nonce, cifrado) = bytes.split_at(12);
    let claro = ChaCha20Poly1305::new(&llave(maestra))
        .decrypt(Nonce::from_slice(nonce), cifrado)
        .ok()?;
    String::from_utf8(claro).ok()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn de_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn nonce(conexion: &Connection) -> rusqlite::Result<[u8; 12]> {
    let v: Vec<u8> = conexion.query_one("SELECT randomblob(12)", [], |f| f.get(0))?;
    let mut n = [0u8; 12];
    n.copy_from_slice(&v[..12]);
    Ok(n)
}

// --- Base de datos ---

fn desde_fila(f: &rusqlite::Row) -> rusqlite::Result<Fuente> {
    let salud: Option<String> = f.get("salud")?;
    Ok(Fuente {
        id: f.get("id")?,
        nombre: f.get("nombre")?,
        tipo: f.get("tipo")?,
        base: f.get("base")?,
        ruta_modelos: f.get("ruta_modelos")?,
        region: f.get("region")?,
        retencion: f.get("retencion")?,
        coste: f.get("coste")?,
        precio_entrada: f.get("precio_entrada")?,
        precio_salida: f.get("precio_salida")?,
        activa: f.get::<_, i64>("activa")? != 0,
        creada: f.get("creada")?,
        actualizada: f.get("actualizada")?,
        salud: salud.and_then(|s| serde_json::from_str(&s).ok()),
        clave_cifrada: f.get("clave")?,
    })
}

pub fn todas(conexion: &Connection) -> Vec<Fuente> {
    let Ok(mut st) = conexion.prepare("SELECT * FROM fuentes ORDER BY id") else {
        return vec![];
    };
    st.query_map([], desde_fila)
        .map(|f| f.flatten().collect())
        .unwrap_or_default()
}

pub fn una(conexion: &Connection, id: &str) -> Option<Fuente> {
    conexion
        .query_row("SELECT * FROM fuentes WHERE id = ?1", [id], desde_fila)
        .optional()
        .ok()
        .flatten()
}

/// Alta o cambio. Si no viene clave nueva, se conserva la que había.
pub fn guardar(conexion: &Connection, f: &Fuente) -> rusqlite::Result<()> {
    conexion.execute(
        "INSERT INTO fuentes (id, nombre, tipo, base, clave, ruta_modelos, region, retencion, coste,
           precio_entrada, precio_salida, activa, creada, actualizada)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12)
         ON CONFLICT(id) DO UPDATE SET nombre = excluded.nombre, tipo = excluded.tipo, base = excluded.base,
           clave = COALESCE(excluded.clave, fuentes.clave), ruta_modelos = excluded.ruta_modelos,
           region = excluded.region, retencion = excluded.retencion, coste = excluded.coste,
           precio_entrada = excluded.precio_entrada, precio_salida = excluded.precio_salida,
           activa = 1, actualizada = excluded.actualizada",
        params![
            f.id,
            f.nombre,
            f.tipo,
            f.base,
            f.clave_cifrada,
            f.ruta_modelos,
            f.region,
            f.retencion,
            f.coste,
            f.precio_entrada,
            f.precio_salida,
            f.actualizada
        ],
    )?;
    Ok(())
}

/// Dar de baja desactiva, no borra: el uso histórico la referencia.
pub fn desactivar(conexion: &Connection, id: &str, fecha: &str) -> bool {
    conexion
        .execute(
            "UPDATE fuentes SET activa = 0, actualizada = ?2 WHERE id = ?1 AND activa = 1",
            params![id, fecha],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

pub fn anota_salud(conexion: &Connection, id: &str, salud: &Value) {
    let _ = conexion.execute(
        "UPDATE fuentes SET salud = ?2 WHERE id = ?1",
        params![id, salud.to_string()],
    );
}

/// `groq:llama-3.3-70b` → (`groq`, `llama-3.3-70b`) si `groq` es una fuente
/// activa. Los ids de OpenRouter llevan barra antes de los dos puntos
/// (`google/x:free`), así que nunca se confunden con un prefijo.
pub fn partir(pedido: &str) -> Option<(&str, &str)> {
    let (prefijo, resto) = pedido.split_once(':')?;
    (id_valido(prefijo) && !resto.is_empty()).then_some((prefijo, resto))
}

// --- HTTP ---

/// Caché del catálogo de cada fuente: (cuándo, modelos ya normalizados).
#[derive(Default)]
pub struct CatalogoFuentes {
    copias: Mutex<HashMap<String, (Instant, Vec<Value>)>>,
}

impl CatalogoFuentes {
    pub fn vigente(&self, id: &str) -> Option<Vec<Value>> {
        let g = self.copias.lock().ok()?;
        let (cuando, lista) = g.get(id)?;
        (cuando.elapsed() < CACHE).then(|| lista.clone())
    }
    pub fn caducada(&self, id: &str) -> Option<Vec<Value>> {
        self.copias.lock().ok()?.get(id).map(|(_, l)| l.clone())
    }
    pub fn guardar(&self, id: &str, lista: Vec<Value>) {
        if let Ok(mut g) = self.copias.lock() {
            g.insert(id.to_string(), (Instant::now(), lista));
        }
    }
    pub fn olvidar(&self, id: &str) {
        if let Ok(mut g) = self.copias.lock() {
            g.remove(id);
        }
    }
    /// Precio (entrada, salida) por millón de un modelo ya pedido con prefijo.
    pub fn precio(&self, id_fuente: &str, id_completo: &str) -> Option<(f64, f64)> {
        let g = self.copias.lock().ok()?;
        let m = g
            .get(id_fuente)?
            .1
            .iter()
            .find(|m| m["id"] == id_completo)?;
        Some((m["entrada"].as_f64()?, m["salida"].as_f64()?))
    }
}

fn url(base: &str, ruta: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        ruta.trim_start_matches('/')
    )
}

fn por_millon(v: &Value) -> Option<f64> {
    let x = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))?;
    Some(x * 1_000_000.0)
}

/// Un modelo del catálogo de la fuente, con la forma del de `/v1/models`.
pub fn normalizar(f: &Fuente, m: &Value) -> Option<Value> {
    let id = m["id"].as_str()?;
    let entrada = por_millon(&m["pricing"]["prompt"]).or(f.precio_entrada);
    let salida = por_millon(&m["pricing"]["completion"]).or(f.precio_salida);
    let contexto = [
        "context_length",
        "context_window",
        "max_model_len",
        "max_context_length",
    ]
    .iter()
    .find_map(|k| m[*k].as_u64())
    .unwrap_or(0);
    Some(json!({
        "id": format!("{}:{id}", f.id), "object": "model", "created": m["created"].as_i64().unwrap_or(0),
        "owned_by": m["owned_by"].as_str().unwrap_or(&f.id), "nombre": m["name"].as_str().unwrap_or(id),
        "contexto": contexto, "entrada": entrada.unwrap_or(0.0), "salida": salida.unwrap_or(0.0),
        "precio_conocido": entrada.is_some() && salida.is_some(),
        "gratis": entrada == Some(0.0) && salida == Some(0.0), "herramientas": false, "json": false,
        "modalidades": ["text"], "modalidades_salida": ["text"], "upstream": f.id,
        "region": f.region, "retencion": f.retencion, "canonical_slug": null,
        "hugging_face_id": m["hugging_face_id"].as_str(),
    }))
}

fn error_red(e: reqwest::Error, f: &Fuente) -> ErrorApi {
    if e.is_timeout() {
        ErrorApi::nuevo(
            StatusCode::GATEWAY_TIMEOUT,
            "fuente_tardo_demasiado",
            format!("La fuente {} no respondió a tiempo.", f.nombre),
        )
    } else {
        ErrorApi::nuevo(
            StatusCode::BAD_GATEWAY,
            "fuente_inalcanzable",
            format!("No se pudo hablar con la fuente {}.", f.nombre),
        )
    }
}

fn rechazo(f: &Fuente, estado: StatusCode, cuerpo: Value) -> ErrorApi {
    let mensaje = cuerpo["error"]["message"]
        .as_str()
        .or_else(|| cuerpo["error"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("La fuente {} respondió {estado}.", f.nombre));
    ErrorApi::nuevo(estado, "fuente_rechaza", mensaje).con_upstream(cuerpo)
}

/// El catálogo de la fuente (`GET <base><ruta_modelos>`), normalizado.
pub async fn modelos(
    http: &reqwest::Client,
    f: &Fuente,
    clave: Option<&str>,
) -> Result<Vec<Value>, ErrorApi> {
    let mut req = http
        .get(url(&f.base, &f.ruta_modelos))
        .timeout(TIEMPO_CATALOGO);
    if let Some(c) = clave {
        req = req.bearer_auth(c);
    }
    let r = req.send().await.map_err(|e| error_red(e, f))?;
    let estado = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let cuerpo: Value = r.json().await.unwrap_or(Value::Null);
    if !estado.is_success() {
        return Err(rechazo(f, estado, cuerpo));
    }
    let lista = cuerpo["data"]
        .as_array()
        .or_else(|| cuerpo.as_array())
        .ok_or_else(|| {
            ErrorApi::nuevo(
                StatusCode::BAD_GATEWAY,
                "fuente_rechaza",
                format!("El catálogo de {} no trae una lista `data`.", f.nombre),
            )
        })?;
    Ok(lista.iter().filter_map(|m| normalizar(f, m)).collect())
}

/// `POST <base>/chat/completions` con la clave de la fuente.
pub async fn chat(
    http: &reqwest::Client,
    f: &Fuente,
    clave: Option<&str>,
    cuerpo: &Value,
) -> Result<Value, ErrorApi> {
    let mut req = http
        .post(url(&f.base, "/chat/completions"))
        .timeout(TIEMPO_CHAT)
        .json(cuerpo);
    if let Some(c) = clave {
        req = req.bearer_auth(c);
    }
    let r = req.send().await.map_err(|e| error_red(e, f))?;
    let estado = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let respuesta: Value = r.json().await.unwrap_or(Value::Null);
    if !estado.is_success() {
        return Err(rechazo(f, estado, respuesta));
    }
    Ok(respuesta)
}

/// Salud de una fuente: pide su catálogo y mide cuánto tarda.
pub async fn probar(
    http: &reqwest::Client,
    f: &Fuente,
    clave: Option<&str>,
    fecha: &str,
) -> (Value, Option<Vec<Value>>) {
    let reloj = Instant::now();
    let r = modelos(http, f, clave).await;
    let ms = reloj.elapsed().as_millis() as u64;
    match r {
        Ok(lista) => (
            json!({ "ok": true, "latencia_ms": ms, "modelos": lista.len(), "comprobada": fecha }),
            Some(lista),
        ),
        Err(e) => (
            json!({ "ok": false, "latencia_ms": ms, "error": e.codigo, "mensaje": e.mensaje, "comprobada": fecha }),
            None,
        ),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_cifrado_va_y_vuelve_y_no_se_lee_sin_la_maestra() {
        let n = [7u8; 12];
        let c = cifrar("maestra-de-prueba", &n, "sk-secreta").unwrap();
        assert!(c.starts_with("v1:") && !c.contains("sk-secreta"));
        assert_eq!(
            descifrar("maestra-de-prueba", &c).as_deref(),
            Some("sk-secreta")
        );
        assert_eq!(descifrar("otra", &c), None, "otra maestra no descifra");
        let mut roto = c.clone();
        roto.replace_range(roto.len() - 2.., "00");
        assert_eq!(
            descifrar("maestra-de-prueba", &roto),
            None,
            "manipulada no descifra"
        );
    }

    #[test]
    fn ids_y_prefijos() {
        assert!(id_valido("groq") && id_valido("mi-ollama2"));
        assert!(!id_valido("hf") && !id_valido("rq") && !id_valido("Groq") && !id_valido("a"));
        assert!(!id_valido("google/gemini"));
        assert_eq!(
            partir("groq:llama-3.3-70b"),
            Some(("groq", "llama-3.3-70b"))
        );
        assert_eq!(
            partir("google/gemini-3:free"),
            None,
            "los de OpenRouter no son fuentes"
        );
        assert_eq!(partir("hf:meta-llama/x"), None, "hf es reservado");
        assert_eq!(partir("groq:"), None);
    }

    #[test]
    fn normaliza_precios_por_token_o_los_declarados() {
        let f = Fuente {
            id: "prueba".into(),
            nombre: "Prueba".into(),
            tipo: "openai_compatible".into(),
            base: "http://x".into(),
            ruta_modelos: "/models".into(),
            region: Some("eu".into()),
            retencion: None,
            coste: "catalogo".into(),
            precio_entrada: Some(0.5),
            precio_salida: Some(1.5),
            activa: true,
            creada: String::new(),
            actualizada: String::new(),
            salud: None,
            clave_cifrada: None,
        };
        let m = normalizar(&f, &json!({"id": "m1", "pricing": {"prompt": "0.000001", "completion": "0.000002"}, "context_length": 8192})).unwrap();
        assert_eq!(m["id"], "prueba:m1");
        assert!((m["entrada"].as_f64().unwrap() - 1.0).abs() < 1e-9 && m["contexto"] == 8192);
        let m = normalizar(&f, &json!({"id": "m2"})).unwrap();
        assert_eq!(
            (m["entrada"].as_f64(), m["salida"].as_f64()),
            (Some(0.5), Some(1.5))
        );
        assert_eq!(m["region"], "eu");
    }
}
