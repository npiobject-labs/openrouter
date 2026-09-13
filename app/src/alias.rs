use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{Map, Value};

/// Nombres que puede tomar una aplicación en vez de un modelo concreto.
///
/// La idea es que una aplicación pida `"model": "alias:redactor"` y sea el
/// servicio quien decida qué modelo hay detrás, con qué parámetros y a qué
/// recurrir si falla. Cambiar de modelo deja así de tocar el código de nadie.
pub const ESQUEMA: &str = "
CREATE TABLE IF NOT EXISTS alias (
    nombre      TEXT PRIMARY KEY,
    modelo      TEXT NOT NULL,
    respaldos   TEXT NOT NULL DEFAULT '[]',
    parametros  TEXT NOT NULL DEFAULT '{}',
    nota        TEXT,
    actualizado TEXT NOT NULL
);
";

/// El prefijo con el que una consulta pide un alias en vez de un modelo.
pub const PREFIJO: &str = "alias:";

/// Parámetros que un alias nunca puede fijar, porque no son del modelo sino de
/// la llamada: dejarlos sobrescribir el cuerpo rompería la consulta o la
/// medición.
pub const RESERVADOS: [&str; 4] = ["model", "messages", "stream", "stream_options"];

#[derive(Clone, Serialize)]
pub struct Alias {
    pub nombre: String,
    /// El modelo que se usa primero.
    pub modelo: String,
    /// A qué recurrir, en orden, si el anterior no responde.
    pub respaldos: Vec<String>,
    /// Valores por defecto del cuerpo. Lo que mande la aplicación gana.
    pub parametros: Map<String, Value>,
    pub nota: Option<String>,
    pub actualizado: String,
}

impl Alias {
    /// El modelo y sus respaldos, en el orden en que se intentan.
    pub fn cadena(&self) -> Vec<String> {
        let mut cadena = vec![self.modelo.clone()];
        cadena.extend(self.respaldos.iter().cloned());
        cadena
    }

    /// Mete los parámetros del alias en el cuerpo **sin pisar** lo que ya trae:
    /// el alias pone el valor por defecto, quien llama tiene la última palabra.
    pub fn aplica(&self, cuerpo: &mut Map<String, Value>) {
        for (clave, valor) in &self.parametros {
            cuerpo.entry(clave.clone()).or_insert_with(|| valor.clone());
        }
    }
}

/// Un nombre de alias es minúsculas, dígitos y guiones, y nada más: viaja
/// dentro del campo `model` y tiene que poder leerse sin ambigüedad.
pub fn nombre_valido(nombre: &str) -> bool {
    !nombre.is_empty()
        && nombre.len() <= 40
        && nombre
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Si la consulta pide un alias, su nombre.
pub fn pedido(modelo: &str) -> Option<&str> {
    modelo.strip_prefix(PREFIJO)
}

fn desde_fila(f: &rusqlite::Row) -> rusqlite::Result<Alias> {
    let respaldos: String = f.get("respaldos")?;
    let parametros: String = f.get("parametros")?;
    Ok(Alias {
        nombre: f.get("nombre")?,
        modelo: f.get("modelo")?,
        // Lo guardado lo escribió este mismo módulo; si aun así no se puede
        // leer, un alias sin respaldos sirve mejor que una consulta caída.
        respaldos: serde_json::from_str(&respaldos).unwrap_or_default(),
        parametros: serde_json::from_str(&parametros).unwrap_or_default(),
        nota: f.get("nota")?,
        actualizado: f.get("actualizado")?,
    })
}

pub fn una(conexion: &Connection, nombre: &str) -> Option<Alias> {
    conexion
        .query_one(
            "SELECT * FROM alias WHERE nombre = ?1",
            params![nombre],
            desde_fila,
        )
        .optional()
        .unwrap_or(None)
}

pub fn lista(conexion: &Connection) -> Vec<Alias> {
    let Ok(mut sentencia) = conexion.prepare("SELECT * FROM alias ORDER BY nombre") else {
        return Vec::new();
    };
    let Ok(filas) = sentencia.query_map([], desde_fila) else {
        return Vec::new();
    };
    filas.filter_map(Result::ok).collect()
}

/// Crea o reemplaza un alias entero. No hay edición por campos: un alias es
/// corto y se entiende mejor mandándolo completo que parcheándolo a trozos.
pub fn guarda(conexion: &Connection, a: &Alias) -> rusqlite::Result<()> {
    let respaldos = serde_json::to_string(&a.respaldos).unwrap_or_else(|_| "[]".into());
    let parametros = serde_json::to_string(&a.parametros).unwrap_or_else(|_| "{}".into());
    conexion.execute(
        "INSERT INTO alias (nombre, modelo, respaldos, parametros, nota, actualizado)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(nombre) DO UPDATE SET
           modelo = ?2, respaldos = ?3, parametros = ?4, nota = ?5, actualizado = ?6",
        params![a.nombre, a.modelo, respaldos, parametros, a.nota, a.actualizado],
    )?;
    Ok(())
}

/// Borra un alias. Aquí sí se borra de verdad, al revés que con una aplicación:
/// el histórico guarda el nombre del alias como texto, así que un informe viejo
/// sigue diciendo por dónde salió la llamada aunque el alias ya no exista.
pub fn borra(conexion: &Connection, nombre: &str) -> rusqlite::Result<bool> {
    let filas = conexion.execute("DELETE FROM alias WHERE nombre = ?1", params![nombre])?;
    Ok(filas > 0)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    fn base() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(ESQUEMA).unwrap();
        c
    }

    fn ejemplo() -> Alias {
        Alias {
            nombre: "redactor".into(),
            modelo: "caro/bueno".into(),
            respaldos: vec!["barato/apanado".into()],
            parametros: json!({ "temperature": 0.2, "max_tokens": 500 })
                .as_object()
                .unwrap()
                .clone(),
            nota: Some("Textos de cliente".into()),
            actualizado: "2026-09-13T00:00:00Z".into(),
        }
    }

    #[test]
    fn un_alias_se_guarda_y_se_recupera_entero() {
        let c = base();
        guarda(&c, &ejemplo()).unwrap();

        let a = una(&c, "redactor").unwrap();
        assert_eq!(a.modelo, "caro/bueno");
        assert_eq!(a.respaldos, vec!["barato/apanado".to_string()]);
        assert_eq!(a.parametros.get("temperature"), Some(&json!(0.2)));
        assert_eq!(a.cadena(), vec!["caro/bueno", "barato/apanado"]);
    }

    #[test]
    fn guardar_dos_veces_reemplaza_en_vez_de_duplicar() {
        let c = base();
        guarda(&c, &ejemplo()).unwrap();
        let mut otro = ejemplo();
        otro.modelo = "otro/modelo".into();
        otro.respaldos.clear();
        guarda(&c, &otro).unwrap();

        assert_eq!(lista(&c).len(), 1);
        assert_eq!(una(&c, "redactor").unwrap().modelo, "otro/modelo");
        assert!(una(&c, "redactor").unwrap().respaldos.is_empty());
    }

    #[test]
    fn lo_que_manda_la_aplicacion_le_gana_al_alias() {
        let a = ejemplo();
        let mut cuerpo = json!({ "temperature": 0.9 }).as_object().unwrap().clone();
        a.aplica(&mut cuerpo);

        // El valor que traía la consulta se respeta; el que faltaba, se rellena.
        assert_eq!(cuerpo.get("temperature"), Some(&json!(0.9)));
        assert_eq!(cuerpo.get("max_tokens"), Some(&json!(500)));
    }

    #[test]
    fn solo_se_reconoce_un_alias_con_su_prefijo() {
        assert_eq!(pedido("alias:redactor"), Some("redactor"));
        assert_eq!(pedido("google/gemini-2.5-flash-lite"), None);
        assert!(nombre_valido("redactor-2"));
        assert!(!nombre_valido("Redactor"));
        assert!(!nombre_valido("alias:redactor"));
        assert!(!nombre_valido(""));
    }

    #[test]
    fn borrar_un_alias_lo_quita_de_la_lista() {
        let c = base();
        guarda(&c, &ejemplo()).unwrap();
        assert!(borra(&c, "redactor").unwrap());
        assert!(!borra(&c, "redactor").unwrap());
        assert!(lista(&c).is_empty());
    }
}
