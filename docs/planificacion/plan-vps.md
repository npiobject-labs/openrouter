# Plan — el servicio openrouter en el VPS, como API para otras aplicaciones

Fecha: 2026-09-13. Estado del servicio al redactar: etapas 1 a 6 entregadas
(`docs/openapi.json` en `0.6.0`), desplegado en Fly.io desde `main`. Primera
aplicación consumidora prevista: **GestiónPresupuestos**.

Objetivo: que el mismo backend de `app/` corra en el VPS del usuario, bajo un
subdominio propio con TLS, y que otras aplicaciones lo usen como API con las
mismas garantías que hoy da Fly (o mejores), sin que nada de lo publicado en
`/v1/` cambie.

Cada fase termina con un criterio de hecho verificable por un workflow, como el
resto del proyecto. **Estado (2026-09-13)**: fases 0, 1 y 2 entregadas
([#25](https://github.com/npiobject-labs/openrouter/pull/25),
[#26](https://github.com/npiobject-labs/openrouter/pull/26)); fase 3 ejecutada:
`vps-preparar.yml` en verde (run
[34764544489](https://github.com/npiobject-labs/openrouter/actions/runs/34764544489))
y `deploy-vps.yml` en verde (run
[34764606970](https://github.com/npiobject-labs/openrouter/actions/runs/34764606970),
SHA `87d8850`), con `https://apisor.oracle402.com/salud` respondiendo el SHA
con certificado de Let's Encrypt. Le faltan los secretos
`VPS_OPENROUTER_API_KEY` y `VPS_SERVICIO_CLAVE`: hasta que existan y se
relance el workflow, `/v1` responde `503 sin_configurar`, que es lo previsto.
La fase 4 tiene el código y la guía, y le falta el alta real de la aplicación
desde el PC; la 5 tiene el runbook y las copias automáticas (timer activo), y
le faltan la prueba de restauración y el monitor externo. Fly sigue como
previsualización de `main` (opción A de la sección 7). Lección de la fase 3:
el `Caddyfile` importaba `sites.d/*.caddy` con comodín y la inspección lo
enseñó expandido; la primera preparación añadió un `import` duplicado que
`caddy validate` rechazó sin que el run se pusiera en rojo (faltaba
`pipefail` tras el `tee`). Arreglado en #26 antes de desplegar.

## 0. Lo que ya se sabe y lo que se supone

### Lo que ya se sabe

- El backend es un único binario Rust (axum), escucha en 8080, guarda el
  histórico en SQLite (`/datos/uso.db`, WAL, una conexión bajo mutex) y ya
  trae lo que una API pública necesita: clave obligatoria en todo `/v1`,
  claves por aplicación con solo el hash guardado, presupuestos con aviso y
  corte, cuota por minuto, cortacircuitos de bucles, sobre de error único,
  CORS con `expose_headers`.
- El `Dockerfile` de `app/` ya construye una imagen de dos etapas que corre
  sin root (uid 10001) y con `/datos` como punto de montaje. Es la misma que
  irá al VPS: no se mantiene una segunda forma de construir.
- **El sandbox de las sesiones no llega al VPS** (ni por SSH ni por HTTPS), y
  tampoco tiene daemon de Docker. Todo lo que toque el VPS lo hace un workflow
  de GitHub Actions, que sí tiene salida a internet. Esto no es una limitación
  a esquivar: es la misma regla que ya cumple `deploy.yml` con Fly y la que
  hace que cada despliegue quede verificado y anotado en un run.
- `CLAUDE.md` ya reserva la rama `release` para el VPS: *"solo tocas `release`
  cuando el usuario lo pida explícitamente"*. El plan la usa tal cual.
- `SERVICIO_CLAVE` de Fly está pendiente de rotar desde la sesión del 12 de
  septiembre (quedó pegada en una conversación). El VPS **no hereda esa
  clave**: nace con una nueva.

### Lo que se supone, y su plan B

- **Confirmado por la inspección (2026-09-13)**: Ubuntu 24.04.4 LTS, Docker
  29.6 con Compose 5.3 ya instalados y en uso, 12 GB de RAM, 164 GB libres,
  NTP activo, `ufw` activo con solo SSH, 80 y 443, y fail2ban activo. El
  usuario de despliegue ya entra por clave y tiene `sudo` sin contraseña.
  Desaparece el plan B del binario con systemd: Docker está y funciona.
- **El VPS ya tiene un Caddy sirviendo otros dominios**: confirmado. Caddy
  v2.11.4 como servicio de systemd en el host, escuchando en 80 y 443 con su
  API de administración en `127.0.0.1:2019`, con 18 hosts cargados entre
  `oracle402.com` y `agentsats.org` y sus certificados emitidos. Eso cambia una cosa importante:
  **no hace falta ningún puerto público nuevo**. Caddy ya escucha en 80 y
  443 para todos sus dominios y reparte por nombre de host; el subdominio
  `apisor.oracle402.com` es un bloque de sitio más en su configuración, con
  un `reverse_proxy` al backend. Lo que sí hay que elegir es el **puerto
  interno** en el que el contenedor del backend escucha en `127.0.0.1`,
  porque el 8080 puede estar ocupado por otro servicio; `vps/inspeccionar.sh`
  lista los ocupados y propone el primer libre. El `compose` del proyecto
  **no lleva Caddy**: un segundo Caddy pelearía por 80/443 con el que ya está.
  Corre en el host, no en Docker, así que el backend se alcanza por
  `127.0.0.1`. La segunda pasada de la inspección enseñó los `import`
  expandidos (un fichero por sitio en `/etc/caddy/sites.d/`), y la primera
  preparación descubrió lo que había detrás: **un solo
  `import /etc/caddy/sites.d/*.caddy` con comodín**. Seguimos esa convención:
  nuestro sitio es `/etc/caddy/sites.d/apisor.oracle402.com.caddy` y con eso
  ya está cargado; `preparar.sh` lo comprueba en la configuración adaptada
  (`caddy adapt`) y solo añadiría una línea `import` si el dominio no
  apareciera. Nada del `Caddyfile` se toca, y `caddy validate` corre antes
  de cualquier `reload`. Todos los demás
  servicios del VPS siguen el mismo patrón que vamos a usar: contenedor
  publicado solo en `127.0.0.1:<puerto>` y Caddy delante.
- **`apisor.oracle402.com` ya resuelve a la IP pública del VPS**, sin proxy
  delante: confirmado por la inspección desde la propia máquina. Los otros
  dominios de ese Caddy obtienen certificado sin problema, así que el nuestro
  también lo hará.
- **[SUPUESTO] GestiónPresupuestos llamará al servicio desde su propio
  backend, nunca desde el navegador.** Es lo que permite que la clave de
  aplicación no salga de un servidor. Si llamara desde el navegador, la clave
  quedaría a la vista de cualquier usuario de esa aplicación y habría que
  añadir una capa (un endpoint propio de GestiónPresupuestos que reenvíe). Se
  trata en la sección 6.
- **[SUPUESTO] Basta con una sola máquina y con cortes de unos segundos al
  desplegar.** SQLite en un fichero obliga a una sola instancia escribiendo,
  igual que en Fly (`--ha=false`). Un despliegue es «parar el contenedor
  viejo, arrancar el nuevo»: entre dos y cinco segundos sin servicio, en los
  que Caddy devuelve `502`. Para el uso previsto (llamadas de segundos, sin
  streaming aún) es aceptable, y el cliente reintenta. Si algún día no lo
  fuera, el camino es Postgres y dos réplicas, no blue/green sobre SQLite.

## 1. Arquitectura en el VPS

```
Internet ──443──▶ Caddy que YA existe en el VPS (TLS automático para todos
                  sus dominios; apisor.oracle402.com es un sitio más)
                     │ reverse_proxy 127.0.0.1:<PUERTO_INTERNO>
                     ▼
              openrouter-backend (contenedor, uid 10001, sin root,
              fs de solo lectura, publicado SOLO en 127.0.0.1:<PUERTO_INTERNO>)
                     │
                     ▼
              /srv/openrouter/datos/uso.db  (bind mount, SQLite WAL)
                     │
                     ▼
              /srv/openrouter/copias/       (copia diaria, rotación 30 días)
```

- **Un `compose.yml`** con un solo servicio, `openrouter`. Publica el puerto
  como `127.0.0.1:${PUERTO_INTERNO}:8080`: alcanzable desde el Caddy del host
  y desde nada más. `PUERTO_INTERNO` sale de la inspección y va en el `.env`.
- **El sitio en Caddy** es un fichero propio, `vps/apisor.caddy`, que
  `desplegar.sh` copia a `/etc/caddy/sites.d/apisor.oracle402.com.caddy`
  (la convención del VPS: un fichero por sitio en una carpeta que el
  `Caddyfile` importa con comodín), valida con `caddy validate` y recarga con
  `caddy reload` (sin corte: Caddy recarga en caliente). El `Caddyfile` no se
  toca.
- **Imagen del backend**: la construye el runner de GitHub con el `Dockerfile`
  de `app/` (caché de capas entre runs) y la **transfiere por SSH** con
  `docker save | gzip | ssh 'docker load'`, unos 30 MB. Sin registro de por
  medio: ni GHCR ni credenciales de pull en el VPS, y la imagen que corre es
  exactamente la que el workflow construyó y verificó. Compilar Rust en el
  VPS queda descartado. Las cinco últimas imágenes se conservan en el VPS
  para el rollback.
- **Secretos**: en `/srv/openrouter/.env`, propiedad de root, permisos `600`,
  escrito por el workflow desde los secretos de repositorio (igual que hoy
  `deploy.yml` los vuelca a Fly). Nunca se copian al chat, nunca al repo.
- **Datos**: bind mount `/srv/openrouter/datos` → `/datos`, con dueño uid
  10001. Un bind mount y no un volumen con nombre para que la copia de
  seguridad y la restauración sean un `cp` que cualquiera entiende.
- **Rollback**: `IMAGEN_TAG=<sha anterior>` en el `.env` y `compose up -d`.
  Tarda lo que tarda el `pull`. Si el cambio incluía un cambio de esquema en
  SQLite, la base ya migrada sigue sirviendo a la versión anterior mientras
  las migraciones sean aditivas (hoy lo son: `CREATE TABLE IF NOT EXISTS` y
  `ALTER TABLE ADD COLUMN` sin borrar nada). Esa regla pasa a ser explícita.

### Dónde vive cada cosa en el repo

```
vps/
  inspeccionar.sh      # solo lectura: proxy en 80/443, dominios, puerto interno libre, DNS (HECHO)
  compose.yml          # un servicio, publicado solo en 127.0.0.1:<PUERTO_INTERNO>, límites, healthcheck
  apisor.caddy         # el sitio; va a /etc/caddy/sites.d/apisor.oracle402.com.caddy, importado por nombre
  preparar.sh          # idempotente: comprueba Docker/ufw/fail2ban/80-443, carpetas, sqlite3, sitio en Caddy, timer de copias
  desplegar.sh         # tag de la imagen, sitio de Caddy validado y recargado, compose up --wait
  copia.sh             # copia diaria de la base con rotación; lo lanza un timer de systemd
  openrouter-copia.service / openrouter-copia.timer
tools/
  verificar-servicio.sh  # las comprobaciones de deploy.yml extraídas, con la URL base como argumento
.github/workflows/
  vps-inspeccionar.yml # workflow_dispatch, solo lectura; el informe sale en el resumen del run (HECHO)
  vps-preparar.yml     # workflow_dispatch, una vez (y cuando cambie preparar.sh)
  deploy-vps.yml       # push a release + workflow_dispatch con sha para rollback
```

`deploy.yml` (Fly) no se toca salvo para llamar a `tools/verificar-servicio.sh`
en vez de repetir las comprobaciones. Las dos verificaciones tienen que ser la
misma o un día dirán cosas distintas.

## 2. Seguridad

Ordenado de fuera hacia dentro. Cada punto dice quién lo aplica.

### 2.1 Máquina

- **SSH**: solo clave, sin contraseña, sin login de root. `preparar.sh` lo
  fija en `sshd_config` (`PasswordAuthentication no`,
  `PermitRootLogin prohibit-password` o `no` si el usuario con sudo es otro).
  Se aplica al final del script, tras comprobar que la clave del usuario
  funciona, para no dejar fuera a nadie.
- **Cortafuegos**: `ufw` ya está activo con solo SSH, 80 y 443, que es
  justo la política del plan; `preparar.sh` solo lo comprueba y no añade
  reglas (el backend no publica ningún puerto fuera de `127.0.0.1`).
- **fail2ban** ya está activo. `preparar.sh` lo comprueba. Nada más: la API
  responde `401` barato a lo no autenticado, y meter fail2ban en los logs de
  Caddy es más fragilidad que protección para el tráfico previsto.
- **Actualizaciones**: `unattended-upgrades` solo para parches de seguridad.
  Docker no se actualiza solo; se anota como tarea mensual en el runbook.
- **Usuario de despliegue**: el que el usuario ya usa para operar el VPS,
  con sudo sin contraseña para todo. **Decidido por el usuario el
  2026-09-13**: se sigue con él, aceptando que la clave SSH guardada en
  GitHub equivale a root en ese VPS. El plan recomendaba un usuario con
  sudo limitado a `desplegar.sh` y `escribir-env.sh`; queda como mejora
  posible (diez líneas en `preparar.sh` y cambiar `VPS_USUARIO`). Lo que
  acota el riesgo hoy: clave SSH dedicada y revocable, huella del servidor
  fija, y que los workflows solo ejecutan lo que está en el repo.
- **Clave SSH dedicada** al despliegue, generada para esto, distinta de la que
  usa el usuario desde su PC. Se revoca borrando una línea de
  `authorized_keys`.
- **Huella del servidor** fija en el secreto `VPS_HOST_KEY` (el resultado de
  `ssh-keyscan`): el workflow no acepta hosts desconocidos. Sin esto un
  secuestro de DNS recibiría el `.env` entero.

### 2.2 Contenedor

- `user: 10001:10001` (ya lo hace el Dockerfile; en `compose` se repite para
  que quede a la vista).
- `read_only: true` y `tmpfs: /tmp`. El binario no escribe nada fuera de
  `/datos`.
- `security_opt: no-new-privileges:true`, `cap_drop: [ALL]`.
- Límites: `mem_limit: 256m` (lo mismo que en Fly), `pids_limit: 256`.
- `restart: unless-stopped`.
- **Healthcheck** cada 30 s contra `/salud`. La imagen `debian:bookworm-slim`
  no trae `curl` ni `wget`, y añadirlos solo para esto es superficie de
  ataque; en su lugar el binario gana un subcomando `openrouter-backend
  --salud` que hace la petición con el `reqwest` que ya lleva y sale con 0 o
  1. Sirve igual en Fly (`[checks]`) y en el PC.
- Logs: driver `json-file` con `max-size: 10m`, `max-file: 5`. El backend
  no escribe prompts en el log, solo metadatos; eso ya es así y se mantiene
  como regla.
- Sin `docker.sock` montado en ningún sitio. Nada de Watchtower: la
  actualización la dispara el workflow, que es quien verifica.

### 2.3 Borde (el Caddy del VPS)

- Todo lo de esta sección va en `vps/apisor.caddy`, el bloque de sitio de
  `apisor.oracle402.com`; el resto de la configuración de Caddy no es nuestra
  y no se toca.
- TLS automático con Let's Encrypt, renovación sola, como ya hace ese Caddy
  con sus otros dominios. HTTP redirige a HTTPS (Caddy lo hace por defecto).
- `Strict-Transport-Security: max-age=31536000` (sin `preload` ni
  `includeSubDomains`: el dominio tiene otros usos que no son nuestros).
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, y se
  quita la cabecera `Server`.
- `request_body max_size 2MB`: el mismo límite que axum aplica por defecto
  (`DefaultBodyLimit`, 2 MiB), para que lo que no cabe muera en el borde.
- Timeouts del `reverse_proxy` de 130 s, por encima de los 120 s que el
  backend concede a OpenRouter en `/v1/chat/completions`: el que corta tiene
  que ser el backend, que anota el registro de uso con su motivo; si cortara
  Caddy primero, la llamada quedaría sin medir.
- Preparado para la etapa 7: `flush_interval -1` en el `reverse_proxy`, para
  que el SSE no se quede en el búfer del proxy. Cuesta nada hoy y evita
  descubrirlo entonces.
- Solo responde al nombre del subdominio. Una petición a la IP en crudo o a
  otro dominio del VPS nunca llega a nuestro backend.

### 2.4 Servicio

- Nada de lo existente se relaja: `Authorization: Bearer` en todo `/v1`,
  `503` sin `SERVICIO_CLAVE`, hashes, comparación en tiempo constante.
- **CORS configurable**: hoy `allow_origin(Any)`. Sigue siendo el valor por
  defecto (la consola de Pages lo necesita), pero se añade la variable
  opcional `CORS_ORIGENES` (lista separada por comas). En el VPS se fija a
  `https://npiobject-labs.github.io` y, si GestiónPresupuestos llegara a
  llamar desde el navegador, a su origen. Con lista, el preflight de
  cualquier otro origen falla. Es una capa más, no la principal: la principal
  es que la clave no esté en un navegador.
- **Clave de OpenRouter propia del VPS**, creada en el panel de OpenRouter
  **con límite de crédito** (OpenRouter permite fijar un tope por clave). Es
  el tope duro que ningún fallo del servicio puede saltarse, y no es la misma
  que usa Fly: si una se filtra, se revoca sin tocar la otra. El nombre en
  OpenRouter dice de dónde es (`openrouter-vps`).
- **`SERVICIO_CLAVE` nueva** para el VPS, generada con `openssl rand -base64
  32` en el PC del usuario y pegada directamente en el secreto de GitHub.
  Nunca en el chat.
- La clave de administración solo se usa para dar de alta aplicaciones y
  mirar el gasto de todas. Ninguna aplicación la conoce. GestiónPresupuestos
  recibe una clave de aplicación con su presupuesto.
- Rotación de una clave de aplicación sin corte: crear la nueva, cambiarla en
  la aplicación, revocar la vieja. Las tres operaciones ya existen.

### 2.5 Secretos y quién los ve

| Secreto / variable | Dónde | Quién lo lee |
|---|---|---|
| `VPS_HOST`, `VPS_USUARIO` | secretos de repositorio | `vps-inspeccionar.yml`, `vps-preparar.yml`, `deploy-vps.yml` |
| `VPS_PUERTO` | **variable** de repositorio (como secreto enmascararía cada «22» del log) | idem |
| `VPS_SSH_CLAVE` (privada, dedicada) | secreto de repositorio | idem |
| `VPS_HOST_KEY` (salida de `ssh-keyscan`) | secreto de repositorio | idem |
| `VPS_OPENROUTER_API_KEY` | secreto de repositorio | `deploy-vps.yml` → `.env` del VPS |
| `VPS_SERVICIO_CLAVE` | secreto de repositorio | idem |
| `API_DOMINIO` = `apisor.oracle402.com` | variable de repositorio | los workflows del VPS, `docs/` |
| `VPS_PUERTO_INTERNO` = `8081` (el que dio la inspección) | variable de repositorio | `deploy-vps.yml` → `.env` y `apisor.caddy` |
| `MODELO_DEFECTO` | variable de repositorio (ya existe) | idem |

Los secretos del VPS llevan prefijo `VPS_` para no confundirlos con los de Fly,
que siguen existiendo mientras Fly siga.

**Cómo entregar la conexión al VPS**: no por el chat. La sesión no puede usar
una IP ni una clave SSH aunque las tenga, y todo lo que se pega en una
conversación queda escrito. El camino es: en Settings → Secrets and variables
→ Actions del repo, crear los secretos de la tabla; en la sesión solo hace
falta saber que existen, y los workflows los usan sin que nadie los vea.

## 3. Fiabilidad

- **Arranque**: `restart: unless-stopped` y Docker habilitado en systemd. Un
  reinicio del VPS levanta todo solo. `preparar.sh` lo comprueba con un
  `docker info` tras `systemctl enable docker`.
- **Apagado limpio**: hoy el backend solo escucha `ctrl_c` (SIGINT). `docker
  stop` manda SIGTERM y, tras 10 s, SIGKILL: el proceso muere sin cerrar la
  conexión SQLite ni terminar una reconciliación de coste en curso. Se
  añade SIGTERM al `graceful_shutdown` (`tokio::signal::unix`). Es un cambio
  de diez líneas que también beneficia a Fly, que igualmente manda SIGTERM.
- **Healthcheck y espera**: `desplegar.sh` hace `compose up -d --wait`, que
  no devuelve hasta que el healthcheck está en `healthy` o vence el plazo, y
  devuelve error si vence. El workflow falla ahí antes de verificar nada.
- **Verificación desde el runner**, la misma que hoy en Fly y con el mismo
  script: `/salud` con el SHA del commit (que también es el tag de la imagen),
  `/holamundo` con CORS, `/v1/estado` con `"almacen":"sqlite"`, `/v1/models`
  con modelos, `/v1/uso`, `/v1/apps` sin claves ni hashes, `/v1/presupuesto`
  con `administracion:true`, y los dos sobres de error. Si algo falla, el
  workflow **no** hace rollback automático: deja el run en rojo con la causa
  y la decisión la toma quien lo lee, porque un rollback ciego sobre una base
  ya migrada puede ser peor que el fallo.
- **Copias de la base**: `copia.sh` cada noche con `sqlite3 uso.db ".backup
  copias/uso-AAAAMMDD.db"` (copia consistente aunque haya escrituras; con
  WAL no hace falta parar nada), comprime, y borra las de más de 30 días.
  Lo lanza un `copia.timer` de systemd en el host, no un contenedor. Además,
  una vez a la semana, `GET /v1/uso/exportar?formato=csv` desde el mismo
  script como exportación lógica legible sin SQLite.
- **Copia fuera del VPS**: [SUPUESTO] el usuario tiene un destino (otro
  servidor, un bucket, o el propio Drive vía `rclone`). Se deja preparado un
  `rclone copy` opcional que solo corre si existe `/srv/openrouter/rclone.conf`.
  Plan B mientras no exista: la copia se queda en el VPS, y el runbook incluye
  bajarla por `scp` a mano de vez en cuando. El histórico son metadatos de
  uso: perderlo duele, pero no para nada.
- **Prueba de restauración** en la fase 5, no después: restaurar una copia en
  una carpeta aparte y abrirla con `sqlite3` es la única manera de saber que
  la copia sirve.
- **Espacio**: cada registro de uso ocupa medio kilobyte; un millón de
  llamadas son 500 MB. `preparar.sh` exige 5 GB libres y el runbook tiene la
  tarea de mirar `df` con las copias. La purga por antigüedad queda para
  después, cuando haya volumen que purgar.
- **Vigilancia externa**: un monitor gratuito (UptimeRobot, Better Stack o
  healthchecks.io, elige el usuario) que pida `/salud` cada 5 minutos y avise
  por correo. `/salud` no exige clave y es barato. [SUPUESTO] el usuario crea
  la cuenta; no se puede desde la sesión ni conviene automatizarlo.
- **Saldo de OpenRouter**: `GET /v1/estado` ya devuelve el uso y el límite de
  la clave. `copia.sh` lo consulta al terminar (con la clave de
  administración que lee del `.env`) y, si queda menos del 20 % del límite,
  escribe una línea de aviso en el log del sistema y, si hay `rclone`, deja
  un fichero `AVISO-saldo.txt` junto a las copias. Sin infraestructura de
  correo: el aviso de verdad lo da OpenRouter por su cuenta, y el corte duro
  lo da el límite de la clave.
- **Reloj**: `timedatectl` con NTP activo; sin hora correcta Let's Encrypt
  falla y los presupuestos por día se calculan mal.

## 4. Fases

Cada fase es un pull request contra `main`, salvo la promoción a `release`,
que es un merge que solo se hace cuando el usuario lo pide. El orden importa:
las fases 1 y 2 no tocan el VPS y se pueden hacer sin la conexión.

### Fase 0 — Inspección y prerrequisitos (sin código del servicio)

0. **Inspeccionar el VPS antes de decidir nada.** HECHO el 2026-09-13:
   `tools/vps-clave.ps1` creó la clave dedicada, la autorizó y subió los
   secretos, y `vps-inspeccionar.yml` corrió en verde (run
   [34763158868](https://github.com/npiobject-labs/openrouter/actions/runs/34763158868)).
   Resultado en la sección 0. Su informe dice: quién sirve 80/443 y su
   versión, qué dominios atiende ya Caddy y con qué certificados, si Docker
   está y qué contenedores corren, el primer puerto interno libre a partir
   de 8080, si `apisor.oracle402.com` ya resuelve y a qué IP, y el estado de
   `ufw`. El script es de solo lectura y no imprime el interior de ningún
   fichero de configuración. Con ese informe se fijan `VPS_PUERTO_INTERNO`
   y se confirma o se descarta cada supuesto de la sección 0.
1. Registro A de `apisor.oracle402.com`: ya existe y apunta a la máquina.
   Nada que hacer.
2. Crear en OpenRouter una clave nueva con límite de crédito para el VPS.
3. Generar `SERVICIO_CLAVE` nueva en el PC (`openssl rand -base64 32`).
4. Generar la clave SSH de despliegue en el PC (`ssh-keygen -t ed25519 -f
   deploy_openrouter -N ""`) y obtener la huella del servidor
   (`ssh-keyscan -p <puerto> <host>`).
5. Crear los secretos y la variable de la tabla 2.5 en GitHub.
6. Decidir qué pasa con Fly (sección 7). Por defecto: sigue mientras dure la
   transición.

Criterio de hecho: run de `vps-inspeccionar.yml` en `success` con veredicto
DNS «apunta a esta máquina», puerto interno anotado en `VPS_PUERTO_INTERNO`,
y los secretos de la tabla 2.5 creados (la sesión lo comprueba por la API de
GitHub, que lista nombres sin valores).

### Fase 1 — Cambios en el backend que el VPS necesita

- SIGTERM en el apagado limpio.
- Subcomando `--salud` para el healthcheck.
- Variable `CORS_ORIGENES` opcional, con `Any` como valor por defecto.
- El esquema de SQLite anota como regla, en `uso.rs`, que las migraciones
  son solo aditivas (para que el rollback sea siempre posible).
- Pruebas unitarias de lo nuevo donde tenga sentido (el parseo de
  `CORS_ORIGENES`).
- `tools/verificar-servicio.sh <url base>`: los pasos de verificación de
  `deploy.yml` extraídos a un script con `bash` y `jq`, que `deploy.yml`
  pasa a llamar. Se prueba primero contra Fly, en el propio `deploy.yml`, para
  que cuando lo use el VPS ya esté validado.

Criterio de hecho: `deploy.yml` en verde en Fly usando el script, con las
mismas comprobaciones que antes.

### Fase 2 — Infraestructura como ficheros

- `vps/compose.yml`, `vps/Caddyfile`, `vps/preparar.sh`, `vps/desplegar.sh`,
  `vps/escribir-env.sh`, `vps/copia.sh` y las unidades de systemd.
- `.github/workflows/vps-preparar.yml` (`workflow_dispatch`): copia `vps/` a
  `/srv/openrouter/` con el usuario con sudo, ejecuta `preparar.sh` y termina
  con un informe: versión de Docker, estado de `ufw`, qué escucha en 80/443,
  espacio libre, hora del sistema. **Para si en 80/443 hay algo que no es
  Caddy nuestro**, y lo dice.
- `.github/workflows/deploy-vps.yml`: en push a `release` y por
  `workflow_dispatch` con entrada `sha` (para rollback). Pasos: construir la
  imagen con `docker/build-push-action`, transferirla por SSH con `docker
  save | docker load`, escribir el `.env` por SSH (`escribir-env.sh` lee de la
  entrada estándar, así los valores no pasan por argumentos ni por el log),
  ejecutar `desplegar.sh <sha>`, y verificar con
  `tools/verificar-servicio.sh https://<API_DOMINIO>`. Resumen del run con
  SHA, URL y `build`, como `deploy.yml`.
- Validación en la sesión, que sí puede: `docker compose config` sobre el
  `compose.yml` (hay cliente de Docker aunque no daemon; si `compose config`
  necesita daemon, plan B: `python3 -c 'import yaml'`), `caddy validate` si se
  puede descargar el binario, `shellcheck` de los scripts, `yaml.safe_load`
  de los workflows.
- `docs/`: `consola.html`, `holamundo.html` y `api.html` toman la URL base de
  un `<meta name="api-base">` nuevo, que en `main` sigue apuntando a Fly hasta
  la fase 4, y conservan `?api=`. `docs/openapi.json` añade el servidor del
  VPS en `servers` **solo cuando responda** (fase 3), nunca antes.

Criterio de hecho: PR fusionado en `main`, `pages.yml` y `deploy.yml` en verde
(nada del VPS corre todavía porque `release` no se ha tocado).

### Fase 3 — Primer despliegue en el VPS

1. Ejecutar `vps-preparar.yml` a mano. Leer el informe. Si para por algo en
   80/443, decidir (sección 0, segundo supuesto).
2. Con el usuario diciéndolo explícitamente: `release` = `main` (fast-forward)
   y push. Se dispara `deploy-vps.yml`.
3. Leer el run. Si está en `success`: el Caddy del VPS ha emitido el
   certificado del subdominio, el backend responde con el SHA, `/v1/estado` dice `sqlite` y la clave de OpenRouter es
   válida.
4. Actualizar `docs/openapi.json` (`servers`) y el `<meta name="api-base">`
   de `docs/` para que apunten al subdominio, y `CLAUDE.md` (URLs vivas,
   secretos, la sección de despliegue). Esto va en un PR a `main` y luego
   a `release`.

Criterio de hecho: run de `deploy-vps.yml` en `success` para el SHA de
`release`; `https://<subdominio>/salud` devuelve ese SHA (lo dice el run, la
sesión no lo puede pedir). Se avisa con SHA, URL y `build`, como siempre.

### Fase 4 — GestiónPresupuestos como primera aplicación

1. Alta desde el PC: `tools/apps.ps1 -Crear GestionPresupuestos -Api
   https://<subdominio>` (el script gana `-Api`, hoy asume Fly). La clave se
   ve una vez; va directa al gestor de secretos de GestiónPresupuestos.
2. Presupuesto: `PUT /v1/apps/{id}/presupuesto` con un tope mensual bajo al
   principio (por ejemplo 5 $ con aviso al 80 %) y 30 llamadas por minuto.
   Se sube cuando haya datos.
3. Guía de integración, `docs/planificacion/integracion-gestionpresupuestos.md`
   (sección 6 de este documento, ampliada con ejemplos ejecutables).
4. Prueba de punta a punta desde el PC con `tools/probar-bom.ps1 -Api
   https://<subdominio>` y la clave de aplicación (no la de administración):
   demuestra que una aplicación con su clave, su presupuesto y su
   `X-Operacion` hace el trabajo real y queda medida.
5. Mejoras pequeñas del servicio que la integración pide y que hoy faltan:
   - `GET /v1/uso?operacion=<id>` y `agrupar=operacion` en `/v1/uso/resumen`:
     la cabecera `X-Operacion` se guarda desde la etapa 5, pero todavía no se
     puede consultar por ella. Para GestiónPresupuestos es la pregunta
     natural: «¿cuánto ha costado analizar este presupuesto?».
   - Documentar en el contrato que un reintento con el mismo cuerpo cuenta
     para el cortacircuitos (cinco iguales en un minuto → `429 bucle`), y
     cómo debe reintentar una aplicación: con espera creciente y como mucho
     dos veces.

Criterio de hecho: en `/v1/uso/resumen?agrupar=app` aparece
GestiónPresupuestos con llamadas reales, y `GET /v1/presupuesto` con su clave
devuelve su margen.

### Fase 5 — Operación

- Ejecutar a mano `copia.sh` una vez, restaurar la copia en `/tmp` y abrirla.
- Activar el monitor externo sobre `/salud`.
- Runbook en `docs/planificacion/runbook-vps.md`: desplegar, hacer rollback a
  un SHA, rotar cada clave (SSH, OpenRouter, administración, aplicación),
  restaurar una copia, ampliar disco, actualizar Docker, qué mirar cuando
  `/v1/estado` dice `memoria`, qué mirar cuando Caddy no consigue certificado.
- Anotar en `CLAUDE.md` que `release` despliega al VPS y qué verifica.

Criterio de hecho: cada procedimiento del runbook se ha ejecutado al menos
una vez por el workflow o por el usuario, y el resultado está en la bitácora.

### Fase 6 — Decidir Fly

Ver sección 7. No tiene código propio; es una decisión con dos commits
posibles.

## 5. Despliegue y promoción: cómo se usa en el día a día

- `main` sigue siendo la rama de desarrollo, con Pages y (mientras exista) Fly.
- `release` es lo que corre en el VPS. Solo avanza con fast-forward desde
  `main` y solo cuando el usuario lo pide en la sesión («promociona a
  release», «despliega en el VPS»). Nunca se desarrolla en `release`.
- Rollback: `deploy-vps.yml` por `workflow_dispatch` con el SHA anterior. La
  imagen se reconstruye con la caché del runner y se transfiere igual. `release` no se mueve hacia atrás:
  el estado real lo dice `/salud`, y el siguiente fast-forward vuelve a
  desplegar lo último.
- Un run de `deploy-vps.yml` en rojo deja el VPS como estuviera: si falló
  antes de `desplegar.sh`, con la versión anterior; si falló en la
  verificación, con la nueva, y el run dice qué comprobación cayó.
- Cambios solo de `vps/` (un ajuste de Caddy): mismo camino, `main` →
  `release`. `desplegar.sh` copia `vps/` de nuevo en cada despliegue, así que
  el `Caddyfile` que corre es siempre el del SHA desplegado.

## 6. Usabilidad para las aplicaciones consumidoras

Lo que GestiónPresupuestos (y la siguiente) tiene que saber, y que irá en la
guía de integración con ejemplos:

- **URL base**: `https://<subdominio>/v1`. Compatible con cualquier cliente
  de la API de OpenAI: `base_url` a esa URL y `api_key` a la clave de
  aplicación. Con el SDK de Python es un cambio de dos parámetros; con
  `curl`, una cabecera.
- **La clave vive en el servidor de la aplicación**, en su gestor de secretos,
  nunca en código ni en el navegador. Una clave por aplicación y por entorno
  (una para la GestiónPresupuestos de pruebas, otra para la real), con
  presupuestos distintos.
- **`X-Operacion`** en cada llamada con el identificador del presupuesto que
  se está analizando. Es lo que permitirá decir cuánto costó cada uno. Y el
  campo `user` del cuerpo con el usuario de la aplicación, que OpenRouter ya
  entiende.
- **Antes de un trabajo grande**, `GET /v1/presupuesto`: devuelve el margen y
  evita empezar un análisis de cien llamadas que se va a cortar a la mitad.
- **Errores que hay que tratar**, todos con el mismo sobre:
  `401 sin_clave`/`clave_invalida` (configuración), `402 presupuesto_agotado`
  (parar y avisar al usuario, no reintentar), `429 cuota` (esperar el
  `Retry-After`) y `429 bucle` (algo repite lo mismo: parar), `502/504
  openrouter_rechaza` con `error.upstream` (reintentar una o dos veces con
  espera creciente), `503 sin_configurar` (avisar al administrador).
- **Cabeceras de respuesta** que conviene leer: `X-Uso-Id` (guardarlo junto
  al resultado para poder auditar el coste), `X-Presupuesto` (aviso de
  umbral: enseñarlo, no ocultarlo).
- **Salida estructurada**: `response_format` con esquema, `max_tokens` y
  `maxLength` en los campos de texto, y elegir el modelo por la prueba del
  BOM, no por precio. Esto ya está aprendido y documentado en
  `prueba-bom.md`; la guía lo enlaza en vez de repetirlo.
- **Timeouts del cliente**: por encima de 120 s en `/v1/chat/completions`,
  para que quien corte sea el servicio y la llamada quede medida.
- **Ejemplos** en la guía: `curl`, PowerShell (`Invoke-RestMethod`, que es lo
  que ya usan `tools/*.ps1`) y Python con el SDK de OpenAI. [SUPUESTO] no se
  sabe en qué está escrita GestiónPresupuestos; cuando se sepa, se añade su
  lenguaje.
- **Documentación viva**: `api.html` lee `openapi.json`, que ya lista rutas,
  esquemas y `x-codigos-de-error`. Con el servidor del VPS en `servers`, quien
  genere un cliente desde el contrato obtiene la URL correcta.
- **Consola**: `consola.html` con selector de servidor (Fly, VPS, local) en
  vez del `<meta>` fijo, y la clave sigue en `localStorage`. Es la forma de
  probar el VPS a mano desde el móvil sin herramientas.

## 7. Qué hacer con Fly

Dos entornos son dos bases SQLite, dos juegos de claves de aplicación y dos
presupuestos: no se sincronizan ni deben. Por eso hay que decidir, y la
decisión es del usuario:

- **A) Fly sigue como entorno de `main`** (previsualización de cada PR
  fusionado, con su propia clave de OpenRouter con un límite pequeño) y el VPS
  es producción desde `release`. Coste: el de Fly, mínimo con
  `auto_stop_machines`. Ventaja: cada PR se verifica en un despliegue real
  antes de promocionarse. Es la opción por defecto del plan.
- **B) Fly se apaga** cuando la fase 5 esté cerrada: `deploy.yml` se
  desactiva (o se deja en verde sin token, como está previsto), la app de Fly
  se borra y se revoca su clave de OpenRouter. `main` se verifica solo con
  `cargo test` y `docs/`. Menos coste y una clave menos que cuidar; a cambio
  un cambio de `app/` solo se ve corriendo cuando llega a `release`.

En cualquiera de las dos, el histórico de Fly se exporta una vez con
`GET /v1/uso/exportar` y se guarda con las copias del VPS. No se migra a la
base del VPS: son llamadas de otra clave y de otra época.

## 8. Lo que este plan no hace

- No cambia nada de `/v1/`: las rutas publicadas responden igual en Fly, en
  el VPS y en el PC. Lo único nuevo en el contrato son los filtros por
  operación de la fase 4, que son aditivos.
- No monta Postgres, réplicas ni balanceo: SQLite en una máquina es lo que el
  uso previsto necesita, y `--ha=false` ya lo asumía.
- No pone una interfaz de administración: `consola.html`, `tools/apps.ps1` y
  la API son la administración.
- No automatiza el rollback ni la promoción a `release`: los dos son
  decisiones que alguien lee antes de tomar.
- No añade OAuth, JWT ni usuarios: las aplicaciones se identifican con su
  clave y los usuarios finales son cosa de cada aplicación (`user` en el
  cuerpo).
- No adelanta la etapa 7 (streaming) ni la 8 (alias): GestiónPresupuestos
  usa salida estructurada síncrona y no las necesita. Caddy queda preparado
  para el streaming, nada más.

## 9. Riesgos que quedan

| Riesgo | Cómo se ve | Qué se hace |
|---|---|---|
| La clave SSH de despliegue se filtra | Acceso al VPS como el usuario de operación, con sudo | Se revoca borrando una línea de `authorized_keys`; rotación en el runbook; huella fija |
| Un secreto de GitHub se filtra en un log | El log del run lo enseña | GitHub enmascara los secretos; `escribir-env.sh` lee de stdin; nunca `set -x` con secretos |
| El VPS se queda sin disco | `/salud` responde pero `/v1/estado` dice `memoria` o SQLite falla al escribir | `preparar.sh` exige 5 GB; copias con rotación; tarea mensual del runbook |
| Let's Encrypt no emite | Caddy en bucle de reintentos, `deploy-vps.yml` en rojo en `/salud` | La inspección comprueba DNS y 80/443 antes; el runbook cubre el caso Cloudflare |
| Nuestro sitio rompe el Caddy del VPS y tira sus otros dominios | `caddy reload` falla | `desplegar.sh` valida con `caddy validate` antes de recargar y no recarga si falla; un `reload` fallido deja la configuración anterior en pie |
| Fallo de OpenRouter | `502/504 openrouter_rechaza` con `error.upstream` | Ya se conserva el cuerpo original; la aplicación reintenta; sin respaldo fuera de OpenRouter (fuera de alcance, ver plan por etapas) |
| Gasto descontrolado | Presupuesto por app, cuota, cortacircuitos, límite de la clave en OpenRouter | Cuatro capas; la última no depende de nuestro código |
| La sesión no puede ver el VPS | Ningún `curl` ni `ssh` sale del sandbox | Todo pasa por workflows con informe; se acepta como regla, no como problema |

## 10. Orden de trabajo propuesto para las próximas sesiones

0. HECHO: fases 0 a 3 (ver el estado al principio del documento).
1. Usuario: crear `VPS_OPENROUTER_API_KEY` y `VPS_SERVICIO_CLAVE` y relanzar
   `deploy-vps.yml` a mano; el run tiene que llegar a «el servicio habla con
   OpenRouter; histórico en sqlite».
2. Usuario: alta de GestiónPresupuestos con `tools/apps.ps1 -Crear` y su
   presupuesto; prueba con `tools/probar-bom.ps1` contra el VPS.
3. Fase 5: probar una restauración, activar el monitor externo, y decidir
   Fly (sección 7).
