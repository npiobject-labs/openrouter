use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;

/// Cuántas llamadas se recuerdan. En memoria: al reiniciar la máquina de Fly se
/// pierden. La persistencia llega en la etapa 4.
const CAPACIDAD: usize = 1000;

/// Lo que se sabe de una llamada a un modelo. Se anota siempre, también cuando
/// falla: un error que costó dos segundos de espera es información de uso.
#[derive(Clone, Serialize)]
pub struct Registro {
    pub id: String,
    pub fecha: String,
    pub modelo_pedido: String,
    pub modelo_servido: Option<String>,
    pub proveedor: Option<String>,
    pub tokens_entrada: u64,
    pub tokens_salida: u64,
    pub tokens_razonamiento: u64,
    pub tokens_cache: u64,
    /// Dólares. `None` mientras no se pueda ni estimar.
    pub coste: Option<f64>,
    /// `estimado` con los precios del catálogo, `openrouter` si ya se reconcilió.
    pub coste_origen: &'static str,
    pub latencia_ms: u64,
    pub motivo_fin: Option<String>,
    pub estado: u16,
    /// El id de la generación en OpenRouter, con el que se reconcilia el coste.
    pub id_openrouter: Option<String>,
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
                self.coste_origen = "openrouter";
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
            self.coste_origen = "estimado";
        }
    }
}

fn texto(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn entero(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

/// El anillo de registros y el contador que da los identificadores.
#[derive(Default)]
pub struct Uso {
    anillo: RwLock<VecDeque<Registro>>,
    contador: AtomicU64,
}

impl Uso {
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
            coste_origen: "desconocido",
            latencia_ms: 0,
            motivo_fin: None,
            estado: 0,
            id_openrouter: None,
        }
    }

    pub fn anota(&self, registro: Registro) {
        let mut anillo = self.anillo.write().unwrap();
        if anillo.len() == CAPACIDAD {
            anillo.pop_front();
        }
        anillo.push_back(registro);
    }

    /// Los últimos `n`, del más reciente al más antiguo.
    pub fn ultimos(&self, n: usize) -> Vec<Registro> {
        let anillo = self.anillo.read().unwrap();
        anillo.iter().rev().take(n).cloned().collect()
    }

    pub fn uno(&self, id: &str) -> Option<Registro> {
        let anillo = self.anillo.read().unwrap();
        anillo.iter().rev().find(|r| r.id == id).cloned()
    }

    pub fn total(&self) -> usize {
        self.anillo.read().unwrap().len()
    }

    /// Sustituye el coste estimado por el que factura OpenRouter, cuando la
    /// reconciliación lo consigue. Si el registro ya salió del anillo, se
    /// descarta sin ruido: es un dato de conveniencia, no una transacción.
    pub fn reconcilia(&self, id: &str, coste: f64, proveedor: Option<String>) {
        let mut anillo = self.anillo.write().unwrap();
        if let Some(r) = anillo.iter_mut().rev().find(|r| r.id == id) {
            r.coste = Some(coste);
            r.coste_origen = "openrouter";
            if r.proveedor.is_none() {
                r.proveedor = proveedor;
            }
        }
    }
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

    #[test]
    fn la_fecha_sale_en_iso() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_757_673_600), "2025-09-12T10:40:00Z");
        // Un 29 de febrero y un fin de año, que es donde falla el cálculo civil.
        assert_eq!(iso(1_709_208_000), "2024-02-29T12:00:00Z");
        assert_eq!(iso(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn el_anillo_no_pasa_de_su_capacidad() {
        let uso = Uso::default();
        for _ in 0..CAPACIDAD + 10 {
            let r = uso.abre("m".into());
            uso.anota(r);
        }
        assert_eq!(uso.total(), CAPACIDAD);
    }

    #[test]
    fn el_coste_de_openrouter_le_gana_a_la_estimacion() {
        let uso = Uso::default();
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
        let uso = Uso::default();
        let mut r = uso.abre("m".into());
        r.desde_respuesta(&serde_json::json!({
            "usage": { "prompt_tokens": 1_000_000, "completion_tokens": 500_000 }
        }));
        r.estima(Some((2.0, 4.0)));
        assert_eq!(r.coste, Some(4.0));
        assert_eq!(r.coste_origen, "estimado");
    }
}
