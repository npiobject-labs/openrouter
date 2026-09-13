use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::{
    apps::{App, Identidad},
    error::ErrorApi,
    uso::{ahora_iso, hace_iso},
    Servicio,
};

/// Cuánto se mira atrás para contar repeticiones, y cuántas se toleran. Un
/// bucle manda la misma consulta una y otra vez; un uso intenso manda consultas
/// distintas, así que la huella del cuerpo separa un caso del otro.
const VENTANA_BUCLE: u64 = 60;
const REPETICIONES_MAXIMAS: i64 = 5;

const VENTANA_CUOTA: u64 = 60;

/// Un importe legible. Con dos decimales un presupuesto de céntésimas de
/// céntimo se leería como `0.00 $`, que es peor que no decir nada.
fn dinero(valor: f64) -> String {
    if valor.abs() >= 0.01 {
        format!("{valor:.2} $")
    } else {
        format!("{valor:.6} $")
    }
}

/// Lo que se sabe del margen de una aplicación.
pub struct Margen {
    pub consumido: f64,
    pub restante: Option<f64>,
    pub aviso: bool,
}

/// Deja pasar la llamada o la corta.
///
/// La clave de administración no tiene límites: es la del dueño del servicio, y
/// ponerle tope sería encerrarse fuera de casa.
///
/// El orden importa. Primero el bucle y la cuota, que protegen del gasto
/// accidental y son baratos de comprobar; después el presupuesto, que protege
/// del gasto legítimo pero excesivo.
pub fn comprueba(
    servicio: &Servicio,
    quien: &Identidad,
    huella: &str,
) -> Result<Option<String>, ErrorApi> {
    let Some(app) = &quien.app else {
        return Ok(None);
    };

    let repetidas = servicio
        .uso
        .repeticiones(Some(&app.id), huella, &hace_iso(VENTANA_BUCLE));
    if repetidas >= REPETICIONES_MAXIMAS {
        return Err(ErrorApi::nuevo(
            StatusCode::TOO_MANY_REQUESTS,
            "bucle",
            format!(
                "La misma consulta se ha repetido {repetidas} veces en {VENTANA_BUCLE} segundos. \
                 Parece un bucle, así que se corta para no gastar crédito."
            ),
        ));
    }

    if let Some(cuota) = app.limites.cuota_minuto {
        let recientes = servicio
            .uso
            .llamadas_desde(Some(&app.id), &hace_iso(VENTANA_CUOTA));
        if recientes >= cuota {
            return Err(ErrorApi::nuevo(
                StatusCode::TOO_MANY_REQUESTS,
                "cuota_superada",
                format!("Esta aplicación tiene una cuota de {cuota} llamadas por minuto."),
            ));
        }
    }

    let margen = margen_de(servicio, app);
    if let (Some(restante), Some(limite)) = (margen.restante, app.limites.limite) {
        if restante <= 0.0 {
            return Err(ErrorApi::nuevo(
                StatusCode::PAYMENT_REQUIRED,
                "presupuesto_agotado",
                format!(
                    "Esta aplicación lleva {} gastados y su presupuesto por {} es de {}.",
                    dinero(margen.consumido),
                    app.limites.periodo.as_deref().unwrap_or("periodo"),
                    dinero(limite)
                ),
            ));
        }
    }

    Ok(if margen.aviso {
        Some(format!(
            "{} gastados de {}",
            dinero(margen.consumido),
            dinero(app.limites.limite.unwrap_or_default())
        ))
    } else {
        None
    })
}

/// Lo gastado en el periodo en curso y lo que queda.
pub fn margen_de(servicio: &Servicio, app: &App) -> Margen {
    let Some(desde) = app.limites.desde(&ahora_iso()) else {
        // Sin periodo no hay presupuesto que vigilar.
        return Margen { consumido: 0.0, restante: None, aviso: false };
    };

    let consumido = servicio.uso.gasto_desde(Some(&app.id), &desde);
    Margen {
        restante: app.limites.limite.map(|l| l - consumido),
        aviso: app.limites.aviso.map(|a| consumido >= a).unwrap_or(false),
        consumido,
    }
}

/// El margen tal como se devuelve por la API.
pub fn como_json(app: &App, margen: &Margen) -> Value {
    json!({
        "app_id": app.id,
        "nombre": app.nombre,
        "periodo": app.limites.periodo,
        "desde": app.limites.desde(&ahora_iso()),
        "limite": app.limites.limite,
        "aviso": app.limites.aviso,
        "cuota_minuto": app.limites.cuota_minuto,
        "consumido": margen.consumido,
        "restante": margen.restante,
        "en_aviso": margen.aviso,
    })
}
