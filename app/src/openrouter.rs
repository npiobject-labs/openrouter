use std::time::Duration;

use axum::http::StatusCode;
use reqwest::Client;
use serde_json::Value;

use crate::{
    config::{Config, REFERER, TITULO},
    error::ErrorApi,
};

/// Las consultas a un modelo pueden tardar; el resto de llamadas, no.
const ESPERA_CHAT: Duration = Duration::from_secs(120);
const ESPERA_CORTA: Duration = Duration::from_secs(20);

/// Cliente de la API de OpenRouter. Es el único sitio del servicio que conoce
/// la clave.
pub struct Cliente {
    http: Client,
    base: String,
    clave: Option<String>,
}

impl Cliente {
    pub fn nuevo(config: &Config) -> Self {
        let http = Client::builder()
            .user_agent(TITULO)
            .build()
            .expect("no se pudo construir el cliente HTTP");
        Self {
            http,
            base: config.base_openrouter.clone(),
            clave: config.clave_openrouter.clone(),
        }
    }

    fn clave(&self) -> Result<&str, ErrorApi> {
        self.clave
            .as_deref()
            .ok_or_else(|| ErrorApi::sin_configurar("clave de OpenRouter"))
    }

    /// `GET /key`: etiqueta, uso y límite de la clave. No gasta crédito, así que
    /// sirve para comprobar la conexión en cada despliegue.
    pub async fn info_clave(&self) -> Result<Value, ErrorApi> {
        let respuesta = self
            .http
            .get(format!("{}/key", self.base))
            .bearer_auth(self.clave()?)
            .timeout(ESPERA_CORTA)
            .send()
            .await
            .map_err(|e| sin_alcanzar("no se pudo consultar la clave", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta.json().await.map_err(respuesta_ilegible)?;

        if !estado.is_success() {
            return Err(ErrorApi::nuevo(
                StatusCode::SERVICE_UNAVAILABLE,
                "openrouter_rechaza",
                format!("OpenRouter respondió {estado} al comprobar la clave."),
            )
            .con_upstream(cuerpo));
        }

        // La respuesta viene envuelta en "data"; devolvemos solo el contenido.
        Ok(cuerpo.get("data").cloned().unwrap_or(cuerpo))
    }

    /// `GET /models`: el catálogo entero. Tampoco gasta crédito, pero son
    /// cientos de modelos, así que se cachea arriba.
    pub async fn modelos(&self) -> Result<Value, ErrorApi> {
        let respuesta = self
            .http
            .get(format!("{}/models", self.base))
            .bearer_auth(self.clave()?)
            .timeout(ESPERA_CORTA)
            .send()
            .await
            .map_err(|e| sin_alcanzar("no se pudo traer el catálogo", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta.json().await.map_err(respuesta_ilegible)?;

        if !estado.is_success() {
            return Err(ErrorApi::nuevo(
                StatusCode::BAD_GATEWAY,
                "openrouter_rechaza",
                format!("OpenRouter respondió {estado} al pedir el catálogo."),
            )
            .con_upstream(cuerpo));
        }
        Ok(cuerpo)
    }

    /// `POST /chat/completions`: se reenvía el cuerpo tal cual y se devuelve la
    /// respuesta tal cual, con su bloque `usage`. Si rechaza, el fallo sale por
    /// `ErrorApi` como cualquier otro: quien llama ve un solo formato de error.
    pub async fn chat(&self, cuerpo: Value) -> Result<Value, ErrorApi> {
        let respuesta = self
            .http
            .post(format!("{}/chat/completions", self.base))
            .bearer_auth(self.clave()?)
            // OpenRouter usa estas dos para atribuir el tráfico en su panel.
            .header("HTTP-Referer", REFERER)
            .header("X-Title", TITULO)
            .json(&cuerpo)
            .timeout(ESPERA_CHAT)
            .send()
            .await
            .map_err(|e| sin_alcanzar("no se pudo lanzar la consulta", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta.json().await.map_err(respuesta_ilegible)?;

        if !estado.is_success() {
            return Err(ErrorApi::de_openrouter(estado, cuerpo));
        }
        Ok(cuerpo)
    }
}

fn sin_alcanzar(que: &str, e: reqwest::Error) -> ErrorApi {
    let codigo = if e.is_timeout() {
        "openrouter_tardo_demasiado"
    } else {
        "openrouter_inalcanzable"
    };
    ErrorApi::nuevo(StatusCode::BAD_GATEWAY, codigo, format!("{que}: {e}"))
}

fn respuesta_ilegible(e: reqwest::Error) -> ErrorApi {
    ErrorApi::nuevo(
        StatusCode::BAD_GATEWAY,
        "respuesta_ilegible",
        format!("OpenRouter devolvió algo que no es JSON: {e}"),
    )
}
