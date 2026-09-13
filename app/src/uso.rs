use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;

use crate::apps;

/// Dónde vive el histórico. En Fly es el volumen montado en `/datos`; si no se
/// puede abrir, el servicio sigue funcionando en memoria y lo dice en
/// `/v1/estado`, que es como se detecta que el volumen no está.
const RUTA_DEFECTO: &str = "/datos/uso.db";

const ESQUEMA: &str = "
CREATE TABLE IF NOT EXISTS uso (
    id             TEXT PRIMARY KEY,
    fecha          TEXT    NOT NULL,
    modelo_pedido  TEXT    NOT NULL,
    modelo_servido TEXT,
    proveedor      TEXT,
    tokens_entrada INTEGER NOT NULL DEFAULT 0,
    tokens_salida  INTEGER NOT NULL DEFAULT 0,
    tokens_razonamiento INTEGER NOT NULL DEFAULT 0,
    tokens_cache   INTEGER NOT NULL DEFAULT 0,
    coste          REAL,
    coste_origen   TEXT    NOT NULL,
    latencia_ms    INTEGER NOT NULL DEFAULT 0,
    motivo_fin     TEXT,
    estado         INTEGER NOT NULL DEFAULT 0,
    id_openrouter  TEXT,
    app_id         TEXT,
    operacion      TEXT
);
CREATE INDEX IF NOT EXISTS uso_fecha ON uso(fecha);
CREATE INDEX IF NOT EXISTS uso_modelo ON uso(modelo_servido);
";

/// Índices sobre columnas que pueden faltar en una base anterior: van después
/// de asegurarlas, nunca en el esquema.
const INDICES: &str = "
CREATE INDEX IF NOT EXISTS uso_app ON uso(app_id);
";

/// Lo que se sabe de una llamada. Se anota siempre, también cuando falla: un
/// error que costó dos segundos de espera es información de uso.
#[derive(Clone, Serialize)]
pub struct Registro {
    pub id: String,
    pub fecha: String,
    pub modelo_pedido: String,
    pub modelo_servido: Option<String>,
    pub proveedor: Option<String>,
    pub tokens_entrada: i64,
    pub tokens_salida: i64,
    pub tokens_razonamiento: i64,
    pub tokens_cache: i64,
    /// Dólares. `None` mientras no se pueda ni estimar.
    pub coste: Option<f64>,
    /// `estimado` con los precios del catálogo, `openrouter` si ya se reconcilió.
    pub coste_origen: String,
    pub latencia_ms: i64,
    pub motivo_fin: Option<String>,
    pub estado: u16,
    /// El id de la generación en OpenRouter, con el que se reconcilia el coste.
    pub id_openrouter: Option<String>,
    /// Qué aplicación llamó. `None` es la clave de administración.
    pub app_id: Option<String>,
    /// Lo que venga en la cabecera `X-Operacion`: el trabajo de negocio al que
    /// pertenece la llamada, para agrupar varias de un mismo presupuesto.
    pub operacion: Option<String>,
}

impl Registro {
    /// Saca de la respuesta lo que se puede medir sin consultar a nadie.
    pub fn desde_respuesta(&mut self, cuerpo: &Value) {
        self.id_openrouter = texto(cuerpo.get("id"));
        self.modelo_servido = texto(cuerpo.get("model"));
        self.proveedor = texto(cuerpo.get("provider"));

        if let Some(u) = cuerpo.get("usage") {
            self.tokens_entrada = entero(u.get("prompt_tokens"));
            self.tokens_salida = entero(u.get("completion_tokens"));
            self.tokens_razonamiento = u
                .get("completion_tokens_details")
                .map(|d| entero(d.get("reasoning_tokens")))
                .unwrap_or(0);
            self.tokens_cache = u
                .get("prompt_tokens_details")
                .map(|d| entero(d.get("cached_tokens")))
                .unwrap_or(0);
            // OpenRouter añade "cost" a usage cuando la cuenta lo expone. Si
            // viene, es el dato bueno y ahorra la reconciliación.
            if let Some(c) = u.get("cost").and_then(Value::as_f64) {
                self.coste = Some(c);
                self.coste_origen = "openrouter".into();
            }
        }

        self.motivo_fin = cuerpo
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .and_then(|e| texto(e.get("finish_reason")));
    }

    /// Coste con los precios del catálogo, que están en dólares por millón.
    /// Solo se aplica si no hay ya un coste dado por OpenRouter.
    pub fn estima(&mut self, precios: Option<(f64, f64)>) {
        if self.coste_origen == "openrouter" {
            return;
        }
        if let Some((entrada, salida)) = precios {
            let total = (self.tokens_entrada as f64 * entrada + self.tokens_salida as f64 * salida)
                / 1_000_000.0;
            self.coste = Some(total);
            self.coste_origen = "estimado".into();
        }
    }

    fn desde_fila(f: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: f.get("id")?,
            fecha: f.get("fecha")?,
            modelo_pedido: f.get("modelo_pedido")?,
            modelo_servido: f.get("modelo_servido")?,
            proveedor: f.get("proveedor")?,
            tokens_entrada: f.get("tokens_entrada")?,
            tokens_salida: f.get("tokens_salida")?,
            tokens_razonamiento: f.get("tokens_razonamiento")?,
            tokens_cache: f.get("tokens_cache")?,
            coste: f.get("coste")?,
            coste_origen: f.get("coste_origen")?,
            latencia_ms: f.get("latencia_ms")?,
            motivo_fin: f.get("motivo_fin")?,
            estado: f.get("estado")?,
            id_openrouter: f.get("id_openrouter")?,
            app_id: f.get("app_id")?,
            operacion: f.get("operacion")?,
        })
    }
}

fn texto(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn entero(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

/// Cómo agrupar el resumen.
pub enum Agrupacion {
    Dia,
    Modelo,
    App,
}

impl Agrupacion {
    /// La columna por la que se agrupa. Nunca viene del usuario sin pasar por
    /// aquí: es la única forma de que el `GROUP BY` no sea inyectable.
    pub fn desde(texto: Option<&str>) -> Self {
        match texto {
            Some("modelo") => Self::Modelo,
            Some("app") => Self::App,
            _ => Self::Dia,
        }
    }

    fn columna(&self) -> &'static str {
        match self {
            // La fecha es ISO, así que el día son sus diez primeros caracteres.
            Self::Dia => "substr(fecha, 1, 10)",
            Self::Modelo => "coalesce(modelo_servido, modelo_pedido)",
            Self::App => "coalesce(app_id, 'administracion')",
        }
    }
}

/// El histórico de llamadas. Una sola conexión bajo mutex: las consultas son de
/// microsegundos y solo hay una máquina, porque el volumen de Fly obliga a
/// `--ha=false`. Un pool aquí sería complicar sin ganar nada.
pub struct Uso {
    conexion: Mutex<Connection>,
    en_disco: bool,
    contador: AtomicU64,
}

impl Uso {
    /// Abre el histórico. Si el fichero no se puede abrir (no hay volumen, o no
    /// hay permiso), avisa por el log y sigue en memoria: perder el histórico es
    /// malo, pero dejar de servir consultas por eso lo es más.
    pub fn nuevo(ruta: Option<&str>) -> Self {
        let ruta = ruta.unwrap_or(RUTA_DEFECTO);

        let en_fichero = Self::en_fichero(ruta).and_then(|c| {
            prepara(&c)?;
            Ok(c)
        });

        let (conexion, en_disco) = match en_fichero {
            Ok(c) => (c, true),
            Err(e) => {
                eprintln!("no se pudo usar {ruta} ({e}); el uso se guarda solo en memoria");
                let memoria =
                    Connection::open_in_memory().expect("SQLite en memoria siempre debe abrir");
                prepara(&memoria).expect("el esquema en memoria debe crearse");
                (memoria, false)
            }
        };

        Self {
            conexion: Mutex::new(conexion),
            en_disco,
            contador: AtomicU64::new(0),
        }
    }

    fn en_fichero(ruta: &str) -> rusqlite::Result<Connection> {
        let conexion = Connection::open(ruta)?;
        // WAL para que una lectura larga no bloquee la anotación de una llamada.
        conexion.pragma_update(None, "journal_mode", "WAL")?;
        conexion.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(conexion)
    }

    pub fn almacen(&self) -> &'static str {
        if self.en_disco {
            "sqlite"
        } else {
            "memoria"
        }
    }

    /// Abre el registro de una llamada. El id se devuelve al cliente en
    /// `X-Uso-Id` antes de saber siquiera si la llamada irá bien.
    pub fn abre(&self, modelo_pedido: String) -> Registro {
        let n = self.contador.fetch_add(1, Ordering::Relaxed);
        let ahora = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Registro {
            id: format!("u-{ahora}-{n}"),
            fecha: iso(ahora / 1000),
            modelo_pedido,
            modelo_servido: None,
            proveedor: None,
            tokens_entrada: 0,
            tokens_salida: 0,
            tokens_razonamiento: 0,
            tokens_cache: 0,
            coste: None,
            coste_origen: "desconocido".into(),
            latencia_ms: 0,
            motivo_fin: None,
            estado: 0,
            id_openrouter: None,
            app_id: None,
            operacion: None,
        }
    }

    pub fn anota(&self, r: Registro) {
        let conexion = self.conexion.lock().unwrap();
        let hecho = conexion.execute(
            "INSERT OR REPLACE INTO uso (id, fecha, modelo_pedido, modelo_servido, proveedor,
                tokens_entrada, tokens_salida, tokens_razonamiento, tokens_cache,
                coste, coste_origen, latencia_ms, motivo_fin, estado, id_openrouter,
                app_id, operacion)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                r.id, r.fecha, r.modelo_pedido, r.modelo_servido, r.proveedor,
                r.tokens_entrada, r.tokens_salida, r.tokens_razonamiento, r.tokens_cache,
                r.coste, r.coste_origen, r.latencia_ms, r.motivo_fin, r.estado, r.id_openrouter,
                r.app_id, r.operacion
            ],
        );
        if let Err(e) = hecho {
            eprintln!("no se pudo anotar el uso {}: {e}", r.id);
        }
    }

    /// Los últimos `n`, del más reciente al más antiguo. Con `app`, solo los de
    /// esa aplicación.
    pub fn ultimos(&self, n: usize, app: Option<&str>) -> Vec<Registro> {
        self.consulta(
            "SELECT * FROM uso
             WHERE (?2 IS NULL OR app_id = ?2)
             ORDER BY fecha DESC, rowid DESC LIMIT ?1",
            params![n as i64, app],
        )
    }

    /// Presta la conexión. La base es una sola y este módulo es su dueño; las
    /// aplicaciones viven en el mismo fichero y la piden por aquí.
    pub fn con<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        let conexion = self.conexion.lock().unwrap();
        f(&conexion)
    }

    pub fn uno(&self, id: &str) -> Option<Registro> {
        let conexion = self.conexion.lock().unwrap();
        conexion
            .query_one("SELECT * FROM uso WHERE id = ?1", params![id], |f| {
                Registro::desde_fila(f)
            })
            .optional()
            .unwrap_or(None)
    }

    pub fn total(&self) -> usize {
        let conexion = self.conexion.lock().unwrap();
        conexion
            .query_one("SELECT count(*) FROM uso", [], |f| f.get::<_, i64>(0))
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// Sustituye el coste estimado por el que factura OpenRouter, cuando la
    /// reconciliación lo consigue.
    pub fn reconcilia(&self, id: &str, coste: f64, proveedor: Option<String>) {
        let conexion = self.conexion.lock().unwrap();
        let hecho = conexion.execute(
            "UPDATE uso SET coste = ?1, coste_origen = 'openrouter',
                    proveedor = coalesce(proveedor, ?2)
             WHERE id = ?3",
            params![coste, proveedor, id],
        );
        if let Err(e) = hecho {
            eprintln!("no se pudo reconciliar el uso {id}: {e}");
        }
    }

    /// Las llamadas de un intervalo, de la más antigua a la más reciente, que es
    /// el orden natural para exportar.
    pub fn intervalo(
        &self,
        desde: Option<&str>,
        hasta: Option<&str>,
        app: Option<&str>,
    ) -> Vec<Registro> {
        self.consulta(
            "SELECT * FROM uso
             WHERE (?1 IS NULL OR fecha >= ?1) AND (?2 IS NULL OR fecha <= ?2)
               AND (?3 IS NULL OR app_id = ?3)
             ORDER BY fecha ASC, rowid ASC",
            params![desde, hasta, app],
        )
    }

    /// Totales por día o por modelo. Devuelve filas ya listas para el JSON.
    pub fn resumen(
        &self,
        desde: Option<&str>,
        hasta: Option<&str>,
        agrupar: &Agrupacion,
        app: Option<&str>,
    ) -> Vec<Value> {
        let consulta = format!(
            "SELECT {columna} AS grupo,
                    count(*)                AS llamadas,
                    sum(estado >= 400)      AS fallos,
                    sum(tokens_entrada)     AS tokens_entrada,
                    sum(tokens_salida)      AS tokens_salida,
                    sum(coalesce(coste, 0)) AS coste,
                    avg(latencia_ms)        AS latencia_media_ms
             FROM uso
             WHERE (?1 IS NULL OR fecha >= ?1) AND (?2 IS NULL OR fecha <= ?2)
               AND (?3 IS NULL OR app_id = ?3)
             GROUP BY grupo
             ORDER BY grupo DESC",
            columna = agrupar.columna()
        );

        let conexion = self.conexion.lock().unwrap();
        let Ok(mut sentencia) = conexion.prepare(&consulta) else {
            return Vec::new();
        };
        let filas = sentencia.query_map(params![desde, hasta, app], |f| {
            Ok(serde_json::json!({
                "grupo": f.get::<_, String>("grupo")?,
                "llamadas": f.get::<_, i64>("llamadas")?,
                "fallos": f.get::<_, i64>("fallos")?,
                "tokens_entrada": f.get::<_, i64>("tokens_entrada")?,
                "tokens_salida": f.get::<_, i64>("tokens_salida")?,
                "coste": f.get::<_, f64>("coste")?,
                "latencia_media_ms": f.get::<_, f64>("latencia_media_ms")?.round() as i64,
            }))
        });

        match filas {
            Ok(filas) => filas.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn consulta(&self, sql: &str, argumentos: impl rusqlite::Params) -> Vec<Registro> {
        let conexion = self.conexion.lock().unwrap();
        let Ok(mut sentencia) = conexion.prepare(sql) else {
            return Vec::new();
        };
        let filas = match sentencia.query_map(argumentos, Registro::desde_fila) {
            Ok(filas) => filas.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        };
        filas
    }
}

/// La fecha de ahora en ISO, que usan tanto el registro como el alta de
/// aplicaciones.
pub fn ahora_iso() -> String {
    let segundos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso(segundos)
}

/// Deja la base lista: tablas, columnas que falten y, solo entonces, índices.
/// El orden importa, porque un índice sobre una columna recién añadida falla si
/// se intenta antes.
fn prepara(conexion: &Connection) -> rusqlite::Result<()> {
    conexion.execute_batch(ESQUEMA)?;
    conexion.execute_batch(apps::ESQUEMA)?;
    for (columna, tipo) in [("app_id", "TEXT"), ("operacion", "TEXT")] {
        asegura_columna(conexion, columna, tipo)?;
    }
    conexion.execute_batch(INDICES)
}

/// Añade una columna si la base viene de una versión anterior. SQLite no tiene
/// `ADD COLUMN IF NOT EXISTS`, así que se mira primero qué columnas hay.
fn asegura_columna(conexion: &Connection, columna: &str, tipo: &str) -> rusqlite::Result<()> {
    let existe = conexion
        .prepare("SELECT 1 FROM pragma_table_info('uso') WHERE name = ?1")?
        .query_one(params![columna], |_| Ok(()))
        .optional()?
        .is_some();

    if !existe {
        conexion.execute(&format!("ALTER TABLE uso ADD COLUMN {columna} {tipo}"), [])?;
    }
    Ok(())
}

/// Fecha ISO 8601 en UTC desde segundos Unix, sin dependencias: el algoritmo de
/// días civiles de Howard Hinnant. Solo se usa para enseñar la fecha.
fn iso(segundos: u64) -> String {
    let dias = (segundos / 86_400) as i64;
    let resto = segundos % 86_400;
    let (h, m, s) = (resto / 3600, (resto % 3600) / 60, resto % 60);

    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let anio = if mes <= 2 { y + 1 } else { y };

    format!("{anio:04}-{mes:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn en_memoria() -> Uso {
        // Una ruta que no existe fuerza el camino de memoria, que es justo lo
        // que hace el servicio cuando Fly no monta el volumen.
        Uso::nuevo(Some("/no/existe/uso.db"))
    }

    #[test]
    fn la_fecha_sale_en_iso() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_757_673_600), "2025-09-12T10:40:00Z");
        // Un 29 de febrero y un fin de año, que es donde falla el cálculo civil.
        assert_eq!(iso(1_709_208_000), "2024-02-29T12:00:00Z");
        assert_eq!(iso(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn sin_volumen_el_servicio_sigue_en_memoria() {
        let uso = en_memoria();
        assert_eq!(uso.almacen(), "memoria");
        let r = uso.abre("m".into());
        uso.anota(r);
        assert_eq!(uso.total(), 1);
    }

    #[test]
    fn el_coste_de_openrouter_le_gana_a_la_estimacion() {
        let uso = en_memoria();
        let mut r = uso.abre("m".into());
        r.desde_respuesta(&serde_json::json!({
            "id": "gen-1", "model": "m",
            "usage": { "prompt_tokens": 100, "completion_tokens": 50, "cost": 0.25 }
        }));
        r.estima(Some((1.0, 2.0)));
        assert_eq!(r.coste, Some(0.25));
        assert_eq!(r.coste_origen, "openrouter");
    }

    #[test]
    fn sin_coste_de_openrouter_se_estima_con_el_catalogo() {
        let uso = en_memoria();
        let mut r = uso.abre("m".into());
        r.desde_respuesta(&serde_json::json!({
            "usage": { "prompt_tokens": 1_000_000, "completion_tokens": 500_000 }
        }));
        r.estima(Some((2.0, 4.0)));
        assert_eq!(r.coste, Some(4.0));
        assert_eq!(r.coste_origen, "estimado");
    }

    #[test]
    fn el_historico_sobrevive_a_reabrir_el_fichero() {
        let ruta = std::env::temp_dir().join(format!("uso-{}.db", std::process::id()));
        let ruta = ruta.to_str().unwrap();
        {
            let uso = Uso::nuevo(Some(ruta));
            assert_eq!(uso.almacen(), "sqlite");
            let mut r = uso.abre("modelo/uno".into());
            r.estado = 200;
            r.coste = Some(0.5);
            uso.anota(r);
        }
        let uso = Uso::nuevo(Some(ruta));
        assert_eq!(uso.total(), 1, "el registro tiene que seguir ahí");
        let _ = std::fs::remove_file(ruta);
    }

    #[test]
    fn el_resumen_agrupa_y_suma() {
        let uso = en_memoria();
        for (modelo, coste, estado) in [("a", 1.0, 200), ("a", 2.0, 500), ("b", 4.0, 200)] {
            let mut r = uso.abre(modelo.into());
            r.modelo_servido = Some(modelo.into());
            r.coste = Some(coste);
            r.estado = estado;
            r.tokens_entrada = 10;
            uso.anota(r);
        }

        let por_modelo = uso.resumen(None, None, &Agrupacion::Modelo, None);
        assert_eq!(por_modelo.len(), 2);
        let a = por_modelo.iter().find(|f| f["grupo"] == "a").unwrap();
        assert_eq!(a["llamadas"], 2);
        assert_eq!(a["fallos"], 1);
        assert_eq!(a["coste"], 3.0);
        assert_eq!(a["tokens_entrada"], 20);

        // Todas son de hoy, así que por día sale un solo grupo con las tres.
        let por_dia = uso.resumen(None, None, &Agrupacion::Dia, None);
        assert_eq!(por_dia.len(), 1);
        assert_eq!(por_dia[0]["llamadas"], 3);
        assert_eq!(por_dia[0]["coste"], 7.0);
    }

    #[test]
    fn el_dia_completo_entra_en_el_intervalo() {
        let uso = en_memoria();
        let mut r = uso.abre("m".into());
        r.fecha = "2026-09-12T14:48:00Z".into();
        uso.anota(r);
        // Lo que manda la ruta tras normalizar un "hasta" de solo fecha.
        assert_eq!(uso.intervalo(None, Some("2026-09-12T23:59:59Z"), None).len(), 1);
        // Sin normalizar, la fecha suelta dejaria el dia fuera.
        assert_eq!(uso.intervalo(None, Some("2026-09-12"), None).len(), 0);
    }

    /// El esquema tal como quedaba en la etapa 4, sin aplicaciones.
    const ESQUEMA_ANTERIOR: &str = "
    CREATE TABLE uso (
        id TEXT PRIMARY KEY, fecha TEXT NOT NULL, modelo_pedido TEXT NOT NULL,
        modelo_servido TEXT, proveedor TEXT,
        tokens_entrada INTEGER NOT NULL DEFAULT 0, tokens_salida INTEGER NOT NULL DEFAULT 0,
        tokens_razonamiento INTEGER NOT NULL DEFAULT 0, tokens_cache INTEGER NOT NULL DEFAULT 0,
        coste REAL, coste_origen TEXT NOT NULL, latencia_ms INTEGER NOT NULL DEFAULT 0,
        motivo_fin TEXT, estado INTEGER NOT NULL DEFAULT 0, id_openrouter TEXT);
    ";

    #[test]
    fn una_base_de_la_etapa_anterior_se_migra_sin_caerse() {
        let ruta = std::env::temp_dir().join(format!("uso-vieja-{}.db", std::process::id()));
        let ruta = ruta.to_str().unwrap();
        let _ = std::fs::remove_file(ruta);

        {
            let vieja = Connection::open(ruta).unwrap();
            vieja.execute_batch(ESQUEMA_ANTERIOR).unwrap();
            vieja
                .execute(
                    "INSERT INTO uso (id, fecha, modelo_pedido, coste_origen) VALUES ('u-1','2026-09-12T10:00:00Z','m','estimado')",
                    [],
                )
                .unwrap();
        }

        // Antes de la etapa 5 esto reventaba al crear el indice de app_id.
        let uso = Uso::nuevo(Some(ruta));
        assert_eq!(uso.almacen(), "sqlite", "tiene que seguir en disco, no caer a memoria");
        assert_eq!(uso.total(), 1, "el historico anterior se conserva");

        let mut r = uso.abre("m".into());
        r.app_id = Some("app_uno".into());
        uso.anota(r);
        assert_eq!(uso.ultimos(50, Some("app_uno")).len(), 1);
        let _ = std::fs::remove_file(ruta);
    }

    #[test]
    fn cada_aplicacion_solo_ve_su_gasto() {
        let uso = en_memoria();
        for (app, coste) in [(Some("app_uno"), 1.0), (Some("app_uno"), 2.0), (Some("app_dos"), 8.0), (None, 4.0)] {
            let mut r = uso.abre("m".into());
            r.app_id = app.map(str::to_string);
            r.coste = Some(coste);
            r.estado = 200;
            uso.anota(r);
        }

        assert_eq!(uso.ultimos(50, Some("app_uno")).len(), 2);
        assert_eq!(uso.ultimos(50, Some("app_dos")).len(), 1);
        assert_eq!(uso.ultimos(50, None).len(), 4, "sin filtro se ve todo");

        let de_una = uso.resumen(None, None, &Agrupacion::Dia, Some("app_uno"));
        assert_eq!(de_una[0]["coste"], 3.0);

        // Agrupado por aplicacion, la de administracion sale con su etiqueta.
        let por_app = uso.resumen(None, None, &Agrupacion::App, None);
        assert_eq!(por_app.len(), 3);
        assert!(por_app.iter().any(|f| f["grupo"] == "administracion"));
    }

    #[test]
    fn el_intervalo_filtra_por_fecha() {
        let uso = en_memoria();
        for fecha in ["2026-01-01T10:00:00Z", "2026-06-01T10:00:00Z"] {
            let mut r = uso.abre("m".into());
            r.fecha = fecha.into();
            uso.anota(r);
        }
        assert_eq!(uso.intervalo(Some("2026-03-01"), None, None).len(), 1);
        assert_eq!(uso.intervalo(None, Some("2026-03-01"), None).len(), 1);
        assert_eq!(uso.intervalo(None, None, None).len(), 2);
    }
}
