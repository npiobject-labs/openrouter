use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Las aplicaciones que pueden llamar al servicio. Cada una tiene su clave, y
/// del lado del servicio solo se guarda el hash: si esta base se filtra, no se
/// puede llamar con lo que hay dentro.
pub const ESQUEMA: &str = "
CREATE TABLE IF NOT EXISTS apps (
    id     TEXT PRIMARY KEY,
    nombre TEXT    NOT NULL,
    hash   TEXT    NOT NULL UNIQUE,
    activa INTEGER NOT NULL DEFAULT 1,
    creada TEXT    NOT NULL,
    periodo       TEXT,
    limite        REAL,
    aviso         REAL,
    cuota_minuto  INTEGER
);
";

/// Columnas que pueden faltar en una base anterior a la etapa 6.
pub const COLUMNAS: [(&str, &str); 4] = [
    ("periodo", "TEXT"),
    ("limite", "REAL"),
    ("aviso", "REAL"),
    ("cuota_minuto", "INTEGER"),
];

/// Una aplicación tal como se enseña. Nunca lleva la clave: esa se ve una sola
/// vez, al crearla.
#[derive(Clone, Serialize)]
pub struct App {
    pub id: String,
    pub nombre: String,
    pub activa: bool,
    pub creada: String,
    /// Los topes de gasto y ritmo. Sin ellos, la aplicación no tiene límite.
    #[serde(flatten)]
    pub limites: Limites,
}

/// Cuánto puede gastar una aplicación y a qué ritmo. Todo opcional: lo que no
/// se fija, no se limita.
#[derive(Clone, Default, Serialize)]
pub struct Limites {
    /// `dia` o `mes`. Sin periodo, el presupuesto no se aplica.
    pub periodo: Option<String>,
    /// Dólares del periodo a partir de los cuales se corta.
    pub limite: Option<f64>,
    /// Dólares a partir de los cuales se avisa, sin cortar.
    pub aviso: Option<f64>,
    /// Peticiones por minuto.
    pub cuota_minuto: Option<i64>,
}

impl Limites {
    /// El prefijo de fecha que delimita el periodo en curso: el día o el mes de
    /// hoy. Las fechas del histórico son ISO, así que comparar por prefijo basta.
    pub fn desde(&self, ahora: &str) -> Option<String> {
        match self.periodo.as_deref() {
            Some("dia") => Some(ahora[..10].to_string()),
            Some("mes") => Some(ahora[..7].to_string()),
            _ => None,
        }
    }
}

/// Quién está llamando. La clave de administración es la del despliegue
/// (`SERVICIO_CLAVE`); las demás son de aplicaciones dadas de alta.
#[derive(Clone)]
pub struct Identidad {
    pub app: Option<App>,
    pub admin: bool,
}

impl Identidad {
    pub fn administracion() -> Self {
        Self { app: None, admin: true }
    }

    /// El identificador que se anota en el registro de uso.
    pub fn app_id(&self) -> Option<String> {
        self.app.as_ref().map(|a| a.id.clone())
    }

    pub fn nombre(&self) -> String {
        match &self.app {
            Some(a) => a.nombre.clone(),
            None => "administracion".to_string(),
        }
    }
}

/// El hash con el que se compara una clave. Hex en minúsculas, para que el
/// valor guardado se pueda mirar sin sorpresas de codificación.
pub fn hash(clave: &str) -> String {
    let resumen = Sha256::digest(clave.as_bytes());
    resumen.iter().map(|b| format!("{b:02x}")).collect()
}

fn desde_fila(f: &rusqlite::Row) -> rusqlite::Result<App> {
    Ok(App {
        id: f.get("id")?,
        nombre: f.get("nombre")?,
        activa: f.get::<_, i64>("activa")? != 0,
        creada: f.get("creada")?,
        limites: Limites {
            periodo: f.get("periodo")?,
            limite: f.get("limite")?,
            aviso: f.get("aviso")?,
            cuota_minuto: f.get("cuota_minuto")?,
        },
    })
}

/// Fija los límites de una aplicación. Un valor a `None` lo quita.
pub fn limita(conexion: &Connection, id: &str, l: &Limites) -> rusqlite::Result<bool> {
    let filas = conexion.execute(
        "UPDATE apps SET periodo = ?1, limite = ?2, aviso = ?3, cuota_minuto = ?4 WHERE id = ?5",
        params![l.periodo, l.limite, l.aviso, l.cuota_minuto, id],
    )?;
    Ok(filas > 0)
}

pub fn una(conexion: &Connection, id: &str) -> Option<App> {
    conexion
        .query_one("SELECT * FROM apps WHERE id = ?1", params![id], desde_fila)
        .optional()
        .unwrap_or(None)
}

/// Da de alta una aplicación y devuelve su clave en claro. Es la única vez que
/// existe fuera de quien la pide: el servicio solo se queda el hash.
pub fn crea(conexion: &Connection, nombre: &str, fecha: &str) -> rusqlite::Result<(App, String)> {
    // randomblob viene del propio SQLite, que lo siembra con el generador del
    // sistema; asi no hace falta una dependencia mas solo para esto.
    // [SUPUESTO] entropia suficiente para una clave de servicio. Plan B si no:
    // getrandom y 32 bytes del sistema.
    let sufijo: String = conexion.query_one("SELECT lower(hex(randomblob(24)))", [], |f| f.get(0))?;
    let id: String = conexion.query_one("SELECT lower(hex(randomblob(5)))", [], |f| f.get(0))?;
    let id = format!("app_{id}");
    let clave = format!("svc_{sufijo}");

    conexion.execute(
        "INSERT INTO apps (id, nombre, hash, activa, creada) VALUES (?1, ?2, ?3, 1, ?4)",
        params![id, nombre, hash(&clave), fecha],
    )?;

    Ok((
        App {
            id,
            nombre: nombre.to_string(),
            activa: true,
            creada: fecha.to_string(),
            limites: Limites::default(),
        },
        clave,
    ))
}

/// La aplicación a la que pertenece una clave, si está activa.
pub fn por_clave(conexion: &Connection, clave: &str) -> Option<App> {
    conexion
        .query_one(
            "SELECT * FROM apps WHERE hash = ?1 AND activa = 1",
            params![hash(clave)],
            desde_fila,
        )
        .optional()
        .unwrap_or(None)
}

pub fn lista(conexion: &Connection) -> Vec<App> {
    let Ok(mut sentencia) = conexion.prepare("SELECT * FROM apps ORDER BY creada DESC") else {
        return Vec::new();
    };
    let filas = match sentencia.query_map([], desde_fila) {
        Ok(filas) => filas.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    };
    filas
}

/// Desactiva una aplicación. No se borra: el histórico de uso la sigue
/// referenciando y un informe de hace un mes tiene que poder decir quién gastó.
pub fn desactiva(conexion: &Connection, id: &str) -> rusqlite::Result<bool> {
    let filas = conexion.execute("UPDATE apps SET activa = 0 WHERE id = ?1", params![id])?;
    Ok(filas > 0)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn bd() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(ESQUEMA).unwrap();
        c
    }

    #[test]
    fn la_clave_no_se_guarda_en_claro() {
        let c = bd();
        let (app, clave) = crea(&c, "presupuestos", "2026-09-13T00:00:00Z").unwrap();

        let guardado: String = c
            .query_one("SELECT hash FROM apps WHERE id = ?1", params![app.id], |f| f.get(0))
            .unwrap();
        assert_ne!(guardado, clave);
        assert_eq!(guardado, hash(&clave));
        assert!(clave.starts_with("svc_"));
    }

    #[test]
    fn una_clave_activa_identifica_a_su_aplicacion() {
        let c = bd();
        let (app, clave) = crea(&c, "presupuestos", "2026-09-13T00:00:00Z").unwrap();
        assert_eq!(por_clave(&c, &clave).unwrap().id, app.id);
        assert!(por_clave(&c, "svc_inventada").is_none());
    }

    #[test]
    fn desactivar_deja_la_clave_sin_valor_pero_conserva_la_aplicacion() {
        let c = bd();
        let (app, clave) = crea(&c, "presupuestos", "2026-09-13T00:00:00Z").unwrap();
        assert!(desactiva(&c, &app.id).unwrap());

        assert!(por_clave(&c, &clave).is_none(), "la clave ya no debe servir");
        let todas = lista(&c);
        assert_eq!(todas.len(), 1, "la aplicacion sigue ahi para el historico");
        assert!(!todas[0].activa);
        assert!(!desactiva(&c, "app_inventada").unwrap());
    }

    #[test]
    fn los_limites_se_guardan_y_definen_el_periodo() {
        let c = bd();
        let (app, _) = crea(&c, "presupuestos", "2026-09-13T00:00:00Z").unwrap();
        assert!(una(&c, &app.id).unwrap().limites.limite.is_none(), "nace sin topes");

        let limites = Limites {
            periodo: Some("mes".into()),
            limite: Some(5.0),
            aviso: Some(4.0),
            cuota_minuto: Some(30),
        };
        assert!(limita(&c, &app.id, &limites).unwrap());

        let guardada = una(&c, &app.id).unwrap();
        assert_eq!(guardada.limites.limite, Some(5.0));
        assert_eq!(guardada.limites.cuota_minuto, Some(30));
        // El periodo decide desde cuándo se cuenta el gasto.
        assert_eq!(guardada.limites.desde("2026-09-13T11:22:33Z").as_deref(), Some("2026-09"));

        let por_dia = Limites { periodo: Some("dia".into()), ..limites.clone() };
        assert_eq!(por_dia.desde("2026-09-13T11:22:33Z").as_deref(), Some("2026-09-13"));
        assert_eq!(Limites::default().desde("2026-09-13T11:22:33Z"), None);
    }

    #[test]
    fn dos_aplicaciones_no_comparten_clave() {
        let c = bd();
        let (_, una) = crea(&c, "una", "2026-09-13T00:00:00Z").unwrap();
        let (_, otra) = crea(&c, "otra", "2026-09-13T00:00:00Z").unwrap();
        assert_ne!(una, otra);
    }
}
