# DesdeMovil — Consolidación como plantilla

**Fecha:** 2026-09-06
**SHA de la consolidación:** `3fc44a79d2e137ea3357f79f9926ee1db1208db0`

Este repo pasa de ser un proyecto concreto a una plantilla reutilizable: "Use this template" → cuatro pasos manuales → una instrucción pegada en una sesión de Code → dos despliegues vivos y verificados.

> **Documento histórico, superado dos veces.** Describe el flujo de cuatro pasos manuales con marcadores `PLANTILLA:` sustituidos a mano. Lo sustituyó [`simplificacion.md`](simplificacion.md) (dos pasos, `init-plantilla.yml` automático) y después [`portado-hijos.md`](portado-hijos.md) (la sustitución cubre también el backend y el prefijo de build, y un paso del workflow falla si queda cualquier residuo). En particular, la tabla de «los diez sitios» de más abajo **ya no describe nada que haya que hacer a mano**, y era justo la trampa que detectó `PruebaPlantilla`: ni el número ni la tabla cuadraban con lo que devolvía el `grep`. Se conserva sin tocar como registro de la consolidación.

## Qué cambió

### `CLAUDE.md` reescrito

- **El repositorio es la única fuente de verdad**, tanto para el código como para `docs/planificacion/`. Se elimina la jerarquía anterior de tres fuentes.
- **Drive es solo destino de copias**, y solo al cerrar sesión. Nunca origen, nunca importación, nunca borrado ni renombrado; si repo y Drive difieren, gana el repo. Solo se añade o se sobrescribe con el mismo nombre (o con sufijo `-v2`, `-v3`).
- **La verificación de despliegues la hace siempre un workflow.** El sandbox de la sesión no alcanza Pages, Fly ni el VPS (`CONNECT tunnel failed, response 403`) y no tiene daemon de Docker; el runner de GitHub sí. No se avisa hasta que el run está en `success`, confirmado por la API de Actions. La API de GitHub sí es accesible desde la sesión.
- **Las dos URLs vivas** quedan documentadas con el workflow que despliega cada una.

### `docs/planificacion/fase0-resultado.md`

Sustituido byte a byte por la versión FINAL que estaba en Drive (id `19rw4NaKRse5Do12OrM-2efBnxmZR2UxE`, 2224 B), que incluye el run #3 en verde y la addenda de la sesión desde el móvil. **Es la última vez que se toma algo de Drive**; a partir de aquí el flujo es solo repo → Drive.

### `tools/aterrizar.ps1` y `tools/estado.ps1`

Parametrizados. Ya no hay rutas fijas al PC de un usuario concreto:

| Parámetro | Por defecto |
|---|---|
| `-Proyecto` | `DesdeMovil` |
| `-Owner` | `npiobject` (solo `aterrizar.ps1`) |
| `-Remote` | `https://github.com/$Owner/$Proyecto.git` (solo `aterrizar.ps1`) |
| `-Root` | `$env:USERPROFILE\C - Desarrollo\$Proyecto` |
| `-Rama` | `main` |

Los valores por defecto se derivan unos de otros, así que para un proyecto nuevo basta `-Proyecto` y `-Owner`. La ruta por defecto reproduce la que había fija, pero vía `$env:USERPROFILE`.

### `ARRANQUE.md` (raíz)

Cabe en una pantalla. Los cuatro pasos que ningún agente puede hacer —crear el repo con *Use this template*, **Settings → Pages → Source: GitHub Actions**, token de Fly guardado como secreto `FLY_API_TOKEN`, carpeta normal de Drive y su id— y después el bloque de texto listo para pegar en la sesión de Code, que rellena los parámetros, hace push y verifica los dos despliegues.

### Marcadores `PLANTILLA:`

`grep -rn "PLANTILLA:"` lista los diez sitios a revisar:

| Fichero | Qué hay que sustituir |
|---|---|
| `app/fly.toml` | `app` (nombre en Fly) y `primary_region` |
| `app/Cargo.toml` | `name` del paquete |
| `app/Dockerfile` | ruta del binario, que depende de ese `name` |
| `.github/workflows/deploy.yml` | `FLY_APP` y `FLY_REGION`, que deben coincidir con `fly.toml` |
| `.github/workflows/pages.yml` | nada; marcado para dejar claro que no hay que tocarlo |
| `CLAUDE.md` | nombre, owner, app de Fly, id de Drive y las dos URLs |
| `tools/aterrizar.ps1` | `-Proyecto` y `-Owner` por defecto |
| `tools/estado.ps1` | `-Proyecto` por defecto |

### `README.md`

Deja de ser una línea suelta y apunta a `ARRANQUE.md`, `CLAUDE.md` y `docs/planificacion/`.

## Verificación

Los tres runs del SHA `3fc44a7`, todos en `success`:

| Run | Workflow | Duración |
|---|---|---|
| [34022244052](https://github.com/npiobject/DesdeMovil/actions/runs/34022244052) | Deploy docs to GitHub Pages | 21 s |
| [34022242846](https://github.com/npiobject/DesdeMovil/actions/runs/34022242846) | pages build and deployment | 41 s |
| [34022243998](https://github.com/npiobject/DesdeMovil/actions/runs/34022243998) | Desplegar backend en Fly.io | 57 s |

El paso *Verificar /salud* del despliegue devolvió al primer intento:

```
Intento 1/10: {"build":"3fc44a79d2e137ea3357f79f9926ee1db1208db0","ok":true}
OK: /salud responde con el build 3fc44a79d2e137ea3357f79f9926ee1db1208db0
```

Es decir: el ciclo completo push → build → deploy → verificación funciona con los cambios de la consolidación, y el `BUILD_ID` que sirve Fly es el del commit exacto.

Comprobaciones locales previas al push: `cargo build --release` en verde tras añadir los comentarios a `Cargo.toml`, YAML de los dos workflows parseado con `yaml.safe_load`, `app/fly.toml` parseado con `tomllib`, y el `fase0-resultado.md` traído de Drive comprobado a 2224 B, el tamaño que declara Drive.

## Limitaciones conocidas

- **[SUPUESTO] Los scripts de PowerShell no se han ejecutado.** El sandbox no tiene `pwsh`, así que la parametrización está revisada a ojo, no probada. La sintaxis usada (valores por defecto de `param()` que referencian parámetros declarados antes) es estándar de PowerShell 5.1 en adelante. Plan B si algo falla: pasar `-Root` explícito, que evita la única expresión con dependencia entre parámetros.
- La región de Fly sigue siendo `cdg`: `mad` está deprecada y no admite recursos nuevos.
- `actions/checkout@v4` avisa de la deprecación de Node 20. No rompe nada hoy; conviene subir a `v5` cuando toque.

## Estado final

| Qué | URL |
|---|---|
| Mock (Pages) | https://npiobject.github.io/DesdeMovil/ |
| Backend (Fly) | https://desdemovil-npi.fly.dev/ · https://desdemovil-npi.fly.dev/salud |
