# Plan por fases: "PC arranca, móvil continúa"

Proyecto: DesdeMovil · Fecha: 2026-09-05 · Autor: Cowork (sesión en la nube)
Estado de este documento: fase 0 ejecutada con resultados reales; fases 1–4 planificadas.

Leyenda: **[VERIFICADO]** = probado en esta sesión · **[SUPUESTO]** = no verificado, con plan B.

---

## Fase 0 — Verificación (ejecutada)

| # | Pregunta | Prueba | Resultado |
|---|---|---|---|
| 1 | ¿La sesión en la nube alcanza el subdominio del VPS? | `curl -I https://<subdominio>` | **No probado** (fuera del alcance autorizado: solo Drive + GitHub). Dato relevante: el sandbox sale por un proxy HTTP con lista de destinos; `npiobject.github.io` devolvió `403 Forbidden` en el CONNECT del proxy y `www.googleapis.com` también. **[SUPUESTO] el VPS tampoco será alcanzable** → ver plan B en fase 3. |
| 2 | ¿Alcanza github.com para `git push`? | `git ls-remote`, API `repos/npiobject/DesdeMovil` | **Parcial [VERIFICADO]**: github.com y api.github.com son alcanzables, pero a través de un proxy de GitHub *ligado a la sesión*: solo responde para repositorios **adjuntados a la sesión** ("GitHub access to this repository is not enabled for this session"). El puerto 22 (SSH) está cerrado. El token de sesión (`GITHUB_TOKEN`) solo sirve para endpoints `repos/{owner}/{repo}/...`; `/user/repos` y crear repos no están disponibles. **Consecuencia: el repo debe existir de antemano y seleccionarse al crear la sesión (selector de repositorio bajo la caja de texto); Claude no puede crearlo ni adjuntarlo en caliente.** Además la cuenta GitHub vinculada a Claude era `proyectoagram-hash`, no `npiobject`: hay que reconectar GitHub con `npiobject` o hacer público el repo. |
| 3 | ¿Puede el usuario ajustar la política de red? | Inspección del entorno | La lista de permitidos del proxy (`no_proxy` + destinos del CONNECT) es de plataforma; en plan individual **no hay ajuste** [VERIFICADO por inspección del entorno; el ajuste documentado es de organización]. Plan B: no depender de red saliente a destinos propios; usar conectores (Drive) y el proxy de GitHub. |
| 4 | ¿Funciona escribir en Drive con el PC apagado? | `create_file` (carpeta + `.md` sin conversión) y relectura con `download_file_content` | **[VERIFICADO]** El conector de Drive es un MCP en la nube, no pasa por el PC. Carpeta `DesdeMovil` creada en Mi unidad (`1-0wWhp_-rrSgxKrr0AN34dg_Y2nAPK2J`), fichero `fase0-prueba-escritura.md` subido con `disableConversionToGoogleType=true` y releído byte a byte. Hallazgo: lo que existía con nombre `DesdeMovil` era un **Proyecto de Drive** (mimeType `google-apps.project`), y el conector no puede escribir dentro (`canAddChildren=false`). Usar siempre la carpeta normal. |
| 5 | ¿Se puede lanzar una sesión nueva desde el móvil dentro del proyecto? | Requiere acción del usuario en el móvil | **[SUPUESTO]** Pendiente de que Fran lo pruebe (fase 1, paso 4). Plan B: dejar siempre una sesión "viva" en el proyecto y continuar en ella; las tareas programadas sí arrancan sesiones nuevas sin dispositivo. |

Hallazgo adicional [VERIFICADO]: la carpeta local del PC (`C:\Users\fsant\C - Desarrollo\DesdeMovil`) es accesible desde la sesión en la nube solo mientras Claude Desktop está abierto (bridge `device_bash`). Confirma la restricción de la sección 2 del encargo.

Criterio de éxito de la fase 0 (pendiente de cerrar): `git push` a `npiobject/DesdeMovil` desde la nube y `curl` de la URL de GitHub Pages devolviendo el `index.html` subido. Ver bloque "Resultado del push" al final.

---

## Fase 1 — Cimientos: proyecto Cowork + repo adjunto + Drive

Objetivo: que cualquier sesión del proyecto, arranque desde donde arranque, sepa dónde escribir.

Pasos:
1. Repo `npiobject/DesdeMovil` creado en GitHub (por Fran, una vez), rama `main`, con `README.md` y carpeta `docs/` (GitHub Pages servirá `/docs` o `/`).
2. Adjuntar el repo al **proyecto** de Cowork (no solo a una sesión), para que herede en sesiones nuevas. [SUPUESTO] que el adjunto es a nivel de proyecto; plan B: adjuntarlo al crear cada sesión desde el móvil.
3. Instrucciones del proyecto: pegar el bloque de la sección "Instrucciones del proyecto" de este documento.
4. Prueba desde el móvil: crear una sesión nueva en el proyecto, pedir "escribe `ping.md` en Drive/DesdeMovil y haz push de `docs/ping.txt`". Éxito = ambos ficheros aparecen sin que el PC esté encendido.

Queda funcionando: escritura automática en Drive y en GitHub desde cualquier sesión del proyecto.
Comprobación: paso 4.

## Fase 2 — Mocks estáticos publicados (vía B3, GitHub Pages)

Objetivo: cada iteración de mock se ve en el navegador del móvil en < 2 min.

Pasos:
1. Activar Pages en el repo: Settings → Pages → Deploy from branch `main` / carpeta `/docs` (acción única de Fran; Claude no puede tocar settings del repo con el token de sesión [VERIFICADO: endpoint no repo-scoped]). Alternativa: workflow `actions/deploy-pages` en `.github/workflows/pages.yml`, que Claude sí puede commitear.
2. Convención: `docs/index.html` es el mock vivo; cada mock anterior se archiva en `docs/mocks/NNN-nombre.html`. Cada build lleva un `<meta name="build" content="DM-B3-YYYYMMDD-NNN">`.
3. Verificación obligatoria tras cada push: la sesión hace `curl` a `https://npiobject.github.io/DesdeMovil/` y compara el `build` servido con el enviado. **[VERIFICADO que NO funciona desde el sandbox]**: `npiobject.github.io` está bloqueado por el proxy (403). Plan B en vigor: verificar por la API de GitHub (`GET repos/npiobject/DesdeMovil/pages/builds/latest` → `status: built` y `commit` = SHA del push) o, si el endpoint `pages` no está permitido, por `GET repos/.../commits/main` + `deployments`. Solo entonces avisa: "listo para probar: <URL> build NNN". Si en 5 min no está `built`, avisa del fallo, no del éxito.
4. Copia del mock también a Drive `DesdeMovil/mocks/` (HTML sin conversión) para que quede en la documentación viva.

Queda funcionando: ciclo instrucción → push → verificación → aviso → prueba en móvil.
Comprobación: pedir un cambio visible desde el móvil y verlo en la URL.

Riesgos de seguridad B3: el sitio es público; nunca poner claves, endpoints internos ni datos reales en los mocks. Repo público si Pages es gratuito con el plan de GitHub del usuario; si se hace privado, Pages requiere plan de pago [SUPUESTO].

## Fase 3 — Backend (vía B2: GitHub → VPS hace pull)

Objetivo: el repo no cambia; cambia quién lo sirve.

Pasos:
1. En el VPS (acción de Fran, una vez): clonar el repo, `systemd` timer cada 2 min que hace `git fetch && git reset --hard origin/main` y reinicia el servicio si cambió el HEAD. Elegido **cron/timer, no webhook**: no expone ningún endpoint entrante y no depende de que la nube alcance el VPS (que [SUPUESTO] no alcanza).
2. Verificación desde la nube: como el sandbox no alcanza el VPS, el VPS **escribe su estado de vuelta**: el timer hace commit de `deploy/status.json` (`{sha, hora, ok}`) en una rama `deploy-status`, que Claude sí puede leer por la API del repo. Claude avisa solo cuando `sha` = SHA enviado.
3. Secretos: en el VPS, nunca en el repo. Deploy key de solo lectura para el pull; PAT de scope mínimo (contents:write en ese repo) para el commit de estado.

Riesgos B2: pull automático de `main` = cualquier push despliega. Mitigación: desplegar desde rama `release` que Claude solo toca cuando Fran lo pide explícitamente; firmar commits (la sesión ya firma con clave SSH [VERIFICADO en `git config`]).

B1 (endpoint HTTPS propio) queda descartada: exige código de recepción con autenticación y, además, la nube probablemente no lo alcanza.

## Fase 4 — Aterrizaje en el PC (operación repetible)

Objetivo: al encender el PC, una orden deja `C:\Users\fsant\C - Desarrollo\DesdeMovil` idéntica a la nube.

Regla: nube = fuente de verdad; carpeta local = espejo de solo lectura. No hay camino local → nube.

Diseño:
- `repo/` ← `git clone` la primera vez; después `git fetch origin && git reset --hard origin/main && git clean -fdx`. Idempotente y destructivo por diseño (no hay nada local que preservar).
- `drive/` ← copia de la carpeta Drive `DesdeMovil`. Dos opciones: (a) Google Drive para escritorio apuntando esa carpeta (cero código, sincronización continua); (b) la propia sesión Cowork, con Desktop abierto, lista Drive con el conector y escribe cada fichero vía `device_bash`/`device_commit_files` a `drive/`. Se recomienda (a) por simplicidad; (b) como respaldo si no quiere instalar Drive en el PC.
- `aterrizar.ps1` en el repo (`tools/aterrizar.ps1`): hace el reset del repo, y si existe Drive de escritorio, compara `drive/` con el listado de Drive (nombres + tamaños) e informa.
- "¿Estoy al día?": `tools/estado.ps1` imprime `HEAD local` vs `origin/main` y fecha del último fichero en `drive/`. Desde Cowork basta con "comprueba si mi copia local está al día": la sesión ejecuta `git fetch` + `git status -sb` por `device_bash` y compara con la API.

Comprobación: ejecutar el aterrizaje dos veces seguidas; la segunda debe terminar sin cambios. Borrar un fichero local a mano y volver a aterrizar: debe reaparecer.

---

## Instrucciones del proyecto de Cowork

Ver fichero `instrucciones-proyecto-cowork.md` en esta misma carpeta de Drive (mismo contenido que la sección homónima del plan en el repo y en la carpeta local).

---

## Resultado del push (se rellena al cerrar la fase 0)

Pendiente: requiere sesión con el repo `npiobject/DesdeMovil` seleccionado. Se publicará como `plan-flujo-movil-v2.md`.
