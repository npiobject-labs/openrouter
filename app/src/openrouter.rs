use std::time::Duration;

use axum::http::StatusCode;
use reqwest::Client;
use serde_json::{json, Value};

use crate::{
    config::{Config, REFERER, TITULO},
    error::ErrorApi,
    upstream::{Conexion, Upstream},
};

/// Las consultas a un modelo pueden tardar; el resto de llamadas, no.
const ESPERA_CHAT: Duration = Duration::from_secs(120);
const ESPERA_CORTA: Duration = Duration::from_secs(20);

/// Cliente HTTP hacia los upstreams. Es el único sitio del servicio que conoce
/// las claves: la de OpenRouter desde la etapa 1 y, desde la 9, el token del
/// router de Hugging Face. Cada método recibe el `Upstream` al que habla;
/// `info_clave` y `generacion` son solo de OpenRouter porque solo él las tiene.
pub struct Cliente {
    http: Client,
    conexiones: [Conexion; 2],
}

impl Cliente {
    pub fn nuevo(config: &Config) -> Self {
        let http = Client::builder()
            .user_agent(TITULO)
            .build()
            .expect("no se pudo construir el cliente HTTP");
        Self {
            http,
            conexiones: [
                Conexion {
                    upstream: Upstream::OpenRouter,
                    base: config.base_openrouter.clone(),
                    clave: config.clave_openrouter.clone(),
                },
                Conexion {
                    upstream: Upstream::HuggingFace,
                    base: config.base_hf.clone(),
                    clave: config.clave_hf.clone(),
                },
            ],
        }
    }

    fn conexion(&self, upstream: Upstream) -> &Conexion {
        self.conexiones
            .iter()
            .find(|c| c.upstream == upstream)
            .expect("cada upstream tiene su conexión")
    }

    /// Si hay clave para ese upstream. No dice cuál.
    pub fn configurado(&self, upstream: Upstream) -> bool {
        self.conexion(upstream).configurada()
    }

    /// `{"openrouter": true, "hf": false}`: lo que enseñan `/salud` y
    /// `/v1/estado` para que un despliegue sepa qué puede comprar.
    pub fn configurados(&self) -> Value {
        let mut mapa = serde_json::Map::new();
        for c in &self.conexiones {
            mapa.insert(c.upstream.nombre().to_string(), json!(c.configurada()));
        }
        Value::Object(mapa)
    }

    /// La base y la clave de un upstream, o el 503 que corresponde. Para
    /// OpenRouter sigue siendo `sin_configurar`, que es lo publicado; para los
    /// demás, `upstream_sin_configurar`, porque el servicio sí funciona.
    fn acceso(&self, upstream: Upstream) -> Result<(&str, &str), ErrorApi> {
        let c = self.conexion(upstream);
        let clave = c.clave.as_deref().ok_or_else(|| match upstream {
            Upstream::OpenRouter => ErrorApi::sin_configurar("clave de OpenRouter"),
            otro => ErrorApi::upstream_sin_configurar(otro),
        })?;
        Ok((&c.base, clave))
    }

    /// `GET /key`: etiqueta, uso y límite de la clave. No gasta crédito, así que
    /// sirve para comprobar la conexión en cada despliegue. Solo OpenRouter.
    pub async fn info_clave(&self) -> Result<Value, ErrorApi> {
        let upstream = Upstream::OpenRouter;
        let (base, clave) = self.acceso(upstream)?;
        let respuesta = self
            .http
            .get(format!("{base}/key"))
            .bearer_auth(clave)
            .timeout(ESPERA_CORTA)
            .send()
            .await
            .map_err(|e| sin_alcanzar(upstream, "no se pudo consultar la clave", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta
            .json()
            .await
            .map_err(|e| respuesta_ilegible(upstream, e))?;

        if !estado.is_success() {
            return Err(ErrorApi::nuevo(
                StatusCode::SERVICE_UNAVAILABLE,
                upstream.codigo_rechaza(),
                format!("OpenRouter respondió {estado} al comprobar la clave."),
            )
            .con_upstream(cuerpo));
        }

        // La respuesta viene envuelta en "data"; devolvemos solo el contenido.
        Ok(cuerpo.get("data").cloned().unwrap_or(cuerpo))
    }

    /// `GET /models`: el catálogo entero del upstream. No gasta crédito, pero
    /// son cientos de modelos, así que se cachea arriba.
    ///
    /// A OpenRouter se le pide con `output_modalities=all`: sin el parámetro
    /// devuelve solo los modelos que escriben texto y esconde los de imagen,
    /// vídeo, voz, transcripción y embeddings. Hugging Face no lo conoce y
    /// devuelve todo lo que sirve.
    pub async fn modelos(&self, upstream: Upstream) -> Result<Value, ErrorApi> {
        let (base, clave) = self.acceso(upstream)?;
        let mut peticion = self.http.get(format!("{base}/models"));
        if upstream == Upstream::OpenRouter {
            peticion = peticion.query(&[("output_modalities", "all")]);
        }
        let respuesta = peticion
            .bearer_auth(clave)
            .timeout(ESPERA_CORTA)
            .send()
            .await
            .map_err(|e| sin_alcanzar(upstream, "no se pudo traer el catálogo", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta
            .json()
            .await
            .map_err(|e| respuesta_ilegible(upstream, e))?;

        if !estado.is_success() {
            return Err(ErrorApi::nuevo(
                StatusCode::BAD_GATEWAY,
                upstream.codigo_rechaza(),
                format!(
                    "{} respondió {estado} al pedir el catálogo.",
                    upstream.titulo()
                ),
            )
            .con_upstream(cuerpo));
        }
        Ok(cuerpo)
    }

    /// `POST /chat/completions`: se reenvía el cuerpo tal cual y se devuelve la
    /// respuesta tal cual, con su bloque `usage`. Si rechaza, el fallo sale por
    /// `ErrorApi` como cualquier otro: quien llama ve un solo formato de error.
    /// El `model` del cuerpo tiene que venir ya sin nuestro prefijo: de eso se
    /// encarga `Destino`, antes de llegar aquí.
    pub async fn chat(&self, upstream: Upstream, cuerpo: Value) -> Result<Value, ErrorApi> {
        let (base, clave) = self.acceso(upstream)?;
        let mut peticion = self
            .http
            .post(format!("{base}/chat/completions"))
            .bearer_auth(clave);
        if upstream == Upstream::OpenRouter {
            // OpenRouter usa estas dos para atribuir el tráfico en su panel.
            peticion = peticion
                .header("HTTP-Referer", REFERER)
                .header("X-Title", TITULO);
        }
        let respuesta = peticion
            .json(&cuerpo)
            .timeout(ESPERA_CHAT)
            .send()
            .await
            .map_err(|e| sin_alcanzar(upstream, "no se pudo lanzar la consulta", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta
            .json()
            .await
            .map_err(|e| respuesta_ilegible(upstream, e))?;

        if !estado.is_success() {
            return Err(ErrorApi::de_upstream(upstream, estado, cuerpo));
        }
        Ok(cuerpo)
    }

    /// `GET /generation?id=`: la contabilidad real de una generación ya
    /// terminada. Tampoco gasta crédito. Solo OpenRouter la tiene: el coste de
    /// Hugging Face se queda en la estimación del catálogo.
    ///
    /// [SUPUESTO] el coste viene en `total_cost` y el proveedor en
    /// `provider_name`, dentro de `data`. Plan B si cambian esos nombres: el
    /// registro se queda con el coste estimado del catálogo, que es lo que ya
    /// hace cuando esta llamada falla.
    pub async fn generacion(&self, id: &str) -> Result<Value, ErrorApi> {
        let upstream = Upstream::OpenRouter;
        let (base, clave) = self.acceso(upstream)?;
        let respuesta = self
            .http
            .get(format!("{base}/generation"))
            .query(&[("id", id)])
            .bearer_auth(clave)
            .timeout(ESPERA_CORTA)
            .send()
            .await
            .map_err(|e| sin_alcanzar(upstream, "no se pudo consultar la generación", e))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta
            .json()
            .await
            .map_err(|e| respuesta_ilegible(upstream, e))?;

        if !estado.is_success() {
            return Err(ErrorApi::de_upstream(upstream, estado, cuerpo));
        }
        Ok(cuerpo.get("data").cloned().unwrap_or(cuerpo))
    }
}

fn sin_alcanzar(upstream: Upstream, que: &str, e: reqwest::Error) -> ErrorApi {
    let codigo = if e.is_timeout() {
        upstream.codigo_tardo_demasiado()
    } else {
        upstream.codigo_inalcanzable()
    };
    ErrorApi::nuevo(StatusCode::BAD_GATEWAY, codigo, format!("{que}: {e}"))
}

fn respuesta_ilegible(upstream: Upstream, e: reqwest::Error) -> ErrorApi {
    ErrorApi::nuevo(
        StatusCode::BAD_GATEWAY,
        "respuesta_ilegible",
        format!("{} devolvió algo que no es JSON: {e}", upstream.titulo()),
    )
}

#[cfg(test)]
mod pruebas {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use super::*;

    /// Un servidor HTTP de una sola petición en un hilo, para ver qué manda el
    /// cliente sin salir de la máquina: la sesión no llega ni a OpenRouter ni
    /// a Hugging Face. Devuelve la base (`http://127.0.0.1:puerto`) y un canal
    /// por el que llega la petición cruda (línea de estado, cabeceras y cuerpo).
    fn servidor_falso(estado: u16, cuerpo: &str) -> (String, mpsc::Receiver<String>) {
        let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", escucha.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let respuesta = format!(
            "HTTP/1.1 {estado} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{cuerpo}",
            cuerpo.len()
        );
        thread::spawn(move || {
            let (mut flujo, _) = escucha.accept().unwrap();
            let mut bytes = Vec::new();
            let mut trozo = [0u8; 4096];
            loop {
                let n = flujo.read(&mut trozo).unwrap_or(0);
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&trozo[..n]);
                let texto = String::from_utf8_lossy(&bytes);
                if let Some(fin) = texto.find("\r\n\r\n") {
                    let largo = texto
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if bytes.len() >= fin + 4 + largo {
                        break;
                    }
                }
            }
            flujo.write_all(respuesta.as_bytes()).unwrap();
            let _ = flujo.flush();
            let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
        });
        (base, rx)
    }

    fn config(base_openrouter: &str, base_hf: &str, clave_hf: Option<&str>) -> Config {
        Config {
            clave_openrouter: Some("sk-or-prueba".into()),
            clave_hf: clave_hf.map(str::to_string),
            clave_servicio: None,
            modelo_defecto: "m".into(),
            modelo_multimodal: "mm".into(),
            base_openrouter: base_openrouter.into(),
            base_hf: base_hf.into(),
            build: "dev".into(),
            puerto: 0,
            bd: None,
            cors_origenes: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_openrouter_se_le_pide_el_catalogo_completo() {
        let (base, peticiones) = servidor_falso(200, r#"{"data":[{"id":"a/b"}]}"#);
        let cliente = Cliente::nuevo(&config(&base, "http://127.0.0.1:1", None));

        let cuerpo = cliente.modelos(Upstream::OpenRouter).await.unwrap();
        assert_eq!(cuerpo["data"][0]["id"], "a/b");

        let peticion = peticiones.recv().unwrap();
        let primera = peticion.lines().next().unwrap();
        assert!(
            primera.starts_with("GET /models?output_modalities=all HTTP/1.1"),
            "{primera}"
        );
        assert!(peticion.contains("authorization: Bearer sk-or-prueba"));
    }

    #[tokio::test]
    async fn sin_token_de_hugging_face_responde_503_upstream_sin_configurar() {
        let cliente = Cliente::nuevo(&config("http://127.0.0.1:1", "http://127.0.0.1:1", None));
        assert!(cliente.configurado(Upstream::OpenRouter));
        assert!(!cliente.configurado(Upstream::HuggingFace));
        assert_eq!(
            cliente.configurados(),
            json!({ "openrouter": true, "hf": false })
        );

        let fallo = cliente
            .chat(
                Upstream::HuggingFace,
                json!({ "model": "openai/gpt-oss-120b" }),
            )
            .await
            .unwrap_err();
        assert_eq!(fallo.estado, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(fallo.codigo, "upstream_sin_configurar");
        assert!(fallo.mensaje.contains("HF_TOKEN"), "{}", fallo.mensaje);

        // El catálogo de Hugging Face, igual.
        let fallo = cliente.modelos(Upstream::HuggingFace).await.unwrap_err();
        assert_eq!(fallo.codigo, "upstream_sin_configurar");
    }

    #[tokio::test]
    async fn la_consulta_a_hugging_face_lleva_su_token_y_el_modelo_sin_prefijo() {
        let (base, peticiones) = servidor_falso(
            200,
            r#"{"id":"chatcmpl-1","model":"openai/gpt-oss-120b","usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
        );
        let cliente = Cliente::nuevo(&config("http://127.0.0.1:1", &base, Some("hf_prueba")));

        let respuesta = cliente
            .chat(
                Upstream::HuggingFace,
                json!({ "model": "openai/gpt-oss-120b:groq", "messages": [] }),
            )
            .await
            .unwrap();
        assert_eq!(respuesta["usage"]["completion_tokens"], 5);

        let peticion = peticiones.recv().unwrap();
        assert!(
            peticion.starts_with("POST /chat/completions HTTP/1.1"),
            "{peticion}"
        );
        assert!(peticion.contains("authorization: Bearer hf_prueba"));
        assert!(
            !peticion.contains("sk-or-prueba"),
            "la clave de OpenRouter no puede viajar a Hugging Face"
        );
        assert!(
            !peticion.to_ascii_lowercase().contains("http-referer"),
            "las cabeceras de atribución son de OpenRouter"
        );
        assert!(peticion.contains(r#""model":"openai/gpt-oss-120b:groq""#));
    }

    #[tokio::test]
    async fn un_rechazo_de_hugging_face_sale_con_su_codigo_y_su_cuerpo() {
        let (base, _peticiones) = servidor_falso(
            404,
            r#"{"error":{"message":"The requested model does not exist."}}"#,
        );
        let cliente = Cliente::nuevo(&config("http://127.0.0.1:1", &base, Some("hf_prueba")));

        let fallo = cliente
            .chat(Upstream::HuggingFace, json!({ "model": "no/existe" }))
            .await
            .unwrap_err();
        assert_eq!(fallo.estado, StatusCode::NOT_FOUND, "se conserva el estado");
        assert_eq!(fallo.codigo, "hf_rechaza");
        assert_eq!(fallo.mensaje, "The requested model does not exist.");
        assert!(fallo.upstream.is_some());
    }
}
