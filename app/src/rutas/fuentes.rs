//! Rutas de las fuentes (etapa 12): `GET /v1/fuentes` para cualquiera con
//! clave (las apps no ven bases ni claves), y alta, baja y prueba de salud
//! solo con la de administración. Dar de alta una fuente no despliega nada.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use serde_json::{json, Value};

use crate::{
    apps::Identidad,
    catalogo::Origen,
    error::ErrorApi,
    fuentes::{self, Fuente},
    upstream::{Medicion, Upstream},
    Servicio,
};

fn solo_administracion(quien: &Identidad) -> Result<(), ErrorApi> {
    if quien.admin {
        return Ok(());
    }
    Err(ErrorApi::nuevo(
        StatusCode::FORBIDDEN,
        "solo_administracion",
        "Esta ruta exige la clave de administración del servicio.",
    ))
}

fn desconocida(id: &str) -> ErrorApi {
    ErrorApi::nuevo(
        StatusCode::NOT_FOUND,
        "fuente_desconocida",
        format!("No hay ninguna fuente activa llamada \"{id}\"."),
    )
}

/// La fuente activa a la que va un `model` con prefijo (`groq:…`), si la hay.
pub fn resolver(servicio: &Servicio, pedido: &str) -> Option<(Fuente, String)> {
    let (id, modelo) = fuentes::partir(pedido)?;
    let f = servicio
        .uso
        .con(|c| fuentes::una(c, id))
        .filter(|f| f.activa)?;
    Some((f, modelo.to_string()))
}

/// La fuente activa con ese id (para `/v1/models?upstream=<id>`).
pub fn activa(servicio: &Servicio, id: &str) -> Option<Fuente> {
    servicio
        .uso
        .con(|c| fuentes::una(c, id))
        .filter(|f| f.activa)
}

/// La clave de la fuente, descifrada solo para usarla.
pub fn clave_de(servicio: &Servicio, f: &Fuente) -> Result<Option<String>, ErrorApi> {
    let Some(cifrada) = &f.clave_cifrada else {
        return Ok(None);
    };
    let maestra = servicio
        .config
        .clave_maestra_fuentes
        .as_deref()
        .ok_or_else(|| {
            ErrorApi::sin_configurar("clave maestra de fuentes (FUENTES_CLAVE_MAESTRA)")
        })?;
    fuentes::descifrar(maestra, cifrada).map(Some).ok_or_else(|| {
        ErrorApi::nuevo(
            StatusCode::SERVICE_UNAVAILABLE,
            "fuente_clave_ilegible",
            format!(
                "La clave de la fuente {} no se puede descifrar: ¿ha cambiado FUENTES_CLAVE_MAESTRA? Vuelve a darla de alta con su clave.",
                f.nombre
            ),
        )
    })
}

/// El catálogo de una fuente: caché de una hora, y la copia caducada antes
/// que un error si la fuente no responde.
pub async fn catalogo(
    servicio: &Servicio,
    f: &Fuente,
    refrescar: bool,
) -> Result<(Vec<Value>, Origen), ErrorApi> {
    if let Some(lista) = servicio
        .catalogo_fuentes
        .vigente(&f.id)
        .filter(|_| !refrescar)
    {
        return Ok((lista, Origen::Cache));
    }
    let clave = clave_de(servicio, f)?;
    match fuentes::modelos(servicio.openrouter.http(), f, clave.as_deref()).await {
        Ok(lista) => {
            servicio.catalogo_fuentes.guardar(&f.id, lista.clone());
            Ok((lista, Origen::Fresco))
        }
        Err(e) => match servicio.catalogo_fuentes.caducada(&f.id) {
            Some(lista) => Ok((lista, Origen::Caducada)),
            None => Err(e),
        },
    }
}

/// `GET /v1/fuentes`: los tres upstreams de siempre y las fuentes
/// declarativas, con su salud. La administración ve también las inactivas,
/// su base y si tienen clave; nunca la clave.
pub async fn lista(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
) -> Json<Value> {
    let mut data: Vec<Value> = Upstream::TODOS
        .iter()
        .map(|u| {
            json!({
                "id": u.nombre(), "nombre": u.titulo(), "tipo": "agregador", "prefijo": u.prefijo(),
                "declarativa": false, "configurada": servicio.openrouter.configurado(*u),
                "medicion": match u.medicion() {
                    Medicion::Reconciliada => "reconciliada",
                    Medicion::Estimada => "estimada",
                    Medicion::Directa => "directa",
                },
                "region": if *u == Upstream::Requesty { json!("eu") } else { Value::Null },
                "retencion": Value::Null, "salud": Value::Null,
            })
        })
        .collect();
    data.extend(
        servicio
            .uso
            .con(fuentes::todas)
            .into_iter()
            .filter(|f| f.activa || quien.admin)
            .map(|f| f.publica(quien.admin)),
    );
    Json(json!({
        "object": "list", "data": data,
        "cifrado": servicio.config.clave_maestra_fuentes.is_some(),
    }))
}

fn texto(cuerpo: &Value, campo: &str) -> Option<String> {
    cuerpo
        .get(campo)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn invalido(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::nuevo(StatusCode::BAD_REQUEST, "cuerpo_invalido", mensaje)
}

/// `POST /v1/fuentes`: alta o cambio de una fuente declarativa compatible
/// con OpenAI. Comprueba su salud al momento (pide su catálogo).
pub async fn alta(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Json(cuerpo): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ErrorApi> {
    solo_administracion(&quien)?;
    let id = texto(&cuerpo, "id")
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| {
            invalido("Hace falta un \"id\" (será el prefijo del modelo: <id>:<modelo>).")
        })?;
    if !fuentes::id_valido(&id) {
        return Err(invalido(format!(
            "El id \"{id}\" no vale: de 2 a 24 caracteres en minúsculas, cifras o guiones, empezando por letra, y no puede ser {}.",
            fuentes::RESERVADOS.join(", ")
        )));
    }
    let base = texto(&cuerpo, "base").ok_or_else(|| {
        invalido("Hace falta la \"base\" de la API (p. ej. https://api.groq.com/openai/v1).")
    })?;
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err(invalido(
            "La base tiene que empezar por https:// (o http:// para una fuente local).",
        ));
    }
    let tipo = texto(&cuerpo, "tipo").unwrap_or_else(|| "openai_compatible".into());
    if !fuentes::TIPOS.contains(&tipo.as_str()) {
        return Err(invalido(format!(
            "El tipo vale uno de: {}.",
            fuentes::TIPOS.join(", ")
        )));
    }
    let coste = texto(&cuerpo, "coste").unwrap_or_else(|| "usage".into());
    if coste != "usage" && coste != "catalogo" {
        return Err(invalido("El coste vale \"usage\" (lo da la respuesta) o \"catalogo\" (se estima con los precios)."));
    }
    let precio = |k: &str| cuerpo.get(k).and_then(Value::as_f64).filter(|x| *x >= 0.0);
    let fecha = crate::uso::ahora_iso();
    let previa = servicio.uso.con(|c| fuentes::una(c, &id));
    let clave_cifrada = match texto(&cuerpo, "clave") {
        Some(clave) => {
            let maestra = servicio
                .config
                .clave_maestra_fuentes
                .as_deref()
                .ok_or_else(|| {
                    ErrorApi::sin_configurar("clave maestra de fuentes (FUENTES_CLAVE_MAESTRA)")
                })?;
            let nonce = servicio.uso.con(fuentes::nonce).map_err(|e| {
                ErrorApi::nuevo(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "sin_guardar",
                    e.to_string(),
                )
            })?;
            Some(fuentes::cifrar(maestra, &nonce, &clave).map_err(|e| {
                ErrorApi::nuevo(StatusCode::INTERNAL_SERVER_ERROR, "sin_guardar", e)
            })?)
        }
        None => None,
    };
    let mut f = Fuente {
        id: id.clone(),
        nombre: texto(&cuerpo, "nombre").unwrap_or_else(|| id.clone()),
        tipo,
        base: base.trim_end_matches('/').to_string(),
        ruta_modelos: texto(&cuerpo, "ruta_modelos").unwrap_or_else(|| "/models".into()),
        region: texto(&cuerpo, "region"),
        retencion: texto(&cuerpo, "retencion"),
        coste,
        precio_entrada: precio("precio_entrada"),
        precio_salida: precio("precio_salida"),
        activa: true,
        creada: previa
            .as_ref()
            .map_or_else(|| fecha.clone(), |p| p.creada.clone()),
        actualizada: fecha.clone(),
        salud: None,
        clave_cifrada,
    };
    servicio.uso.con(|c| fuentes::guardar(c, &f)).map_err(|e| {
        ErrorApi::nuevo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "sin_guardar",
            e.to_string(),
        )
    })?;
    // Con la clave que quede guardada (la nueva o la de antes).
    if let Some(g) = servicio.uso.con(|c| fuentes::una(c, &id)) {
        f = g;
    }
    servicio.catalogo_fuentes.olvidar(&id);
    let salud = comprobar(&servicio, &f).await;
    f.salud = Some(salud);
    let estado = if previa.is_some() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        estado,
        Json(json!({ "ok": true, "fuente": f.publica(true) })),
    ))
}

async fn comprobar(servicio: &Servicio, f: &Fuente) -> Value {
    let fecha = crate::uso::ahora_iso();
    let salud = match clave_de(servicio, f) {
        Ok(clave) => {
            let (s, lista) =
                fuentes::probar(servicio.openrouter.http(), f, clave.as_deref(), &fecha).await;
            if let Some(l) = lista {
                servicio.catalogo_fuentes.guardar(&f.id, l);
            }
            s
        }
        Err(e) => {
            json!({ "ok": false, "error": e.codigo, "mensaje": e.mensaje, "comprobada": fecha })
        }
    };
    servicio.uso.con(|c| fuentes::anota_salud(c, &f.id, &salud));
    salud
}

/// `DELETE /v1/fuentes/{id}`: la desactiva (no la borra).
pub async fn baja(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;
    let fecha = crate::uso::ahora_iso();
    if !servicio.uso.con(|c| fuentes::desactivar(c, &id, &fecha)) {
        return Err(desconocida(&id));
    }
    servicio.catalogo_fuentes.olvidar(&id);
    Ok(Json(json!({ "ok": true, "id": id, "activa": false })))
}

/// `POST /v1/fuentes/{id}/probar`: la salud ahora mismo.
pub async fn probar(
    State(servicio): State<Arc<Servicio>>,
    Extension(quien): Extension<Identidad>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    solo_administracion(&quien)?;
    let f = activa(&servicio, &id).ok_or_else(|| desconocida(&id))?;
    let salud = comprobar(&servicio, &f).await;
    Ok(Json(json!({ "ok": true, "id": id, "salud": salud })))
}

/// Para `/v1/models?upstream=<fuente>`.
pub async fn modelos(
    servicio: &Servicio,
    id: &str,
    refrescar: bool,
) -> Result<(Vec<Value>, Origen), ErrorApi> {
    let f = activa(servicio, id).ok_or_else(|| desconocida(id))?;
    catalogo(servicio, &f, refrescar).await
}

/// Filtros sencillos sobre el catálogo de una fuente (`texto`, `gratis`).
pub fn filtrar(lista: Vec<Value>, parametros: &HashMap<String, String>) -> Vec<Value> {
    let texto = parametros
        .get("texto")
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty());
    let gratis = matches!(
        parametros.get("gratis").map(String::as_str),
        Some("1" | "true")
    );
    lista
        .into_iter()
        .filter(|m| {
            texto.as_ref().is_none_or(|t| {
                m["id"].as_str().unwrap_or("").to_lowercase().contains(t)
                    || m["nombre"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(t)
            })
        })
        .filter(|m| !gratis || m["gratis"] == true)
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::{
        apps,
        catalogo::Catalogo,
        openrouter::{pruebas as falso, Cliente},
        uso::Uso,
    };
    use axum::{http::HeaderMap, response::IntoResponse};
    use std::sync::Mutex;

    type Vistas = Arc<Mutex<Vec<(String, String, Value)>>>;

    /// Una fuente compatible con OpenAI de mentira: catálogo y chat, y apunta
    /// (ruta, autorización, cuerpo) de cada petición.
    async fn fuente_falsa() -> (String, Vistas) {
        let vistas: Vistas = Arc::default();
        async fn modelos(
            axum::extract::State(v): axum::extract::State<Vistas>,
            c: HeaderMap,
        ) -> Json<Value> {
            let auth = c
                .get("authorization")
                .and_then(|x| x.to_str().ok())
                .unwrap_or("")
                .to_string();
            v.lock().unwrap().push(("models".into(), auth, Value::Null));
            Json(
                json!({"data": [{"id": "m1", "pricing": {"prompt": "0.000001", "completion": "0.000002"}, "context_length": 8192}, {"id": "m2"}]}),
            )
        }
        async fn chat(
            axum::extract::State(v): axum::extract::State<Vistas>,
            c: HeaderMap,
            Json(b): Json<Value>,
        ) -> axum::response::Response {
            let auth = c
                .get("authorization")
                .and_then(|x| x.to_str().ok())
                .unwrap_or("")
                .to_string();
            v.lock().unwrap().push(("chat".into(), auth, b.clone()));
            if b["model"] == "roto" {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"message": "modelo roto"}})),
                )
                    .into_response();
            }
            Json(json!({"id": "c1", "model": b["model"], "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "hola"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}})).into_response()
        }
        let r = axum::Router::new()
            .route("/v1/models", axum::routing::get(modelos))
            .route("/v1/chat/completions", axum::routing::post(chat))
            .with_state(vistas.clone());
        let escucha = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", escucha.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(escucha, r).await.unwrap() });
        (base, vistas)
    }

    fn servicio(maestra: Option<&str>) -> Arc<Servicio> {
        let mut config = falso::config("http://127.0.0.1:1", "http://127.0.0.1:1", None);
        config.clave_maestra_fuentes = maestra.map(str::to_string);
        Arc::new(Servicio {
            openrouter: Cliente::nuevo(&config),
            catalogo: Catalogo::default(),
            catalogo_video: crate::video::CatalogoVideo::default(),
            catalogo_fuentes: crate::fuentes::CatalogoFuentes::default(),
            uso: Uso::nuevo(Some("/no/existe/uso.db")),
            config,
        })
    }

    async fn alta_de(
        s: &Arc<Servicio>,
        quien: Identidad,
        cuerpo: Value,
    ) -> Result<(StatusCode, Json<Value>), ErrorApi> {
        alta(State(s.clone()), Extension(quien), Json(cuerpo)).await
    }

    #[tokio::test]
    async fn una_fuente_declarativa_de_extremo_a_extremo() {
        let (base, vistas) = fuente_falsa().await;
        let fecha = crate::uso::ahora_iso();
        let s = servicio(Some("maestra-de-prueba"));
        let (app, _) = s.uso.con(|c| apps::crea(c, "app1", &fecha)).unwrap();
        let de_app = Identidad {
            app: Some(app),
            admin: false,
        };
        let cuerpo = json!({"id": "prueba", "nombre": "Prueba", "base": base, "clave": "sk-de-la-fuente",
            "region": "eu", "coste": "catalogo"});
        // Solo la administración da de alta.
        let e = alta_de(&s, de_app.clone(), cuerpo.clone())
            .await
            .unwrap_err();
        assert_eq!(e.codigo, "solo_administracion");
        // Ids reservados o raros, no.
        let e = alta_de(
            &s,
            Identidad::administracion(),
            json!({"id": "hf", "base": base}),
        )
        .await
        .unwrap_err();
        assert_eq!(e.codigo, "cuerpo_invalido");
        // Alta: se guarda cifrada, nunca se devuelve y la salud se comprueba.
        let (estado, Json(v)) = alta_de(&s, Identidad::administracion(), cuerpo.clone())
            .await
            .unwrap();
        assert_eq!(estado, StatusCode::CREATED);
        assert!(!v.to_string().contains("sk-de-la-fuente"), "{v}");
        assert_eq!(v["fuente"]["con_clave"], true);
        assert_eq!(v["fuente"]["salud"]["ok"], true, "{v}");
        assert_eq!(v["fuente"]["salud"]["modelos"], 2);
        let guardada = s.uso.con(|c| crate::fuentes::una(c, "prueba")).unwrap();
        assert!(guardada
            .clave_cifrada
            .as_deref()
            .unwrap()
            .starts_with("v1:"));
        assert_eq!(vistas.lock().unwrap()[0].1, "Bearer sk-de-la-fuente");
        // Una app ve la fuente sin base ni clave; la administración, con base.
        let Json(l) = lista(State(s.clone()), Extension(de_app.clone())).await;
        let f = l["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "prueba")
            .unwrap()
            .clone();
        assert!(
            f.get("base").is_none() && f["prefijo"] == "prueba:" && f["region"] == "eu",
            "{f}"
        );
        assert!(l["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "openrouter" && x["declarativa"] == false));
        let Json(l) = lista(State(s.clone()), Extension(Identidad::administracion())).await;
        assert!(l.to_string().contains(&base) && !l.to_string().contains("sk-de-la-fuente"));
        // Chat con el prefijo: va a la fuente, sin prefijo, con su clave, medido.
        let r = crate::rutas::chat::chat(
            State(s.clone()),
            Extension(de_app.clone()),
            HeaderMap::new(),
            Json(json!({"model": "prueba:m1", "messages": [{"role": "user", "content": "hola"}]})),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["x-upstream"], "prueba");
        let id_uso = r.headers()["x-uso-id"].to_str().unwrap().to_string();
        {
            let v = vistas.lock().unwrap();
            let (_, auth, b) = v.iter().rev().find(|x| x.0 == "chat").unwrap();
            assert_eq!(auth, "Bearer sk-de-la-fuente");
            assert_eq!(b["model"], "m1");
        }
        let reg = s
            .uso
            .con(|c| {
                c.query_row(
                    "SELECT upstream, coste, coste_origen, app_id FROM uso WHERE id = ?1",
                    [&id_uso],
                    |f| {
                        Ok((
                            f.get::<_, String>(0)?,
                            f.get::<_, Option<f64>>(1)?,
                            f.get::<_, String>(2)?,
                            f.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
            })
            .unwrap();
        assert_eq!(reg.0, "prueba");
        assert!(
            (reg.1.unwrap() - (10.0 * 1.0 + 5.0 * 2.0) / 1e6).abs() < 1e-12,
            "{reg:?}"
        );
        assert_eq!(reg.2, "estimado");
        assert!(reg.3.is_some(), "anotado con su app");
        // Un rechazo de la fuente sale con su código y su cuerpo.
        let e = crate::rutas::chat::chat(
            State(s.clone()),
            Extension(de_app.clone()),
            HeaderMap::new(),
            Json(
                json!({"model": "prueba:roto", "messages": [{"role": "user", "content": "hola"}]}),
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.estado, e.codigo),
            (StatusCode::BAD_REQUEST, "fuente_rechaza")
        );
        // Catálogo de la fuente con /v1/models?upstream=prueba (de la caché).
        let mut p = HashMap::new();
        p.insert("upstream".to_string(), "prueba".to_string());
        let r = crate::rutas::modelos::modelos(State(s.clone()), axum::extract::Query(p.clone()))
            .await
            .unwrap()
            .into_response();
        assert_eq!(r.status(), StatusCode::OK);
        let cuerpo = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&cuerpo).unwrap();
        assert_eq!(v["data"][0]["id"], "prueba:m1");
        // /v1/uso?upstream=prueba filtra por la fuente.
        assert!(
            crate::rutas::uso::filtro_de_upstream(&s, &p)
                .unwrap()
                .as_deref()
                == Some("prueba")
        );
        // Probar y dar de baja (no se borra).
        let Json(v) = probar(
            State(s.clone()),
            Extension(Identidad::administracion()),
            Path("prueba".into()),
        )
        .await
        .unwrap();
        assert_eq!(v["salud"]["ok"], true);
        let Json(v) = baja(
            State(s.clone()),
            Extension(Identidad::administracion()),
            Path("prueba".into()),
        )
        .await
        .unwrap();
        assert_eq!(v["activa"], false);
        assert!(
            s.uso.con(|c| crate::fuentes::una(c, "prueba")).is_some(),
            "no se borra"
        );
        let Json(l) = lista(State(s.clone()), Extension(de_app)).await;
        assert!(!l.to_string().contains("\"prueba\""), "una app ya no la ve");
        let e = crate::rutas::modelos::modelos(State(s.clone()), axum::extract::Query(p.clone()))
            .await
            .err()
            .unwrap();
        assert_eq!(e.codigo, "upstream_desconocido");
        assert_eq!(
            crate::rutas::uso::filtro_de_upstream(&s, &p)
                .unwrap()
                .as_deref(),
            Some("prueba"),
            "su histórico sigue"
        );
    }

    #[tokio::test]
    async fn sin_clave_maestra_solo_fuentes_sin_clave() {
        let (base, _) = fuente_falsa().await;
        let s = servicio(None);
        let e = alta_de(
            &s,
            Identidad::administracion(),
            json!({"id": "conclave", "base": base, "clave": "x"}),
        )
        .await
        .unwrap_err();
        assert_eq!(e.codigo, "sin_configurar");
        let (estado, Json(v)) = alta_de(
            &s,
            Identidad::administracion(),
            json!({"id": "local", "tipo": "local", "base": base}),
        )
        .await
        .unwrap();
        assert_eq!(estado, StatusCode::CREATED);
        assert_eq!(v["fuente"]["salud"]["ok"], true);
        let Json(l) = lista(State(s.clone()), Extension(Identidad::administracion())).await;
        assert_eq!(l["cifrado"], false);
    }
}
