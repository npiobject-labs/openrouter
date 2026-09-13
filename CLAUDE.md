# openrouter — instrucciones del proyecto

Flujo "PC arranca, móvil continúa": el desarrollo, la revisión y las pruebas se hacen desde sesiones en la nube (claude.ai/code con este repo seleccionado, desde web o móvil), con el PC apagado. Trabaja en español. Perfil del usuario: desarrollador senior en solitario; no expliques conceptos básicos; marca toda suposición no verificada como [SUPUESTO] e indica su plan B.

## Parámetros

| Parámetro | Valor |
|---|---|
| Proyecto | `openrouter` |
| Owner de GitHub | `npiobject-labs` |
| App de Fly.io | `openrouter-npiobject-labs` |
| Carpeta de Drive (id) | `1PHZR1t8BjYHhIqvCzrzA2US6S1SVRDEj` |

Esta tabla la rellena sola `.github/workflows/init-plantilla.yml` en el primer push de un repo creado desde la plantilla; no hay nada que tocar a mano salvo el id de Drive.

- **App de Fly.io**: `derivada` significa que `deploy.yml` la calcula como `<repo>-<owner>` en minúsculas, saneado a `[a-z0-9-]` y recortado a 30 caracteres. Si existe la variable de repositorio `FLY_APP`, esa manda; anota aquí el valor cuando la definas.
- **Carpeta de Drive (id)**: vacío significa que este proyecto no usa Drive. Ver ARRANQUE.md para activarlo a mitad de proyecto.

## Fuente de verdad

El repositorio `npiobject-labs/openrouter`, rama `main`, es la **única** fuente de verdad, tanto para el código como para la documentación de `docs/planificacion/`. Todo lo que importe vive aquí y se edita aquí.

Google Drive es **opcional** y, cuando está configurado, **solo un destino de copias**, nunca un origen:

- Si el id de la sección **Parámetros** está vacío, este proyecto no usa Drive: omite el paso sin comentarlo.
- Si hay id, al cerrar sesión se suben copias de `docs/planificacion/` a esa carpeta. Solo crear o sobrescribir por nombre: nunca borrar ni renombrar nada en Drive.
- Nunca se toma nada de Drive como origen ni se importa contenido desde allí. Si el repo y Drive difieren, gana el repo.
- La carpeta tiene que ser una carpeta normal de `Mi unidad`. Nunca uses el "Proyecto" de Drive del mismo nombre: el conector no puede escribir en él.

La carpeta local del PC es un espejo de solo lectura. Nunca la trates como origen ni construyas un camino local → nube.

## URLs vivas

| Qué | URL | Despliegue |
|---|---|---|
| Mock estático (Pages) | https://npiobject-labs.github.io/openrouter/ | `.github/workflows/pages.yml` en push a `main` |
| Bitácora (Pages) | https://npiobject-labs.github.io/openrouter/bitacora.html | idem; el índice lo genera `pages.yml` |
| Backend (Fly.io, opcional) | `https://<app de Fly>.fly.dev/` · `/salud` · `/holamundo` | `.github/workflows/deploy.yml` en push a `main` que toque `app/**` |
| Comprobación del backend (Pages) | https://npiobject-labs.github.io/openrouter/holamundo.html | página estática que llama a `/holamundo` y `/salud` desde el navegador |
| Consola del servicio (Pages) | https://npiobject-labs.github.io/openrouter/consola.html | cliente de referencia; pide la clave de servicio y llama a `/v1/...` |
| Documentación de la API (Pages) | https://npiobject-labs.github.io/openrouter/api.html | contrato para apps consumidoras; lo genera `docs/openapi.json` |

Pages está siempre activo. Fly también: `FLY_API_TOKEN` es un secreto de la organización `npiobject-labs` y lo heredan sus repos **públicos**, así que `deploy.yml` despliega sin configurar nada. Si el repo fuera privado (plan Free) o viviera fuera de la organización, el secreto no llega y `deploy.yml` termina en verde con el aviso "Fly no configurado" sin desplegar nada.

## Código

- Todo cambio termina en commit + push a `main`. Mensajes de commit en español, imperativo.
- Backend en `app/` (Rust, axum + tokio + reqwest). `GET /` devuelve texto plano; `GET /salud` devuelve `{"ok":true,"build":"<BUILD_ID>",...}`, donde `BUILD_ID` es el SHA que inyecta el workflow, más si hay clave de OpenRouter y de servicio configuradas.
- El contrato del servicio vive bajo `/v1/`, compatible con la API de OpenAI donde hay equivalente. Publicada una ruta, no se rompe: lo incompatible iría a `/v2/`. Hoy existen `GET /v1/estado` (consulta la clave en OpenRouter, no gasta crédito), `GET /v1/models` (catálogo con caché de una hora), `POST /v1/chat/completions` (proxy fino, sin streaming hasta la etapa 7) las de uso (`GET /v1/uso`, `GET /v1/uso/{id}`, `GET /v1/uso/resumen` y `GET /v1/uso/exportar`) las de aplicaciones (`POST /v1/apps`, `GET /v1/apps` y `DELETE /v1/apps/{id}`) y las de presupuesto (`GET /v1/presupuesto` y `PUT /v1/apps/{id}/presupuesto`).
- **Todo `/v1` exige `Authorization: Bearer <clave>`**, y sin `SERVICIO_CLAVE` configurada el servicio responde `503` a todo `/v1`: un proxy abierto en una URL pública es crédito regalado. Desde la etapa 5 hay dos tipos de clave: `SERVICIO_CLAVE` es la **de administración**, la única que entra en `/v1/apps` y la única que ve el gasto de todas; las **de aplicación** se dan de alta en `POST /v1/apps`, se revocan con `DELETE /v1/apps/{id}` sin tocar el despliegue, y solo ven lo suyo. La clave de OpenRouter (`OPENROUTER_API_KEY`) solo la conoce `app/src/openrouter.rs` y nunca sale del backend.
- **Todo error sale con el mismo sobre**, venga del servicio o de OpenRouter: `{"ok":false,"error":{"message","code","type"}}`. `message`, `code` y `type` van en inglés porque es lo que leen sin adaptaciones los clientes de la API de OpenAI; `code` es nuestro código estable en español (`sin_clave`, `clave_invalida`, `openrouter_rechaza`, `ruta_desconocida`, `sin_configurar`...) y `type` se deduce del estado HTTP. Cuando rechaza OpenRouter se conserva su código de estado y su cuerpo original en `error.upstream`. `deploy.yml` lo verifica en cada despliegue y falla si el sobre cambia.
- **Toda llamada a `/v1/chat/completions` queda medida**, salga bien o mal: tokens, coste, latencia, proveedor, modelo servido y motivo de fin van al histórico (`app/src/uso.rs`), que desde la etapa 4 es una base SQLite en el volumen de Fly montado en `/datos`. El id del registro vuelve en la cabecera `X-Uso-Id`, que va en `expose_headers` del CORS junto a `X-Cache`. El coste sale primero estimado con los precios del catálogo y una tarea en segundo plano lo reconcilia con `GET /generation` de OpenRouter; `coste_origen` dice cuál de los dos es.
- **El histórico vive en un volumen de Fly**, `datos`, montado en `/datos`, con SQLite en modo WAL y una sola conexión bajo mutex. El volumen lo crea `deploy.yml` si no existe, comprobando antes que no esté: `flyctl volumes create` no es idempotente y cada despliegue dejaría uno suelto. Obliga a una sola máquina, que es lo que ya hace `--ha=false`. Si el fichero no se puede abrir, el servicio avisa por el log y sigue en memoria en vez de caerse; `GET /v1/estado` lo delata con `"almacen":"memoria"` y `deploy.yml` falla el run. `BD_RUTA` cambia el fichero, y solo se usa para probar en el PC.
- **De cada clave de aplicación solo se guarda su hash** (`app/src/apps.rs`, SHA-256): la clave se enseña una sola vez al crearla y no hay forma de recuperarla. Dar de baja **desactiva, no borra**, porque el histórico de uso referencia la aplicación y un informe del mes pasado tiene que poder decir quién gastó. La clave se genera con `randomblob` del propio SQLite, para no añadir una dependencia solo por esto. `deploy.yml` comprueba en cada despliegue que `GET /v1/apps` no devuelve ni claves ni hashes.
- **Cada llamada se anota con su aplicación**, y la cabecera opcional `X-Operacion` guarda el trabajo de negocio al que pertenece, para agrupar después varias llamadas de un mismo presupuesto. Con clave de aplicación, `/v1/uso` y sus rutas hermanas solo devuelven lo de esa aplicación, y un registro ajeno responde como si no existiera; con la de administración, `?app=` filtra y `agrupar=app` reparte el gasto.
- **Cada aplicación puede llevar topes** (`app/src/guardia.rs`): presupuesto en dólares por día o por mes con umbral de aviso y de corte, y cuota de llamadas por minuto. Antes de salir hacia OpenRouter se comprueban en este orden, del más barato al más caro de evaluar: el cortacircuitos de bucles, la cuota y el presupuesto. Pasado el aviso, la respuesta lleva la cabecera `X-Presupuesto` sin cortar nada; pasado el corte, `402 presupuesto_agotado`. **La clave de administración no tiene topes**: ponérselos sería encerrarse fuera de casa, y `deploy.yml` lo comprueba en cada despliegue.
- **El cortacircuitos mira la huella del cuerpo**, no el ritmo: la misma consulta repetida cinco veces en un minuto se corta con `429 bucle`, mientras que cinco consultas distintas pasan. Un bucle repite; un uso intenso, no. La huella es el hash del cuerpo ya completado, así que dos peticiones sin `model` cuentan como la misma.
- Secretos y variables del backend: `OPENROUTER_API_KEY` y `SERVICIO_CLAVE` (secretos de repositorio, los vuelca `deploy.yml` a Fly), `MODELO_DEFECTO` (variable de repositorio, opcional), y `OPENROUTER_BASE` y `BD_RUTA` (solo para pruebas locales).
- `GET /holamundo` devuelve `holamundo` en texto plano; `/holamundo` y `/salud` llevan `Access-Control-Allow-Origin: *` porque los consume `docs/holamundo.html` desde Pages (otro origen). Si añades más rutas para el frontend, ponles la misma cabecera. `deploy.yml` verifica las dos rutas y falla si cambian.
- `GET /v1/models` da el formato de la API de OpenAI más precio en dólares por millón, contexto, modalidades y si admite herramientas y salida estructurada. Filtros: `texto`, `proveedor`, `contexto_min`, `gratis=1`; con `refrescar=1` se salta la caché. La cabecera `X-Cache` dice de dónde salió la lista (`miss`, `hit`, `stale`), y va en `expose_headers` del CORS para que el navegador pueda leerla. Si OpenRouter no responde se sirve la copia caducada antes que un error.
- **`docs/openapi.json` es el contrato publicado** y la fuente única de la documentación: `docs/api.html` no repite nada, lo lee y se pinta sola. Toda etapa que publique, cambie o retire una ruta actualiza ese JSON en el mismo commit, sube `info.version` a la etapa entregada y mueve la etapa de `x-etapas` a `estado: "entregada"`. Las rutas que aún no existen viven solo en `x-etapas`, nunca en `paths`: quien genere un cliente desde el contrato debe obtener únicamente lo que ya responde. Los códigos de error se listan en `x-codigos-de-error`.
- `docs/consola.html` es el cliente de referencia del servicio y crece con cada etapa. `docs/index.html` es el mock de todas las etapas, no llama a nada.
- `docs/holamundo.html` toma el nombre de la app de Fly del `<meta name="fly-app">` (`<repo>-<owner>`, como lo deriva `deploy.yml`). Si el proyecto define `FLY_APP` con otro nombre, actualiza ese `content` en el mismo commit.
- `app/fly.toml` no lleva clave `app`: el nombre se pasa con `--app` desde `deploy.yml`.
- El backend escucha en 8080, que es lo que espera Fly; la variable de entorno `PUERTO` solo la usa `tools/arrancar.ps1` para probar en el PC.
- Mocks estáticos en `docs/`. `docs/index.html` es el mock vivo; los anteriores se archivan en `docs/mocks/NNN-nombre.html`.
- El índice `docs/mocks/index.html` lo genera `pages.yml` en cada publicación, leyendo el `<title>` y el `<meta name="build">` de cada mock archivado. No lo edites ni lo commitees: está en `.gitignore`.
- Cada mock lleva `<meta name="build" content="OP-B1-AAAAMMDD-NNN">` con un número nuevo en cada iteración.
- Nunca pongas claves, endpoints internos ni datos reales en `docs/`: el sitio es público.

## Documentación

- Cada documento de planificación, decisión o resumen de sesión se escribe en `docs/planificacion/` de este repo, y solo ahí se edita.
- Si existe `docs/plantilla/`, es el historial de la plantilla de origen que apartó `init-plantilla.yml`: referencia de solo lectura, nunca se edita ni se mezcla con `docs/planificacion/`.
- Si hay id de Drive en **Parámetros**, al cerrar sesión se sube copia como fichero, sin conversión a formato Google (`disableConversionToGoogleType=true`), tanto `.md` como `.html/.png/.svg`.
- No hay edición incremental en Drive: se vuelve a subir el fichero completo con el mismo nombre, o con sufijo de versión (`-v2`, `-v3`) si quieres conservar la copia anterior.

## Verificación antes de avisar

**El sandbox de la sesión no alcanza Pages, Fly ni el VPS**: `curl` a `*.github.io`, `*.fly.dev` o al VPS devuelve `CONNECT tunnel failed, response 403`. Tampoco hay daemon de Docker. Por eso **la verificación de un despliegue la hace siempre un workflow**, que corre en el runner de GitHub y sí tiene salida a internet:

- `pages.yml` da por bueno el despliegue con el paso `deploy-pages`, **pero eso solo prueba que el artefacto se subió**, no que se esté sirviendo. Si tras el push aparece además un run `pages build and deployment` con un paso `Build with Jekyll`, el **Source** de Pages sigue en «Deploy from a branch»: el sitio sirve la raíz del repo (README en `/`, `docs/` colgando de `/docs/`) y los runs de `pages.yml` salen verdes sin efecto. Comprobarlo es parte de la verificación; el arreglo es manual, en Settings.
- `deploy.yml` tiene pasos finales que hacen `curl` a `/salud` (falla si la respuesta no contiene el SHA del commit), a `/holamundo` y a `/v1/estado` y a `/v1/models` con la clave de servicio. Los dos últimos solo corren si están los dos secretos; comprueban la conexión real con OpenRouter sin gastar crédito.

No anuncies "puedes probarlo" hasta confirmar por la API de GitHub Actions que el run del workflow para el SHA que acabas de enviar está en `success`. Si en 5 minutos no está, avisa del fallo con la causa leída en los logs, no del éxito. Al avisar, da siempre: SHA, URL y número de `build`.

Si necesitas comprobar algo desde la sesión, hazlo contra la API de GitHub (`https://api.github.com/repos/npiobject-labs/openrouter/actions/runs/...`), que sí es accesible.

`pages.yml` solo se puede validar en `main`: el entorno `github-pages` únicamente despliega desde la rama por defecto, así que un `workflow_dispatch` sobre una rama de trabajo no sirve de verificación. `deploy.yml` sí acepta cualquier rama.

## Despliegue

- Estático: GitHub Pages vía `.github/workflows/pages.yml` (push a `main` publica `docs/`). Requiere **Settings → Pages → Source: GitHub Actions** una vez a mano. Se aplica también a este repo: el sitio de la plantilla estuvo sirviendo el README hasta que se hizo.
- Backend (opcional): Fly.io vía `.github/workflows/deploy.yml`. `FLY_API_TOKEN` **llega heredado de la organización `npiobject-labs`** (secreto de organización, repos públicos); no hay que crear ni guardar ningún token por proyecto. Nunca lo imprimas en los logs.
- Si el proyecto usa además un VPS con rama `release`, solo tocas `release` cuando el usuario lo pida explícitamente.
- No intentes SSH, scp, rsync ni curl al VPS, a Fly ni a `*.github.io` desde la sesión: el sandbox los bloquea.

## Aterrizaje en el PC

- Solo a petición y solo con Claude Desktop conectado: `tools/aterrizar.ps1` (idempotente, sobrescribe la copia local sin preguntar). "¿Estoy al día?" = `tools/estado.ps1`. Ambos aceptan `-Proyecto`, `-Owner`, `-Remote`, `-Root` y `-Rama`.
- `tools/arrancar.ps1` levanta la app entera en el PC sin tocar la nube: compila el backend, lo sirve en `localhost:8080` y publica `docs/` en `localhost:8081`. Acepta `-PuertoApi`, `-PuertoWeb`, `-Release` y `-SinNavegador`. Necesita Rust; no necesita Docker.
- `tools/probar-bom.ps1` prueba el servicio con un fichero de materiales real: lee `.xlsx` sin Excel ni módulos, manda solo la cabecera y unas filas, pide el mapeo de columnas con salida estructurada y enseña lo que costó la llamada. `-SoloMuestra` enseña lo que se enviaría sin gastar crédito, y `-Comparar` mide el mismo BOM contra tres escalones de precio del catálogo, porque la calidad del mapeo depende del modelo y un modelo pequeño rellena nulos con toda la confianza. Guía en `docs/planificacion/prueba-bom.md`.
- `tools/apps.ps1` gestiona las aplicaciones desde el PC con la clave de administración: sin argumentos lista, `-Crear <nombre>` da de alta y enseña la clave una sola vez, `-Baja <id>` la revoca pidiendo que escribas el id, y `-Gasto` reparte el consumo por aplicación con su nombre en vez del identificador.
- `tools/eliminar.ps1` borra el proyecto entero: app de Fly, repositorio y copia local. Sin `-Confirmar` solo enseña el plan; con él pide escribir el nombre. Drive y las sesiones quedan a mano. Solo se ejecuta si el usuario lo pide explícitamente.
- Servidas desde `localhost`, las páginas de `docs/` llaman al backend local en vez de al de Fly, tomando el puerto de `?api=` (8080 por defecto). En Pages no cambia nada.

## Cierre de sesión

- Termina cada sesión con un resumen de 5 líneas (qué cambió, SHA, URL para probar, resultado en Drive, qué falta), guárdalo en el repo en `docs/planificacion/sesiones/AAAAMMDD-HHMM.md` y, si hay id de Drive, sube copia a Drive en `sesiones/`.
- Además, entrada nueva en `docs/bitacora/AAAAMMDD-HHMM.json` (ver sección **Bitácora**).

## Bitácora

Página pública: https://npiobject-labs.github.io/openrouter/bitacora.html · formato en `docs/bitacora/README.md`.

BITACORA: al cerrar sesión, además del resumen en docs/planificacion/sesiones/,
crea SIEMPRE un fichero nuevo docs/bitacora/AAAAMMDD-HHMM.json. Nunca edites ni
borres entradas anteriores, y nunca toques docs/bitacora.html ni
docs/bitacora/index.json (lo genera el workflow de Pages).

Campos: fecha (ISO con zona), titulo, objetivo, prompts (array de objetos con
texto y nota opcional), cambios (array), sha, sha_completo, run (id del run de
Actions), mock (ruta relativa al mock archivado de esa sesión), build, pagina,
fly (URL de /salud o null), pendiente (array), enlaces (array de {texto,url}),
notas. Obligatorios: fecha y titulo; el workflow falla si faltan.

Los prompts son una transcripción fiel de lo que pidió el usuario en esa sesión,
en sus términos, no un resumen de lo que hiciste. Si la sesión fue larga y la
transcripción es aproximada, dilo en el campo notas.

docs/ es público: nunca copies a la bitácora prompts que contengan claves,
rutas internas, datos personales o nombres de clientes. Si un prompt los
contiene, resúmelo en su lugar y anótalo en notas.
