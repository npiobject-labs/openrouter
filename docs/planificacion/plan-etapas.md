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

## Segundo agregador: Requesty (propuesta del 06/10/2026, aprobada el mismo día)

Origen: [`segundo-agregador-requesty.md`](segundo-agregador-requesty.md) y
[`alternativas-a-openrouter.md`](alternativas-a-openrouter.md). Objetivo: que
el gateway deje de depender de un solo agregador sin que las apps cambien nada,
y que mercamodels pueda enseñar y lanzar las ofertas de Requesty (proveedor y
región). Las etapas R0–R4 **se adelantan a la 7 (streaming) y a la 8 (alias)**;
la R0 lleva fecha fija.

### Decisiones que fija esta propuesta

- **Nada de lo publicado cambia por defecto.** `GET /v1/models` sin parámetros
  sigue devolviendo exactamente el catálogo de OpenRouter. Hay dos motivos: el
  contrato, y que la captura de mercamodels lee esa ruta y, si le entraran de
  golpe ~750 modelos, generaría fichas con LLM para todos. Requesty se pide con
  `?agregador=requesty|todos`.
- **Un id sin prefijo es de OpenRouter; uno con prefijo `requesty/` es de
  Requesty.** El prefijo hace falta porque 69 ids existen igual en los dos
  catálogos (`openai/…`, `anthropic/…`, `google/…`) y otros, como
  `vertex/gemini-2.5-flash-lite@europe-west1`, solo existen en Requesty. El
  gateway quita el prefijo antes de reenviar la petición. [SUPUESTO] OpenRouter
  no tiene ningún fabricante llamado `requesty`. Plan B: la R1 lo comprueba
  contra el catálogo real y, si chocara, se usa `rq:`.
- **Ningún dato del cuerpo elige URL ni credencial:** el prefijo solo
  selecciona uno de los dos upstreams declarados en `config.rs`. Es la lección
  del SSRF de LiteLLM.
- **Endpoint `https://router.eu.requesty.ai/v1` por defecto** (`REQUESTY_BASE`),
  porque procesar en Fráncfort no cuesta más. Que la inferencia quede también
  en la UE depende del id que se elija (`@eu`, `@europe-…`), no del endpoint.
- **Secretos**: `REQUESTY_API_KEY` (Fly) y `VPS_REQUESTY_API_KEY` (VPS), dos
  claves distintas como ya pasa con OpenRouter. Las crea el dueño en Requesty
  **con el log de prompts apagado** y las guarda como secretos de repositorio;
  nunca se pegan en una sesión. Son **opcionales**: sin clave, las rutas
  `requesty/…` responden `503 sin_configurar` y `deploy-vps.yml` no falla
  (no es como `VPS_OPENROUTER_API_KEY`, que sí es obligatoria).
- **Presupuestos, cuotas y cortacircuitos (`guardia.rs`) no cambian**: cuentan
  el coste real venga de donde venga, en dólares.
- **La verificación de cada despliegue sigue sin gastar**: catálogo, sobre de
  error y `503` sin clave. La llamada real a Requesty va en un workflow a mano,
  `probar-requesty.yml`, con texto de prueba y tope de `max_tokens`.

### Etapa R0 — Sustituto de `gemini-2.5-flash-lite` (antes del 20/10/2026)

**Estado (06/10/2026): hecha en `main` y en Fly** (`6a953ae` y `2249dfc`,
contrato 0.6.3); **falta el VPS**, que espera a «OK release». Decisión del
dueño, tras la comparación de mercamodels (run 37444012246):

- Sin `model` y solo texto: `openai/gpt-oss-120b`, con
  `reasoning: {effort: "low"}` si la app no fija el razonamiento.
- Sin `model` y con imagen, audio, vídeo o fichero:
  `google/gemini-3.1-flash-lite` (`MODELO_MULTIMODAL`).
- `/v1/estado` publica `modelo_defecto` y `modelo_multimodal`.

[SUPUESTO] Las variables de repositorio `MODELO_DEFECTO` y `MODELO_MULTIMODAL`
no existen; si `MODELO_DEFECTO` existe con el 2.5, manda sobre la constante.
Plan B: borrarla o cambiarla en *Settings → Variables*.


Hoy es el modelo por defecto (`MODELO_DEFECTO` en `config.rs` y en la variable
de repositorio), el de las fichas y el clasificador de mercamodels y el de la
extracción de maydom. Vertex lo apaga el 20/10/2026. [SUPUESTO] OpenRouter lo
seguirá sirviendo por Google AI Studio, pero con menos redundancia; Requesty
dejará de tenerlo en la UE.

- Candidatos que están en los dos agregadores y en la UE por Requesty:
  `openai/gpt-oss-120b`, `mistralai/mistral-small-2603`,
  `google/gemini-3.1-flash-lite` y DeepSeek V4 Flash. [SUPUESTO] los ids de
  OpenRouter son estos; se confirman con `/v1/models?texto=`.
- La prueba se hace en mercamodels (etapa 9 de su plan): la misma batería de
  fichas y clasificación, lanzada por el gateway con tope de 0,50 $, para medir
  JSON válido, acuerdo con las fichas actuales, coste y latencia. Cada app con
  casos propios (maydom, gestionpresupuestos con `tools/probar-bom.ps1
  -Comparar`) repite la prueba con los suyos.
- Cambio en el gateway: la constante `MODELO_DEFECTO` de `config.rs` y la
  variable de repositorio; para el VPS, un `deploy-vps.yml` a mano sobre el SHA
  de `release`, porque el `.env` se escribe al desplegar. En `docs/openapi.json`,
  la descripción del modelo por defecto.
- No entra: cambiar el modelo de las apps que lo escriben en su código. Eso es
  de cada app, o de los alias de la etapa 8.
- Hecho cuando: `/salud` del VPS sirve con el nuevo `MODELO_DEFECTO` antes del
  16/10 y una llamada sin `model` lo usa.

### Etapa R1 — Catálogo de Requesty en `/v1/models` (no necesita clave)

- `catalogo.rs` descarga también el catálogo público de Requesty, con la misma
  caché de una hora y una copia caducada propia: si Requesty no responde, la
  parte de OpenRouter se sirve igual.
- `?agregador=openrouter` (valor por defecto), `requesty` o `todos`. Los
  filtros de hoy valen para los dos.
- Campos nuevos, todos opcionales:
  - En todos los modelos: `agregador` y `nombre_canonico` (`canonical_slug` en
    OpenRouter, `model_canonical_name` en Requesty, normalizado).
  - En los de Requesty: `region` (`geolocation`), `retencion`, `retencion_dias`,
    `entrena` y `cuantizacion`, más los precios por tramo y de caché donde ya
    existan campos.
  - En los de OpenRouter, esos campos van a `null`, porque OpenRouter no los
    publica. Nunca a `false`.
- Filtros nuevos: `region=eu` y `sin_retencion=1` (sin retención y sin
  entrenamiento). Un modelo de OpenRouter no pasa ninguno de los dos.
- `X-Cache` informa del peor de los dos estados.
- `docs/openapi.json` sube a 0.7.0. Consola: selector de agregador en el
  catálogo.
- Verificación (`verificar-servicio.sh`):
  - `/v1/models` sin parámetros no trae ningún `requesty/`.
  - `?agregador=requesty` trae más de 0 modelos y ninguno sin `region`.
- [SUPUESTO] El catálogo de Requesty es público en `/v1/models` del endpoint UE
  (lo descargó así el run 37436216507), con precio por token en
  `input_price`/`output_price`. Plan B: si pide clave, R1 se aplaza hasta tener
  `REQUESTY_API_KEY`.

### Etapa R2 — Lanzar por Requesty (necesita la clave)

- `openrouter.rs` pasa a ser un módulo de upstreams. Lo común va en un cliente
  compartido: `chat()`, tiempo de espera y traducción de errores. Lo propio de
  cada upstream se queda en el suyo: en OpenRouter, `generacion()` y
  `HTTP-Referer`; en Requesty, nada más. Las claves solo las conoce ese módulo.
- `rutas/chat.rs`: `requesty/<id>` va a Requesty con `<id>`; lo demás, como
  hoy. Cabecera `X-Upstream` (`openrouter` | `requesty`) en la respuesta y en
  `expose_headers`.
- Errores con el mismo sobre:
  - `requesty_rechaza`, con su estado y su cuerpo en `error.upstream`.
  - `sin_configurar` si falta la clave.
  - Se añaden a `x-codigos-de-error`.
- `uso.rs`:
  - Columna `upstream` (por defecto `openrouter`, con la migración `ALTER` que
    ya existe) y columna `id_upstream`. `id_openrouter` se mantiene en la
    salida por compatibilidad.
  - Con Requesty, el coste se toma de `usage.cost` con
    `coste_origen = "requesty"`, sin conciliación.
  - La estimación previa (`guardia.rs` y `estima()`) usa los precios del
    catálogo del upstream que va a responder.
  - `/v1/uso/resumen` admite `agrupar=upstream`.
- `/salud` y `/v1/estado` dicen si hay clave de Requesty. [SUPUESTO] Requesty no
  tiene un equivalente a `GET /key`. Plan B: solo `"requesty":"configurada"`.
- `deploy.yml` y `deploy-vps.yml` vuelcan `REQUESTY_API_KEY` y
  `VPS_REQUESTY_API_KEY` si existen.
- `probar-requesty.yml` (a mano) hace estas llamadas:
  - una de texto,
  - una con `response_format` `json_schema` a un modelo `vertex/…@eu` y a uno
    `azure/…@swedencentral`,
  - una sin clave de aplicación.

  Después imprime `usage` y compara `usage.cost` con tokens × precio del
  catálogo, para resolver los dos [SUPUESTO] del 5 %.
- Hecho cuando: con la clave puesta, la consola lanza
  `requesty/vertex/…@europe-west1`, `/v1/uso` lo anota con `upstream` y coste
  real, y el presupuesto de la app lo descuenta.

### Etapa R3 — Respaldo cuando OpenRouter falla

- Equivalencia automática de id de OpenRouter a id de Requesty:
  - Se cruzan por nombre canónico normalizado (puntos y guiones, sufijos de
    fecha).
  - Entre las variantes de Requesty, se elige la de la UE, sin retención ni
    entrenamiento, con `json_schema` si la petición lo usa, y la más barata.
- Una tabla a mano (`EQUIVALENCIAS`, en el código) manda sobre lo automático.
  `/v1/models` añade `respaldo` (el id equivalente o `null`).
- Cuándo se conmuta:
  - Pasa a Requesty con el id equivalente si hay error de conexión, se agota el
    tiempo de espera, o OpenRouter responde 5xx, 429 o 402.
  - Cualquier otro 4xx se devuelve tal cual.
  - Si fallan los dos, `502 todos_fallan` con los dos errores en
    `error.upstream`.
  - Nunca se conmuta a mitad de un stream; eso aplica cuando llegue la etapa 7.
- Cortacircuitos por upstream: tras N fallos seguidos, OpenRouter se salta
  durante M segundos. Cada intento que cobre un upstream se anota en el uso y
  en el presupuesto.
- Desactivable por petición con la cabecera `X-Respaldo: no`. Ese «no» es lo
  que mandará una operación «solo UE» cuando pruebe OpenRouter.
- Verificación sin gasto: el sobre de `todos_fallan` se fuerza en una prueba
  unitaria con `OPENROUTER_BASE` apuntando a un host inválido.
  `probar-requesty.yml` hace lo mismo contra el despliegue de Fly.

### Etapa R4 — «Solo UE» por aplicación u operación (más adelante)

- Política por aplicación en `PUT /v1/apps/{id}/politica`:
  `{"solo_ue":true,"sin_retencion":true}`.
- Con esa política, la llamada va directa a Requesty. Si el id pedido no cumple
  la política, se traduce a su variante UE; si no tiene una, se responde
  `422 sin_oferta_ue`. Nunca se conmuta fuera de la UE: si Requesty falla, `502`.
- Junto a esta etapa, OpenRouter con `provider: {zdr: true, data_collection:
  "deny"}`:
  - **por aplicación**, no global: forzarlo en todas las llamadas dejaría sin
    proveedor (404) a los modelos que no tienen ninguno ZDR y rompería a quien
    hoy funciona;
  - mientras tanto, una app puede mandarlo ya en el cuerpo, porque el proxy lo
    reenvía tal cual.

### Orden y dependencias

R0 primero, por la fecha. R1 no espera a la clave. R2 espera a
`REQUESTY_API_KEY` y R3 a R2. Cada etapa llega al VPS solo cuando el dueño dice
«OK release». mercamodels empieza sus ofertas tras R1 y lanza por Requesty
tras R2 en el VPS.

## Vídeo: generación y edición (propuesta del 06/10/2026, pendiente de visto bueno)

Petición del dueño: un catálogo de modelos para resolver problemas de vídeo,
desde la edición hasta la generación, y lanzarlos desde mercamodels, incluida
**Higgsfield**. Tope: **10 $/mes** de vídeo.

Por qué hoy no hay ninguno: el catálogo de chat de OpenRouter
(`/api/v1/models`, el que sirve `/v1/models`) no trae generadores de vídeo.
OpenRouter los sirve desde el 15/04/2026 en una **API aparte y asíncrona**:
`POST /api/v1/videos` encola el trabajo, `GET /api/v1/videos/{id}` consulta su
estado (`pending`, `in_progress`, `completed`, `failed`) y, al acabar, trae
`unsigned_urls` y `usage.cost` en dólares. Modelos: Veo 3.1, Seedance 2.x,
Sora 2 Pro, Wan, Kling y HeyGen, entre otros. Higgsfield tiene su propia API,
también asíncrona (`https://api.higgsfield.ai`, envío a un endpoint por modelo
y consulta o webhook), con pago por uso en dólares y más de 50 modelos:
generación, edición por instrucción (Kling 3.0 Omni Edit, FLUX Video Edit),
alargar, reencuadrar, ampliar a 4K y sincronizar labios. No cobra los trabajos
fallidos. Lo de esta sección se ha leído por resúmenes de búsqueda: openrouter.ai
y higgsfield.ai están bloqueados desde la sesión.

Precios orientativos por segundo: Kling 2.5 a 0,042 $; Kling 3.0 a 0,112 $;
Seedance 2.0 entre 0,14 y 0,30 $; Veo 3.1 Fast entre 0,10 y 0,15 $; Veo 3.1
entre 0,20 y 0,40 $ con audio. Un clip de 8 s cuesta entre 0,35 y 3,20 $:
**10 $/mes dan para unos 10–25 clips cortos**.

### Decisiones que fija esta propuesta

- **Rutas propias bajo `/v1/videos`**, con la forma de la API de vídeo de
  OpenAI donde coincida (`POST /v1/videos`, `GET /v1/videos/{id}`). Nada de lo
  publicado cambia: `/v1/models` sigue siendo solo de chat. Si los modelos de
  vídeo entraran ahí, la captura de mercamodels los tomaría por modelos de
  texto.
- **Un id con prefijo `higgsfield/` va a Higgsfield**; sin prefijo, a la API
  de vídeo de OpenRouter. Es la misma regla que `requesty/`. Secretos
  opcionales: `HIGGSFIELD_API_KEY` (Fly) y `VPS_HIGGSFIELD_API_KEY` (VPS). Sin
  ellos, las rutas `higgsfield/…` responden `503 sin_configurar`.
- **El gateway no guarda vídeos.** Devuelve las URL del proveedor y dice
  cuándo caducan; descargarlas a tiempo es cosa de quien llama. El volumen de
  Fly es de 1 GB y es para el histórico.
- **El presupuesto se comprueba antes de encolar**, con el coste estimado
  (precio por segundo × duración). Un vídeo cuesta lo que cien consultas de
  texto: si no cabe, `402` antes de gastar. Al terminar se anota el coste real.
- **El vídeo va con su propia clave de aplicación**, `mercamodels-video`, con
  10 $/mes de tope, aparte de los 5 $ de texto de `mercamodels`. Así no hacen
  falta topes por tipo dentro de una misma app.
- **La verificación no gasta**: catálogo de vídeo, sobre de error y `402` con
  un presupuesto a cero. Las pruebas con gasto, en un workflow a mano con un
  clip de 2 s del modelo más barato.

### Etapa V1 — Catálogo de vídeo (`GET /v1/videos/models`)

- Junta el catálogo de vídeo de OpenRouter y el de Higgsfield, con caché de
  una hora y copia caducada si un proveedor no responde.
- Cada modelo trae:
  - `id` y `agregador`;
  - `tareas`, de una lista cerrada: `texto_a_video`, `imagen_a_video`,
    `video_a_video`, `alargar`, `ampliar`, `labios`, `avatar`;
  - `duraciones`, `resoluciones`, `formatos` y `audio`;
  - `precios`: dólares por segundo según resolución y audio;
  - `descripcion`.
- Filtros: `tarea`, `audio`, `max_precio_segundo`.
- [SUPUESTO] OpenRouter publica la lista de modelos de vídeo con precios por
  API; su documentación lo describe, pero no se ha leído la ruta exacta. Plan
  B: un workflow temporal lee la doc desde el runner. [SUPUESTO] Higgsfield no
  tiene listado de modelos con precios; plan B: una tabla a mano en
  `app/src/video_higgsfield.json`, con la fecha en que se comprobó cada precio.

### Etapa V2 — Generar (trabajos asíncronos)

- `POST /v1/videos` admite dos cuerpos:
  - texto o imagen a vídeo: `{model, prompt, imagen?, duracion, resolucion,
    formato, audio}`;
  - edición: `{model, prompt, video_url, …}`.
- Responde `202` con `{id, estado, coste_estimado}` y la cabecera `X-Uso-Id`.
- Tabla `videos` en SQLite: id propio, id del proveedor, app, operación,
  estado, coste estimado y real, URL y caducidad.
- Una tarea en segundo plano consulta al proveedor (5 s, 10 s, 30 s y después
  cada minuto, hasta 30 minutos).
- `GET /v1/videos/{id}` devuelve el estado y, al acabar, `urls` y `caduca`.
- Registro de uso:
  - `tipo = "video"`, segundos y resolución;
  - coste real de `usage.cost` (OpenRouter) o de la tarifa (Higgsfield);
  - un fallo cuesta 0 si el proveedor no lo cobra.
- Contrato 0.8.0. La consola gana una pestaña «Vídeo» para probar a mano.

### Etapa V3 — Editar

Vídeo a vídeo, alargar, ampliar y labios con los modelos de Higgsfield y los de
OpenRouter que lo admitan.

- El vídeo de entrada va por URL.
- [SUPUESTO] Los proveedores aceptan vídeos de hasta 100 MB por URL. Plan B:
  la página de mercamodels sube el vídeo a un almacén temporal; eso se decide
  en V3, no antes.

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
