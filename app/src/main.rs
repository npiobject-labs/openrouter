mod auth;
mod config;
mod error;
mod openrouter;
mod rutas;

use std::sync::Arc;

use axum::{
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        Method,
    },
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};

use crate::{config::Config, openrouter::Cliente};

/// Lo que comparten todas las rutas.
pub struct Servicio {
    pub config: Config,
    pub openrouter: Cliente,
}

#[tokio::main]
async fn main() {
    let config = Config::del_entorno();
    let puerto = config.puerto;

    let servicio = Arc::new(Servicio {
        openrouter: Cliente::nuevo(&config),
        config,
    });

    // Todo /v1 exige clave de servicio. La capa va aquí y no en cada ruta para
    // que una ruta nueva nazca protegida.
    let v1 = Router::new()
        .route("/estado", get(rutas::estado::estado))
        .route("/chat/completions", post(rutas::chat::chat))
        .route_layer(middleware::from_fn_with_state(
            servicio.clone(),
            auth::exigir_clave,
        ));

    // docs/ se sirve desde Pages, que es otro origen. El preflight tiene que
    // pasar antes de la comprobación de clave, así que el CORS envuelve todo.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);

    let app = Router::new()
        .route("/", get(rutas::basicas::raiz))
        .route("/salud", get(rutas::basicas::salud))
        .route("/holamundo", get(rutas::basicas::holamundo))
        .nest("/v1", v1)
        .layer(cors)
        .with_state(servicio);

    let direccion = format!("0.0.0.0:{puerto}");
    let escucha = tokio::net::TcpListener::bind(&direccion)
        .await
        .unwrap_or_else(|e| panic!("no se pudo abrir {direccion}: {e}"));

    println!("openrouter backend escuchando en {direccion}");

    axum::serve(escucha, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("fallo del servidor HTTP");
}
