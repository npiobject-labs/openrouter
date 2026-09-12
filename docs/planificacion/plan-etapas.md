# Plan por etapas — servicio openrouter

Fecha: 2026-09-12. Origen: `ideas-entrada.md` (entradas 01 a 04) y la decisión de
la entrada 03: **esto no es una app, es un conjunto de servicios pensados para ser
llamados por otras apps**. El frontend de `docs/` es solo una consola básica para
probar cada servicio a mano desde el navegador.

Cada etapa añade una API nueva al backend de `app/` y lo mínimo en `docs/` para
probarla. Una etapa se da por hecha cuando su criterio de verificación pasa en
`deploy.yml` y queda anotada en bitácora. No se empieza la siguiente hasta cerrar
la anterior.

## Reglas transversales

- **Contrato bajo `/v1/`**, compatible con la API de OpenAI donde exista
  equivalente (`/v1/chat/completions`, `/v1/models`). Lo propio del servicio va en
  rutas propias y en cabeceras `X-...`. Una vez publicada una ruta, no se rompe:
  los cambios incompatibles van a `/v2/`.
- **La clave de OpenRouter nunca sale del backend.** Vive como secreto de Fly
  (`OPENROUTER_API_KEY`). `docs/` es público: ni claves ni endpoints internos.
- **El servicio no es abierto.** Desde la etapa 1 toda ruta bajo `/v1/` exige una
  clave de servicio en `Authorization: Bearer <clave>`. Sin eso, publicar el proxy
  en Fly es regalar crédito.
- **CORS**: las rutas que consume `docs/` llevan `Access-Control-Allow-Origin: *`
  y responden al preflight (`OPTIONS`) permitiendo `Authorization` y
  `Content-Type`. Se hace con `tower-http` (`CorsLayer`), no a mano por ruta.
- **Secretos por workflow.** `deploy.yml` ya hace `flyctl secrets set BUILD_ID`;
  se amplía para volcar los secretos de repositorio que existan
  (`OPENROUTER_API_KEY`, `SERVICIO_CLAVE`) con `--stage` antes del deploy.
  [SUPUESTO] el usuario puede crear secretos de repositorio en GitHub. Plan B:
  `flyctl secrets set` una vez desde el PC.
- **Persistencia**: nada hasta la etapa 4. Antes, todo en memoria y se asume que
  se pierde al reiniciar la máquina de Fly.
- **Verificación sin gasto.** Los pasos de `deploy.yml` no lanzan consultas a
  modelos; usan rutas que no consumen crédito. Las pruebas con gasto las hace el
  usuario desde la consola de `docs/`.
- **Código**: `app/src/main.rs` se parte en módulos por etapa (`openrouter.rs`
  para el cliente HTTP, `auth.rs`, `rutas/*.rs`). Dependencias nuevas de la
  etapa 1: `reqwest` (features `json`, `rustls-tls`, sin `default-features`),
  `serde` (`derive`), `tower-http` (`cors`). El runtime ya lleva
  `ca-certificates`.

## Etapa 1 — Conexión a OpenRouter y una consulta

**Objetivo**: el servicio habla con OpenRouter y devuelve la respuesta de un
modelo. Nada más.

Rutas:

- `GET /v1/estado` → llama a `GET https://openrouter.ai/api/v1/key` con la clave
  del backend y devuelve `{"ok":true,"clave":{...}}` con la etiqueta de la clave,
  su uso y su límite tal como los da OpenRouter. No gasta crédito. Si falta la
  clave o OpenRouter la rechaza, `503` con motivo.
- `POST /v1/chat/completions` → proxy fino: reenvía el cuerpo tal cual a
  `POST https://openrouter.ai/api/v1/chat/completions` y devuelve la respuesta
  tal cual, incluido el bloque `usage`. Si el cuerpo no trae `model`, se rellena
  con `MODELO_DEFECTO` (variable de entorno, por defecto un modelo barato; se
  fija al implementar). Sin streaming: si llega `"stream": true` se responde
  `400`, el streaming es la etapa 7. Cabeceras `HTTP-Referer` y `X-Title` hacia
  OpenRouter con el nombre del proyecto.

Autenticación: una sola clave de servicio, secreto `SERVICIO_CLAVE`. Falta o no
coincide → `401`. Las credenciales por app llegan en la etapa 5.

Frontend: `docs/consola.html`. Campo para la clave de servicio (se guarda en
`localStorage`, nunca en el HTML), botón «Estado» que llama a `/v1/estado`,
área de texto para el prompt, botón «Enviar» y dos paneles: texto de la
respuesta y JSON crudo. Toma la URL del backend del mismo `<meta name="fly-app">`
y del mismo truco `?api=` que `docs/holamundo.html`. Enlace desde `index.html`.

Verificación en `deploy.yml`: `curl` a `/v1/estado` con la clave de servicio
del secreto de repositorio; falla si no devuelve `"ok":true`. Sigue la
verificación de `/salud`.

Criterio de hecho: desde Pages, con la clave pegada, «Estado» muestra la clave y
«Enviar» devuelve una respuesta del modelo por defecto con su `usage`.

No entra: catálogo, elección de modelo en la interfaz, registro de uso,
streaming, varias claves.

## Etapa 2 — Catálogo de modelos

- `GET /v1/models` → lista con el formato de OpenAI más los campos de OpenRouter
  que importan: precio de entrada y salida, ventana de contexto, modalidades,
  soporte de herramientas y de salida estructurada, proveedor.
- Caché en memoria con TTL (1 h) sobre `GET https://openrouter.ai/api/v1/models`.
  Si OpenRouter no responde y hay caché caducada, se sirve la caducada con
  cabecera `X-Cache: stale`.
- Filtros por query string: `?gratis=1`, `?proveedor=`, `?contexto_min=`,
  `?texto=` (busca en id y nombre).
- Consola: selector de modelo en `consola.html` alimentado por `/v1/models`, con
  filtro de texto y precio visible al lado de cada modelo. El modelo elegido se
  manda en el cuerpo.
- Verificación: `curl` a `/v1/models` devuelve más de 0 modelos.

## Etapa 3 — Medición por llamada

- Cada llamada a `/v1/chat/completions` genera un registro: id propio, fecha,
  modelo pedido, modelo servido, proveedor, tokens de entrada, salida,
  razonamiento y caché, coste según `usage`, latencia total, motivo de
  finalización, código de estado. En memoria, anillo de las últimas 1 000.
- La respuesta al cliente lleva `X-Uso-Id` con ese id.
- `GET /v1/uso` → últimas N (`?n=`, por defecto 50). `GET /v1/uso/{id}` → una.
- Reconciliación en segundo plano: al terminar la llamada se consulta
  `GET https://openrouter.ai/api/v1/generation?id=<id de OpenRouter>` y se
  actualiza el coste real y los tokens nativos. [SUPUESTO] ese dato tarda unos
  cientos de milisegundos en estar disponible; plan B: reintento con espera.
- Consola: ficha de coste bajo cada respuesta (tokens, coste, latencia,
  proveedor) y una pestaña «Uso» con la tabla de las últimas llamadas.
- No entra: persistencia, agregados. Se pierde al reiniciar.

## Etapa 4 — Persistencia y agregados

- Volumen de Fly (`fly volumes create`, 1 GB, misma región) montado en `/datos`;
  SQLite vía `rusqlite` (feature `bundled`). `fly.toml` con la sección `[mounts]`.
  Con volumen, `--ha=false` sigue siendo obligatorio: una sola máquina.
  [SUPUESTO] crear el volumen se puede hacer desde `deploy.yml` con
  `flyctl volumes create ... || true`; plan B: una vez desde el PC.
- Los registros de la etapa 3 pasan a la tabla `uso`; el anillo en memoria
  desaparece.
- `GET /v1/uso/resumen?desde=&hasta=&agrupar=dia|modelo` → totales de llamadas,
  tokens y coste.
- `GET /v1/uso/exportar?formato=csv|json&desde=&hasta=`.
- Consola: pestaña «Resumen» con totales por día y por modelo.
- Verificación: `/v1/estado` pasa a incluir `"almacen":"sqlite"` y el número de
  registros; `deploy.yml` comprueba que aparece.

## Etapa 5 — Apps consumidoras

- Varias claves de servicio, cada una con etiqueta de app. Tabla `apps` (id,
  nombre, hash de la clave, activa, creada). La clave se genera en el servicio y
  se enseña una sola vez.
- `SERVICIO_CLAVE` pasa a ser la clave de administración: solo ella puede llamar
  a `/v1/apps`.
- `POST /v1/apps` (crear, devuelve la clave), `GET /v1/apps`, `DELETE
  /v1/apps/{id}` (desactiva, no borra: el histórico la sigue referenciando).
- Cada registro de uso lleva `app_id`. `/v1/uso` y `/v1/uso/resumen` aceptan
  `?app=` y, con clave de app, solo devuelven lo de esa app.
- Campo opcional `X-Operacion` en la petición: identificador de operación de
  negocio que la app propaga; se guarda para agrupar varias llamadas de un mismo
  trabajo. Se acepta también el campo `user` del cuerpo, que ya entiende
  OpenRouter.
- Consola: pestaña «Apps» (solo con clave de administración): crear, listar,
  desactivar.

## Etapa 6 — Presupuestos, cuotas y cortacircuitos

- Por app: presupuesto en USD por periodo (día o mes) con dos umbrales: aviso
  y corte. Cuota de peticiones por minuto.
- `GET /v1/presupuesto` → margen restante de la app que llama, para que decida
  antes de gastar. `PUT /v1/apps/{id}/presupuesto` (administración).
- Corte: `402` con detalle cuando se supera; `429` en la cuota por minuto.
- Cortacircuitos por bucle: la misma app repitiendo el mismo cuerpo más de N
  veces en M segundos se corta con `429` y motivo `bucle`. Protege del gasto
  accidental; el presupuesto protege del legítimo excesivo.
- [SUPUESTO] OpenRouter ofrece claves hijas con límite de crédito por programa
  (provisioning keys) en la cuenta del usuario; si es así, se evalúa que el tope
  duro por app lo aplique OpenRouter y el servicio solo lo refleje. Plan B: el
  corte lo aplica el servicio, como se describe aquí.
- Consola: presupuesto y consumo del periodo en la cabecera; edición en «Apps».

## Etapa 7 — Streaming

- `/v1/chat/completions` acepta `"stream": true` y devuelve SSE tal como lo
  emite OpenRouter, con `stream_options.include_usage` forzado para recibir el
  bloque final de `usage`.
- El servicio intercepta el flujo entero: cuenta, mide el tiempo hasta el
  primer token y los tokens por segundo, y al cerrar el flujo escribe el
  registro de uso igual que en la etapa 3. Cancelación del cliente → registro
  con motivo `cancelado` y coste según lo recibido hasta entonces.
- Consola: respuesta en vivo con contador de tokens y botón «Cancelar».

## Etapa 8 — Alias y enrutado

- Tabla `alias`: nombre semántico («redactor», «clasificador»), modelo destino,
  cadena de respaldo ordenada, parámetros por defecto, prompt de sistema
  opcional, estrategia de proveedor (más barato, más rápido). Se traduce a los
  campos `models`, `provider` y `route` que OpenRouter ya soporta.
- Una app manda `"model": "alias:redactor"` y el servicio resuelve. La app no
  vuelve a escribir un nombre de modelo en su código.
- Registro de uso con alias, modelo pedido y modelo servido, para ver cuándo
  actuó el respaldo.
- `GET /v1/alias`, `PUT /v1/alias/{nombre}` (administración).
- Simulador: `GET /v1/alias/{nombre}/simular?modelo=&desde=&hasta=` → qué habría
  costado el histórico de ese alias con otro modelo detrás, a precios del
  catálogo actual.
- Consola: pestaña «Alias» y opción de elegir alias en el selector de modelo.

## Después (sin orden ni compromiso)

Especificación OpenAPI publicada en `docs/`, salida estructurada validada contra
esquema con reintento, trabajos diferidos con webhook, política de proveedores
por app (sin retención de datos, región), retención y purga configurables,
histórico diario de precios del catálogo, canario por alias, último respaldo
fuera de OpenRouter, detección de anomalías de gasto. Todo esto cuelga del
núcleo de las etapas 1 a 6 y no se planifica hasta tenerlo en producción.

## Lo que este plan descarta

- Un chat completo con hilos y carpetas: el valor está en medir y reutilizar.
- Guardar el contenido de los prompts: solo metadatos. Si algún día se guarda,
  será opcional por app y nunca en el repositorio público.
- Una interfaz de producto: `docs/consola.html` es una consola de pruebas y no
  crece más allá de lo que cada etapa necesita para verificarse a mano.
