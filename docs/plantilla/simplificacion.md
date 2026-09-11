# DesdeMovil — Simplificación de la plantilla

**Fecha:** 2026-09-06
**Objetivo:** que un proyecto nuevo arranque solo con GitHub (Pages) y **un único paso manual**. Fly y Drive pasan a ser opcionales y se activan a mitad de proyecto sin rehacer nada.

> **Ampliado después.** La sustitución que hace `init-plantilla.yml` se quedaba en el nombre y el owner; [`portado-hijos.md`](portado-hijos.md) la extendió al paquete Rust, al usuario del runtime, a los textos de `app/src/main.rs` y al prefijo del `build`, añadió el apartado de la documentación heredada a `docs/plantilla/` y un paso final que falla el run si queda cualquier residuo. Lo de aquí sigue siendo válido; la lista de ficheros del punto 3 es la corta.

Antes: 4 pasos manuales (crear repo, activar Pages, token de Fly, carpeta de Drive) + pegar un bloque de instrucciones con cuatro valores rellenados a mano.
Después: crear repo desde la plantilla + activar Pages. Nada que rellenar; el resto lo hace un workflow.

## Qué cambió

### 1. Fly opcional — `.github/workflows/deploy.yml`

- Primer paso `Comprobar si Fly esta configurado`: pasa `secrets.FLY_API_TOKEN` a `env` y compara con vacío (los secretos no se pueden usar en `if:` a nivel de job). Sin token, escribe en `$GITHUB_STEP_SUMMARY` el aviso `Fly no configurado: ver ARRANQUE.md, seccion Fly` y el job **termina en verde** sin desplegar. Con token, sigue igual que antes.
- Todos los pasos posteriores llevan `if: steps.fly.outputs.configurado == 'true'`.
- **Nombre de la app**: variable de repositorio `FLY_APP` si existe (tiene prioridad); si no, se deriva de `github.repository` como `<repo>-<owner>` en minúsculas, saneado a `[a-z0-9-]` y recortado a 30 caracteres.
- `app/fly.toml` pierde la clave `app`; el nombre se pasa con `flyctl deploy app --app "$FLY_APP"`. La plantilla ya no arrastra el nombre de otro proyecto.

Derivaciones comprobadas en el sandbox:

| `github.repository` | App |
|---|---|
| `npiobject/DesdeMovil` | `desdemovil-npiobject` |
| `miusuario/Mi-Proyecto` | `mi-proyecto-miusuario` |
| `Empresa_Larga/Un.Proyecto_Con.Nombre.Muy.Largo` | `un-proyecto-con-nombre-muy-lar` |

### 2. Drive opcional — `CLAUDE.md`

- Nueva sección **Parámetros** al principio: proyecto, owner de GitHub, app de Fly (`derivada` o el valor de `FLY_APP`) y carpeta de Drive (id).
- Si la fila del id está vacía, el proyecto no usa Drive y el paso se omite **sin comentarlo**. Si hay id, al cerrar sesión se suben copias de `docs/planificacion/`: solo crear o sobrescribir por nombre, nunca borrar ni renombrar.
- Las secciones «Fuente de verdad», «Documentación» y «Cierre de sesión» quedan condicionadas al id.
- La tabla de URLs vivas ya no cita una app de Fly concreta.

### 3. Inicialización automática — `.github/workflows/init-plantilla.yml`

Dos barreras para que **nunca** corra en la plantilla:

1. `if: github.event.repository.is_template == false` a nivel de job (la plantilla tiene `is_template: true`, verificado por la API).
2. Existencia del marcador `.plantilla-pendiente` en la raíz.

Qué hace: sustituye `DesdeMovil` y `npiobject` por el nombre y el owner reales (de `github.repository`) en `CLAUDE.md`, `README.md`, `ARRANQUE.md`, `docs/index.html`, `docs/guia.html` y `tools/*.ps1`; deja la sección **Parámetros** rellenada; borra el marcador y se borra a sí mismo; commitea con `permissions: contents: write`.

Un commit hecho con `GITHUB_TOKEN` no dispara workflows por push, así que el último paso lanza `pages.yml` por `workflow_dispatch` (`gh workflow run`), que sí está permitido; de ahí el `permissions: actions: write`.

### 4. `ARRANQUE.md` reescrito

Cabe en una pantalla: «Por proyecto (2 pasos)» (enlace directo a *Use this template* + cambiar el desplegable de Pages de «Deploy from a branch» a «GitHub Actions», con el aviso de que si se deja como está el enlace muestra el README y no el mock), «Si a mitad del proyecto necesitas Fly», «Si quieres copias en Drive», «Aterrizar en el PC» y enlace a la guía extendida.

### 5. `README.md`

Diez líneas con el método y los enlaces a `ARRANQUE.md`, `docs/guia.html` y `CLAUDE.md`.

### 6. `tools/aterrizar.ps1` y `tools/estado.ps1`

- `$ErrorActionPreference` no detiene a los ejecutables externos: hay un helper `Assert-Git` que mira `$LASTEXITCODE` tras **cada** llamada a git y aborta con mensaje claro. El resumen final no se imprime nunca si algún git falló.
- `repo\` existe sin `.git`: si está vacía, se clona; si tiene ficheros, se listan, no se toca nada y se sale con error explicando que hay que vaciarla o apartarla. La comprobación está en los dos scripts.
- El aviso de Drive pasa a `drive/: no configurado (opcional)`, sin instrucciones de Google Drive de escritorio.

### 7. `docs/guia.html`

Guía extendida nueva (no existía): la idea, qué trae la plantilla y qué es opcional, ciclo de una sesión, Pages, Fly, Drive, aterrizaje y una tabla de errores frecuentes.

## Verificación en el sandbox

`pwsh` no venía instalado, pero **sí se pudo descargar** el tarball de PowerShell 7.4.6 desde github.com e instalarlo en `/opt/pwsh`. Los scripts se ejecutaron de verdad, no solo se revisaron:

| Caso | Resultado |
|---|---|
| Análisis sintáctico (`Parser::ParseFile`) de los dos scripts | 0 errores |
| `estado.ps1` sin copia local | mensaje + `exit 1` |
| `aterrizar.ps1` con `repo\` no vacía y sin `.git` | lista el contenido, no toca nada, `exit 1` |
| `estado.ps1` con `repo\` no vacía y sin `.git` | ídem, `exit 1` |
| `aterrizar.ps1` con `repo\` vacía | clona, imprime resumen, `exit 0` |
| Segunda pasada con fichero sucio | `reset --hard` + `clean -fdx` lo borran, `exit 0` |
| `estado.ps1` al día | `AL DIA (c7f646e)`, `exit 0` |
| git falla (remoto borrado) | `ERROR - git fetch fallo con codigo 128`, **sin resumen**, `exit 1` |

Además: los tres YAML pasan `yaml.safe_load`, `docs/guia.html` cierra todas las etiquetas, y la sustitución de `init-plantilla.yml` se ensayó sobre una copia del repo con `GITHUB_REPOSITORY=miusuario/Mi-Proyecto` (resultado: cero restos de `DesdeMovil`/`npiobject` salvo el enlace protegido, tabla de Parámetros rellenada, marcador y workflow borrados).

## Decisiones que no venían especificadas

1. **La app de Fly de esta misma plantilla cambia de nombre.** La derivación da `desdemovil-npiobject`, no la app actual `desdemovil-npi`. No hay API de variables de repositorio en la sesión, así que no se podía crear `FLY_APP` desde aquí. Consultado y decidido: adelante con la derivada. `desdemovil-npi` queda huérfana y se borra a mano cuando se quiera; alternativa siempre disponible: definir la variable `FLY_APP` y relanzar `deploy.yml`.
2. **`docs/guia.html` no existía.** Se pedía enlazarla desde `ARRANQUE.md` y `README.md`, así que se ha escrito, y se ha añadido a la lista de ficheros que sustituye `init-plantilla.yml`.
3. **El enlace *Use this template* de `ARRANQUE.md` se protege de la sustitución** (se aparta a un centinela y se restaura después). Si no, el `ARRANQUE.md` de un proyecto generado apuntaría a sí mismo como plantilla en vez de a la original.
4. **El id de Drive no se hereda.** `init-plantilla.yml` vacía la fila «Carpeta de Drive (id)» de `CLAUDE.md`. Así la plantilla conserva el suyo (`1-0wWhp...`, que sigue en uso) y ningún proyecto generado escribe por error en esa carpeta ajena.
5. **[SUPUESTO] Crear un repo desde una plantilla no dispara workflows por push.** Por eso `init-plantilla.yml` lleva también `workflow_dispatch` y `ARRANQUE.md` dice que, si no se lanzó solo, la sesión lo lanza desde Actions. Plan B si el supuesto es correcto y molesta: la sesión de verificación lo dispara, que es exactamente lo que se le pide en el primer mensaje.
   **[FALSO, comprobado el 2026-09-06]** Crear un repo desde la plantilla **sí** dispara los tres workflows: el commit inicial de `PruebaInit` generó los tres runs con `event: push` y `run_number: 1`. El `workflow_dispatch` se queda como red de seguridad. Ver [`prueba-init.md`](prueba-init.md).
6. **Saneado del nombre derivado.** No estaba especificado qué hacer con caracteres no válidos ni con el recorte: se convierte todo lo que no sea `[a-z0-9-]` en `-` y se eliminan los guiones finales que deje `cut`, porque una app de Fly no puede terminar en guion.
7. **La región de Fly ya no está duplicada** en `deploy.yml`: el resumen la lee de `app/fly.toml`, que es donde manda.
8. **`Assert-Git` en vez de una función envoltorio de git.** Un `Invoke-Git -C $Repo fetch` fallaría en el binding de parámetros de PowerShell (`-C` se interpretaría como nombre de parámetro de la función). El helper solo comprueba `$LASTEXITCODE` y las llamadas a git quedan literales.
9. **La comprobación de `repo\` no vacía se añadió también a `estado.ps1`**, no solo a `aterrizar.ps1`: el diagnóstico útil tiene que salir en el script que se ejecuta primero.

## Runs de verificación (SHA `a5e5cd32587a851de95232cdd84070a9aa37cf0c`)

| Workflow | Run | Resultado |
|---|---|---|
| `pages.yml` | [34030354493](https://github.com/npiobject/DesdeMovil/actions/runs/34030354493) | **success** |
| `deploy.yml` | [34030354491](https://github.com/npiobject/DesdeMovil/actions/runs/34030354491) | **success**, vía «Fly configurado» |
| `init-plantilla.yml` | [34030354554](https://github.com/npiobject/DesdeMovil/actions/runs/34030354554) | **skipped** — la barrera `is_template == false` corta el job en la plantilla |

Del log de `deploy.yml`, los diez pasos en `success`:

- `FLY_APP_VAR:` vacío (no hay variable de repositorio) → `FLY_APP: desdemovil-npiobject`, el nombre derivado.
- `flyctl apps create` creó la app; se provisionaron IPs y una máquina en `cdg`.
- `Intento 1/10: {"build":"a5e5cd32587a851de95232cdd84070a9aa37cf0c","ok":true}` → verificación a la primera.

El despliegue completo tardó 30 s, no varios minutos: el builder remoto de Fly tenía en caché todas las capas del Dockerfile de la app anterior de la misma organización (`#14 [builder 6/6] RUN ... cargo build --release` → `CACHED`). En un proyecto nuevo con otra organización de Fly no habrá caché y sí tardará lo que dice `ARRANQUE.md`.

`init-plantilla.yml` aparece en la lista de Actions porque su `on: push` sí se evalúa, pero el job queda en `skipped`: no hay forma de filtrar por `is_template` en el `on:`, y una condición a nivel de job es exactamente lo que se pedía. Una vez que un proyecto generado se inicializa, el workflow ya no existe en ese repo.
