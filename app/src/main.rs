mod apps;
mod auth;
mod catalogo;
mod config;
mod error;
mod guardia;
mod openrouter;
mod rutas;
mod uso;

use std::sync::Arc;

use axum::{
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    middleware,
    response::Response,
    routing::{delete, get, post, put},
    Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{catalogo::Catalogo, config::Config, openrouter::Cliente, uso::Uso};

/// Lo que comparten todas las rutas.
pub struct Servicio {
    pub config: Config,
    pub openrouter: Cliente,
    pub catalogo: Catalogo,
    pub uso: Uso,
}

#[tokio::main]
async fn main() {
    let config = Config::del_entorno();
    let puerto = config.puerto;

    // `openrouter-backend --salud`: el healthcheck del contenedor. La imagen no
    // lleva curl ni wget, y el binario ya tiene un cliente HTTP.
    if std::env::args().nth(1).as_deref() == Some("--salud") {
        std::process::exit(comprobar_salud(puerto).await);
    }

    let servicio = Arc::new(Servicio {
        openrouter: Cliente::nuevo(&config),
        catalogo: Catalogo::default(),
        uso: Uso::nuevo(config.bd.as_deref()),
        config,
    });

    // Todo /v1 exige clave de servicio. La capa va aquí y no en cada ruta para
    // que una ruta nueva nazca protegida.
    let v1 = Router::new()
        .route("/estado", get(rutas::estado::estado))
        .route("/models", get(rutas::modelos::modelos))
        .route("/chat/completions", post(rutas::chat::chat))
        .route("/uso", get(rutas::uso::lista))
        .route("/uso/resumen", get(rutas::uso::resumen))
        .route("/uso/exportar", get(rutas::uso::exportar))
        .route("/uso/{id}", get(rutas::uso::una))
        .route("/apps", post(rutas::apps::alta).get(rutas::apps::lista))
        .route("/apps/{id}", delete(rutas::apps::baja))
        .route("/apps/{id}/presupuesto", put(rutas::apps::limites))
        .route("/presupuesto", get(rutas::apps::presupuesto))
        .route_layer(middleware::from_fn_with_state(
            servicio.clone(),
            auth::exigir_clave,
        ));

    // docs/ se sirve desde Pages, que es otro origen. El preflight tiene que
    // pasar antes de la comprobación de clave, así que el CORS envuelve todo.
    // Sin CORS_ORIGENES, cualquiera (la consola de Pages es otro origen); con
    // la lista, solo esos, y el preflight del resto falla antes de llegar aquí.
    let origen = if servicio.config.cors_origenes.is_empty() {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(
            servicio
                .config
                .cors_origenes
                .iter()
                .filter_map(|o| o.parse().ok()),
        )
    };
    let cors = CorsLayer::new()
        .allow_origin(origen)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        // X-Operacion es parte del contrato: sin ella aquí, el preflight del
        // navegador rechaza a cualquier cliente web que la mande.
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            HeaderName::from_static("x-operacion"),
        ])
        // Sin esto el navegador no puede leer estas dos aunque viajen.
        .expose_headers([
            HeaderName::from_static("x-cache"),
            HeaderName::from_static("x-uso-id"),
            HeaderName::from_static("x-presupuesto"),
        ]);

    let app = Router::new()
        .route("/", get(rutas::basicas::raiz))
        .route("/salud", get(rutas::basicas::salud))
        .route("/holamundo", get(rutas::basicas::holamundo))
        .nest("/v1", v1)
        .fallback(rutas::basicas::desconocida)
        .layer(middleware::map_response(json_en_utf8))
        .layer(cors)
        .with_state(servicio);

    let direccion = format!("0.0.0.0:{puerto}");
    let escucha = tokio::net::TcpListener::bind(&direccion)
        .await
        .unwrap_or_else(|e| panic!("no se pudo abrir {direccion}: {e}"));

    println!("openrouter backend escuchando en {direccion}");

    axum::serve(escucha, app)
        .with_graceful_shutdown(apagado())
        .await
        .expect("fallo del servidor HTTP");
}

/// JSON siempre es UTF-8 (RFC 8259), pero quien no lo dice se lo encuentra
/// roto: PowerShell 5.1 decodifica como Latin-1 toda respuesta cuyo
/// `Content-Type` no traiga charset, asi que `Invoke-RestMethod` devuelve
/// «Â¡Hola!» y cada cliente acaba leyendo los bytes a mano. El CSV de
/// `/v1/uso/exportar` ya lo declaraba; esto pone al JSON al mismo nivel.
async fn json_en_utf8(mut respuesta: Response) -> Response {
    let cabeceras = respuesta.headers_mut();
    let json_pelado = cabeceras
        .get(CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes() == b"application/json");
    if json_pelado {
        cabeceras.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    respuesta
}

/// Ctrl+C en el PC y SIGTERM en Docker y en Fly: los dos deben cerrar bien.
/// Sin SIGTERM, `docker stop` mata el proceso a los diez segundos con la
/// conexión SQLite abierta y las reconciliaciones de coste a medias.
async fn apagado() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let sigterm = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = sigterm => {},
    }
    println!("openrouter backend apagándose");
}

/// Pide `/salud` al propio proceso y devuelve el código de salida del
/// healthcheck: 0 si responde `ok`, 1 si no.
async fn comprobar_salud(puerto: u16) -> i32 {
    let url = format!("http://127.0.0.1:{puerto}/salud");
    let cliente = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(_) => return 1,
    };
    match cliente.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            let cuerpo = r.text().await.unwrap_or_default();
            if cuerpo.contains("\"ok\":true") {
                0
            } else {
                1
            }
        }
        _ => 1,
    }
}
