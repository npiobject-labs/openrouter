# Primera ejecución real de `init-plantilla.yml`

**Fecha:** 2026-09-06
**Repo de prueba:** [`npiobject/PruebaInit`](https://github.com/npiobject/PruebaInit), público, creado con *Use this template*.
**Motivo:** hasta ahora el workflow solo se había ensayado en el sandbox, extrayendo sus `run:` sobre copias del árbol. En la plantilla no puede correr (la barrera `is_template == false` corta el job), así que la única prueba posible era un repo generado de verdad.

Encontró un fallo que ningún ensayo en sandbox podía encontrar, porque no es del script: es de los permisos del token con el que corre.

## Fallo: el push del commit de inicialización, rechazado

Run [34038649858](https://github.com/npiobject/PruebaInit/actions/runs/34038649858), commit inicial `337281f`. Los siete primeros pasos en verde —sustitución, documentación heredada apartada y **el paso antirresiduos incluido**— y el octavo:

```
[main 3656bc9] Inicializar la plantilla para npiobject/PruebaInit
 28 files changed, 43 insertions(+), 248 deletions(-)
 delete mode 100644 .github/workflows/init-plantilla.yml
 delete mode 100644 .plantilla-pendiente
 ...
 ! [remote rejected] HEAD -> main (refusing to allow a GitHub App to create or
   update workflow `.github/workflows/pages.yml` without `workflows` permission)
```

**`GITHUB_TOKEN` no puede crear ni modificar nada bajo `.github/workflows/`**, y no es algo que se arregle con `permissions:`: no existe una clave `workflows` para ese token (sí existe el scope `workflow`, pero solo para PAT y GitHub Apps). Si el commit toca un solo fichero de esa carpeta, GitHub **rechaza el push entero** — no solo ese fichero. El commit hacía dos cosas ahí:

1. `sed -i '/^# PLANTILLA:/d' .github/workflows/pages.yml`, para quitar un comentario de una línea;
2. `rm -f .github/workflows/init-plantilla.yml`, la autodestrucción del propio workflow.

Resultado: **el trabajo entero se perdía**. El repo se quedaba exactamente como salió de la plantilla, con el marcador puesto y el nombre de `DesdeMovil` por todas partes, y el único aviso era un run en rojo.

### Corrección

- **`pages.yml` deja de llevar marcador `PLANTILLA:`.** Su comentario se reescribe en la plantilla (`# Este workflow no lleva el nombre del proyecto: init-plantilla.yml no lo toca.`), así que ya no hay nada que editar ahí en la inicialización. Era un comentario que solo decía «aquí no hay nada que sustituir»; costaba el push entero.
- **El workflow no se borra a sí mismo.** No puede. Se **deshabilita por la API** con `gh workflow disable init-plantilla.yml`, que solo necesita el `actions: write` que ya tenía. El marcador `.plantilla-pendiente` —que sí se borra, y no está bajo `.github/`— ya lo dejaba inerte; deshabilitarlo evita además un run vacío en cada push.
- **El grep antirresiduos excluye `init-plantilla.yml`**, que ahora sobrevive en el repo generado y contiene, por fuerza, las cadenas que busca. Es la herramienta, no el resultado.

Tras la corrección, el commit de inicialización **no toca ningún fichero de `.github/workflows/`**; comprobado en el sandbox con un `diff -rq` de esa carpeta antes y después: idénticas.

## Verificación tras la corrección

Run [34038895372](https://github.com/npiobject/PruebaInit/actions/runs/34038895372) — **success**. Commit resultante `f209929`, "Inicializar la plantilla para npiobject/PruebaInit". Lo comprobado en el árbol publicado, no en una copia local:

| Qué | Esperado | En el repo |
|---|---|---|
| Paquete Rust | `pruebainit-backend` | `Cargo.toml`, `Cargo.lock` y `Dockerfile`, los tres |
| Usuario del runtime | `pruebainit` | `useradd … pruebainit` + `USER pruebainit` |
| `GET /` y log | `PruebaInit backend` | `app/src/main.rs`, las dos cadenas |
| Prefijo del `build` | `PI-B1` | `docs/index.html` (`PI-B1-20260905-001`) y la regla de `CLAUDE.md` |
| Título y `h1` del mock | `PruebaInit · mock 0` | `docs/index.html` |
| Sección **Parámetros** | rellenada, Drive vacío | `PruebaInit` / `npiobject` / `derivada` / *(vacío)* |
| Documentación heredada | en `docs/plantilla/` | los 7 documentos + `sesiones/` |
| `docs/planificacion/` | limpia, con índice | `README.md` + `sesiones/` vacía |
| Marcador | borrado | no existe |
| `init-plantilla.yml` | deshabilitado | estado `disabled_manually` por la API |

Rastros de la plantilla que quedan, los tres queridos: el enlace *Use this template* de `ARRANQUE.md` (protegido a propósito), la cita de origen del `README.md` generado y el propio `init-plantilla.yml`.

Y el ciclo se cierra solo: `pages.yml` corrió por `workflow_dispatch` sobre el commit del bot ([34038902703](https://github.com/npiobject/PruebaInit/actions/runs/34038902703), **success**), que es el paso que existe precisamente porque un commit hecho con `GITHUB_TOKEN` no dispara workflows por push. Eso sí se confirma: el commit `f209929` no generó ningún run por `push`.

## Un supuesto que era falso

`simplificacion.md`, punto 5 de «Decisiones que no venían especificadas», daba por **[SUPUESTO]** que *crear un repo desde una plantilla no dispara workflows por push*. **Es falso.** El commit inicial `337281f` de `PruebaInit` disparó los tres workflows a la vez, todos con `event: push` y `run_number: 1`:

| Workflow | Run | Resultado |
|---|---|---|
| `init-plantilla.yml` | [34038649858](https://github.com/npiobject/PruebaInit/actions/runs/34038649858) | failure (el push rechazado de arriba) |
| `pages.yml` | [34038649877](https://github.com/npiobject/PruebaInit/actions/runs/34038649877) | failure en `Configure Pages`, Pages sin activar aún |
| `deploy.yml` | [34038649874](https://github.com/npiobject/PruebaInit/actions/runs/34038649874) | **success**, por la vía «Fly no configurado» |

Consecuencias, todas buenas menos la primera:

- El `workflow_dispatch` de `init-plantilla.yml` y la frase de `ARRANQUE.md` («si al crear el repo no llegó a lanzarse, la sesión lo lanza desde Actions») pasan de ser el camino esperado a ser una red de seguridad que normalmente no hace falta. Se quedan: no estorban.
- El aviso de `ARRANQUE.md` de que **el primer run sale en rojo** queda confirmado, y con el matiz exacto que dice: cae `pages.yml` —falla en `Configure Pages`, no en el despliegue— mientras `deploy.yml` termina en verde sin token. Un solo run rojo, no dos.

## Prueba definitiva: repo generado de cero

La verificación anterior tenía una pega: la corrección llegó a `PruebaInit` por un push mío, no dentro de la plantilla, así que quedaba **[SUPUESTO]** que un repo generado de cero se comportara igual. Se ha comprobado borrando `PruebaInit` y creándolo otra vez con *Use this template*, ya con la plantilla corregida y **sin tocar nada**:

| Run | Evento | Commit | Resultado |
|---|---|---|---|
| [34039428339](https://github.com/npiobject/PruebaInit/actions/runs/34039428339) `init-plantilla.yml` | `push`, `run_number: 1` | `e8e0e62` (*Initial commit*) | **success** |
| [34039428323](https://github.com/npiobject/PruebaInit/actions/runs/34039428323) `deploy.yml` | `push`, `run_number: 1` | `e8e0e62` | **success**, vía «Fly no configurado» |
| [34039428319](https://github.com/npiobject/PruebaInit/actions/runs/34039428319) `pages.yml` | `push`, `run_number: 1` | `e8e0e62` | failure en `Configure Pages`, Pages sin activar a propósito |

La inicialización se completó sola y a la primera, 13 segundos después de crear el repo, en el commit `3049bae`. Comprobado en el árbol publicado:

- El commit **no toca ni un fichero de `.github/workflows/`** (`git show --name-only` no devuelve nada bajo esa ruta), que era la causa del fallo.
- Backend: `pruebainit-backend` en `Cargo.toml`, `Cargo.lock` y `Dockerfile`; usuario `pruebainit`; `GET /` y log → `PruebaInit backend`.
- Mock: `PI-B1-20260905-001`, título `PruebaInit · mock 0`; la regla de `CLAUDE.md` con `PI-B1-AAAAMMDD-NNN`.
- **Parámetros** rellenada (`PruebaInit` / `npiobject` / `derivada`) con el id de Drive vacío.
- `docs/plantilla/` con los 8 documentos heredados; `docs/planificacion/` con solo su `README.md` y `sesiones/`.
- Marcador borrado, ningún comentario `# PLANTILLA` en todo el repo, y `init-plantilla.yml` en `disabled_manually`.

Los únicos rastros de la plantilla son los tres queridos: el enlace *Use this template* de `ARRANQUE.md`, la cita de origen del `README.md` generado y el propio `init-plantilla.yml`.

El **[SUPUESTO]** queda cerrado: la plantilla inicializa un proyecto nuevo sin intervención, con un solo paso manual pendiente (activar Pages).

### Nota sobre Pages

El error del primer run se leyó entero y es, literalmente, el que documenta `ARRANQUE.md`:

```
##[warning]Get Pages site failed. Error: Not Found
##[error]Create Pages site failed. Error: Resource not accessible by integration
```

Confirmado también que `enablement: true` no basta: el `GITHUB_TOKEN` del job llega con `Pages: write` y aun así no puede **crear** el sitio. El `workflow_dispatch` que lanza `init-plantilla.yml` al terminar falla por lo mismo mientras Pages siga sin activar; en cuanto se cambia Source a «GitHub Actions», basta relanzarlo.

Hecho eso, el relanzamiento cierra el ciclo: run [34039653290](https://github.com/npiobject/PruebaInit/actions/runs/34039653290), **success**, con los cinco pasos en verde —`Configure Pages`, que antes fallaba, incluido— y `Deploy to GitHub Pages` publicando https://npiobject.github.io/PruebaInit/ con el mock `PI-B1-20260905-001`.

## Estado final de la prueba

| Paso | Cómo terminó |
|---|---|
| Crear el repo desde la plantilla | manual, el único que no puede hacer un agente |
| `init-plantilla.yml` | **success** solo, en el `run_number: 1` del *Initial commit* |
| `deploy.yml` | **success** sin token, por la vía «Fly no configurado» |
| Activar Pages | manual, el segundo y último paso |
| `pages.yml` relanzado | **success**, sitio publicado |

Los dos pasos manuales que promete `ARRANQUE.md` son exactamente dos, y todo lo demás salió solo.
