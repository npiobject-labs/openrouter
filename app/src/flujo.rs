use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::body::Bytes;
use serde_json::Value;

use crate::{uso::Registro, Servicio};

/// Mide un flujo de streaming mientras pasa hacia el cliente.
///
/// El registro se escribe en `Drop`, no al final del bucle, porque el final
/// puede no llegar nunca: si el cliente cierra la conexión a mitad, axum suelta
/// el cuerpo y esto es lo único que se ejecuta. Así una respuesta cancelada
/// queda anotada con lo que costó hasta ese momento, que es justo el caso que
/// de otro modo desaparecería del histórico.
pub struct Medidor {
    servicio: Arc<Servicio>,
    registro: Option<Registro>,
    reloj: Instant,
    /// Tiempo hasta el primer trozo con contenido, que es lo que percibe quien
    /// espera delante de una pantalla.
    primer_token: Option<Duration>,
    completo: bool,
    /// Lo que quedó a medias de una línea partida entre dos trozos de red.
    resto: String,
}

impl Medidor {
    pub fn nuevo(servicio: Arc<Servicio>, registro: Registro) -> Self {
        Self {
            servicio,
            registro: Some(registro),
            reloj: Instant::now(),
            primer_token: None,
            completo: false,
            resto: String::new(),
        }
    }

    /// Lee un trozo del flujo sin tocarlo: solo mira lo que lleva dentro.
    pub fn anota(&mut self, trozo: &Bytes) {
        let Ok(texto) = std::str::from_utf8(trozo) else {
            return;
        };
        self.resto.push_str(texto);

        // Los eventos vienen separados por saltos de línea; el último puede
        // estar partido, así que se guarda para el trozo siguiente.
        let Some(corte) = self.resto.rfind('\n') else {
            return;
        };
        let completas: String = self.resto.drain(..=corte).collect();

        for linea in completas.lines() {
            let Some(datos) = linea.strip_prefix("data:") else {
                continue;
            };
            let datos = datos.trim();
            if datos == "[DONE]" {
                self.completo = true;
                continue;
            }
            let Ok(evento) = serde_json::from_str::<Value>(datos) else {
                continue;
            };
            self.evento(&evento);
        }
    }

    fn evento(&mut self, evento: &Value) {
        let Some(registro) = self.registro.as_mut() else {
            return;
        };

        if self.primer_token.is_none() {
            let hay_contenido = evento
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                .map(|c| !c.is_empty())
                .unwrap_or(false);
            if hay_contenido {
                self.primer_token = Some(self.reloj.elapsed());
            }
        }

        // Cada evento aporta lo suyo y no borra lo anterior: el primero trae el
        // id y el modelo, el último el usage entero si se pidió include_usage.
        registro.desde_respuesta(evento);
    }
}

impl Drop for Medidor {
    fn drop(&mut self) {
        let Some(mut registro) = self.registro.take() else {
            return;
        };

        registro.latencia_ms = self.reloj.elapsed().as_millis() as i64;
        registro.primer_token_ms = self.primer_token.map(|d| d.as_millis() as i64);
        if !self.completo {
            // Nadie mandó [DONE]: o el cliente se fue, o el flujo se rompió.
            registro.motivo_fin = Some("cancelado".to_string());
        }
        if registro.motivo_fin.is_none() {
            registro.motivo_fin = Some("stop".to_string());
        }

        let servido = registro
            .modelo_servido
            .clone()
            .unwrap_or_else(|| registro.modelo_pedido.clone());
        registro.estima(self.servicio.catalogo.precio(&servido));

        let pendiente = registro.id_openrouter.clone();
        let id_uso = registro.id.clone();
        self.servicio.uso.anota(registro);

        if let Some(generacion) = pendiente {
            crate::rutas::chat::reconcilia(self.servicio.clone(), id_uso, generacion);
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::{catalogo::Catalogo, config::Config, openrouter::Cliente, uso::Uso};

    fn servicio() -> Arc<Servicio> {
        let config = Config::del_entorno();
        Arc::new(Servicio {
            openrouter: Cliente::nuevo(&config),
            catalogo: Catalogo::default(),
            // Una ruta imposible fuerza el histórico en memoria.
            uso: Uso::nuevo(Some("/no/existe/flujo.db")),
            config,
        })
    }

    fn medidor(servicio: &Arc<Servicio>) -> Medidor {
        Medidor::nuevo(
            servicio.clone(),
            servicio.uso.abre("prueba/modelo".to_string()),
        )
    }

    fn trozo(s: &str) -> Bytes {
        Bytes::from(s.to_string())
    }

    #[tokio::test]
    async fn un_evento_partido_entre_dos_trozos_se_lee_entero() {
        let servicio = servicio();
        {
            let mut m = medidor(&servicio);
            // El corte cae justo en mitad del JSON, que es lo que hace la red.
            m.anota(&trozo("data: {\"id\":\"gen-1\",\"choices\":[{\"delta\":{\"con"));
            m.anota(&trozo("tent\":\"hola\"}}]}\n\n"));
            m.anota(&trozo(
                "data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
            ));
            m.anota(&trozo("data: [DONE]\n\n"));
        }

        let r = &servicio.uso.ultimos(1, None)[0];
        assert_eq!(r.id_openrouter.as_deref(), Some("gen-1"));
        assert_eq!(r.tokens_entrada, 7);
        assert_eq!(r.tokens_salida, 3);
        assert_eq!(r.motivo_fin.as_deref(), Some("stop"));
        assert!(r.primer_token_ms.is_some(), "hubo contenido: hay primer token");
    }

    #[tokio::test]
    async fn un_flujo_sin_done_queda_anotado_como_cancelado() {
        let servicio = servicio();
        {
            let mut m = medidor(&servicio);
            m.anota(&trozo(
                "data: {\"id\":\"gen-2\",\"choices\":[{\"delta\":{\"content\":\"a med\"}}]}\n\n",
            ));
            // Y aquí el cliente cierra: nadie manda [DONE].
        }

        let r = &servicio.uso.ultimos(1, None)[0];
        assert_eq!(r.motivo_fin.as_deref(), Some("cancelado"));
        assert_eq!(r.id_openrouter.as_deref(), Some("gen-2"));
    }

    #[tokio::test]
    async fn un_evento_ilegible_no_rompe_el_resto() {
        let servicio = servicio();
        {
            let mut m = medidor(&servicio);
            m.anota(&trozo(": comentario de keep-alive\n"));
            m.anota(&trozo("data: {esto no es json}\n"));
            m.anota(&trozo(
                "data: {\"choices\":[{\"finish_reason\":\"length\"}]}\ndata: [DONE]\n",
            ));
        }

        let r = &servicio.uso.ultimos(1, None)[0];
        assert_eq!(r.motivo_fin.as_deref(), Some("length"));
    }
}
