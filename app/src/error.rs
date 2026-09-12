use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

/// Error de la API. Siempre sale como `{"ok":false,"error":{...}}`, para que la
/// app que llama distinga un fallo nuestro de una respuesta del modelo.
#[derive(Debug)]
pub struct ErrorApi {
    pub estado: StatusCode,
    pub codigo: &'static str,
    pub mensaje: String,
}

impl ErrorApi {
    pub fn nuevo(estado: StatusCode, codigo: &'static str, mensaje: impl Into<String>) -> Self {
        Self { estado, codigo, mensaje: mensaje.into() }
    }

    pub fn sin_configurar(que: &str) -> Self {
        Self::nuevo(
            StatusCode::SERVICE_UNAVAILABLE,
            "sin_configurar",
            format!("El servicio no tiene {que} configurada; revisa los secretos de Fly."),
        )
    }
}

impl IntoResponse for ErrorApi {
    fn into_response(self) -> axum::response::Response {
        let cuerpo = json!({
            "ok": false,
            "error": { "codigo": self.codigo, "mensaje": self.mensaje },
        });
        (self.estado, Json(cuerpo)).into_response()
    }
}
