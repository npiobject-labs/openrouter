use axum::{
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::rutas::uso::CABECERA_USO;

/// Error de la API. Sale siempre con el mismo sobre, venga de aquí o de
/// OpenRouter, porque una app que llama no tiene por qué saber dónde falló:
///
/// ```json
/// {"ok":false,"error":{"message":"...","code":"...","type":"..."}}
/// ```
///
/// `message`, `code` y `type` van en inglés a propósito: es lo que leen los
/// clientes de la API de OpenAI sin tocar una línea. `ok` es añadido nuestro y
/// quien no lo conozca lo ignora.
#[derive(Debug)]
pub struct ErrorApi {
    pub estado: StatusCode,
    /// Código estable nuestro, en español, como `clave_invalida`. No cambia
    /// aunque cambie el texto del mensaje: es lo que una app puede comparar.
    pub codigo: &'static str,
    pub mensaje: String,
    /// El cuerpo tal cual lo mandó OpenRouter, cuando el fallo fue suyo. Sirve
    /// para depurar sin tener que mirar los logs del backend.
    pub upstream: Option<Value>,
    /// Id del registro de uso, cuando el fallo ocurrió dentro de una llamada
    /// medida: un error también consumió tiempo y merece quedar anotado.
    pub uso: Option<String>,
}

impl ErrorApi {
    pub fn nuevo(estado: StatusCode, codigo: &'static str, mensaje: impl Into<String>) -> Self {
        Self { estado, codigo, mensaje: mensaje.into(), upstream: None, uso: None }
    }

    pub fn sin_configurar(que: &str) -> Self {
        Self::nuevo(
            StatusCode::SERVICE_UNAVAILABLE,
            "sin_configurar",
            format!("El servicio no tiene {que} configurada; revisa los secretos de Fly."),
        )
    }

    /// Un rechazo de OpenRouter, traducido a nuestro sobre. Se conserva su
    /// código de estado: un 402 por saldo o un 404 por modelo inexistente
    /// significan lo mismo para quien llama, y taparlos con un 502 le quitaría
    /// la única pista útil.
    pub fn de_openrouter(estado: StatusCode, cuerpo: Value) -> Self {
        Self {
            estado,
            codigo: "openrouter_rechaza",
            mensaje: mensaje_de(&cuerpo)
                .unwrap_or_else(|| format!("OpenRouter respondió {estado} sin explicar el motivo.")),
            upstream: Some(cuerpo),
            uso: None,
        }
    }

    pub fn con_upstream(mut self, cuerpo: Value) -> Self {
        self.upstream = Some(cuerpo);
        self
    }

    pub fn con_uso(mut self, id: &str) -> Self {
        self.uso = Some(id.to_string());
        self
    }
}

/// El error de OpenRouter unas veces es `{"error":{"message":"..."}}` y otras
/// `{"error":"..."}` a secas. Se aceptan las dos y, si no hay ninguna, se
/// devuelve `None` para que decida quien llama.
fn mensaje_de(cuerpo: &Value) -> Option<String> {
    let error = cuerpo.get("error")?;
    match error {
        Value::String(texto) => Some(texto.clone()),
        _ => error
            .get("message")
            .or_else(|| error.get("mensaje"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// El `type` que esperan los clientes de OpenAI, deducido del código de estado.
/// Así una app puede ramificar sin conocer nuestros códigos propios.
fn tipo(estado: StatusCode) -> &'static str {
    match estado {
        StatusCode::BAD_REQUEST => "invalid_request_error",
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "authentication_error",
        StatusCode::PAYMENT_REQUIRED => "insufficient_quota",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        _ => "api_error",
    }
}

impl IntoResponse for ErrorApi {
    fn into_response(self) -> axum::response::Response {
        let mut error = json!({
            "message": self.mensaje,
            "code": self.codigo,
            "type": tipo(self.estado),
        });

        if let Some(upstream) = self.upstream {
            error["upstream"] = upstream;
        }

        let cuerpo = Json(json!({ "ok": false, "error": error }));
        match self.uso.and_then(|id| HeaderValue::from_str(&id).ok()) {
            Some(id) => (self.estado, [(CABECERA_USO, id)], cuerpo).into_response(),
            None => (self.estado, cuerpo).into_response(),
        }
    }
}
