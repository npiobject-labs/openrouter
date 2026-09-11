# Portado de las mejoras de los proyectos hijos a la plantilla

**Fecha:** 2026-09-06
**Origen:** `npiobject/PruebaPlantilla` y `npiobject/proyecto_2`, los dos repos creados desde esta plantilla.
**Método:** diff fichero a fichero (workflows, `CLAUDE.md`, `ARRANQUE.md`, `app/`, `tools/`, `docs/index.html`, `README.md`), normalizando antes el nombre del proyecto, el slug y el prefijo de build para que las diferencias de puro renombrado no aparecieran.

El contexto importa: `PruebaPlantilla` levantó diez fricciones al ejecutar `ARRANQUE.md` a mano y las corrigió con un `tools/inicializar.sh`; `proyecto_2` rehízo lo mismo a mano y dejó un aprendizaje sobre Pages. Después, la plantilla se **simplificó** (Pages siempre, Fly y Drive opcionales, `init-plantilla.yml` automático), lo que resolvió por otro camino parte de aquellas fricciones y dejó obsoleta la corrección de fondo. Lo que sigue separa lo que seguía sin resolver de lo que ya no aplicaba.

## Portado

### 1. `init-plantilla.yml` no tocaba el backend ni el prefijo de build

Era la fricción 1 de `PruebaPlantilla` (la lista de sitios a sustituir estaba incompleta) y la 3 (el prefijo del `build` no estaba parametrizado). La simplificación automatizó la sustitución pero heredó la lista corta: solo `CLAUDE.md`, `README.md`, `ARRANQUE.md`, `docs/*.html` y `tools/*.ps1`.

Consecuencia real, no teórica: un proyecto generado compilaba el paquete `desdemovil-backend`, creaba el usuario `desdemovil` en el contenedor, respondía `"DesdeMovil backend"` en `GET /` —público, en la URL de Fly del proyecto nuevo— y publicaba mocks con el prefijo `DM-B3`, que son las iniciales y la fase de *este* proyecto. Es exactamente lo que `proyecto_2` tuvo que arreglar a mano.

El workflow ahora deriva y sustituye:

| Valor | Cómo se calcula | Dónde se aplica |
|---|---|---|
| Nombre y owner | `github.repository` | `CLAUDE.md`, `README.md`, `ARRANQUE.md`, `docs/*.html`, `tools/*.ps1`, **`app/src/main.rs`** |
| Slug | nombre en minúsculas, saneado a `[a-z0-9-]` | **`app/Cargo.toml`**, **`app/Cargo.lock`**, **`app/Dockerfile`** (`<slug>-backend`) |
| Usuario del runtime | slug sin guiones, sin empezar por dígito | **`app/Dockerfile`** |
| Prefijo del `build` | iniciales del nombre (`MiProyecto` → `MP`, `proyecto_2` → `P2`) | **`CLAUDE.md`**, **`docs/index.html`**, **`docs/guia.html`** |

También borra los comentarios `PLANTILLA:` ya resueltos de `pages.yml`, `app/Cargo.toml` y `app/Dockerfile`.

### 2. Verificación antirresiduos

Fricción 2: `grep -rn "PLANTILLA:"` a mano no es un checklist fiable, porque la cadena aparece también en prosa. En `inicializar.sh` la corrección fue un grep final que aborta; aquí es un paso del workflow, **`Comprobar que no queda ningun residuo`**, que busca `desdemovil`, `DM-B3` y `PLANTILLA:` en todo el repo excluyendo `docs/plantilla/` y el enlace protegido *Use this template*, y **falla el run** listando fichero y línea. Que el checklist esté en Actions y no en la cabeza de nadie es la diferencia entre las dos ejecuciones reales que hemos tenido.

Ese paso ya se ganó el sitio: al ensayarlo destapó que `docs/guia.html` —añadida después de las fricciones— llevaba su propio `meta name="build"` con el prefijo `DM-B3` y se quedaba sin sustituir.

### 3. Documentación heredada

Fricción 4: el repo hijo nacía con la planificación completa de la plantilla mezclada en la carpeta donde va la del proyecto nuevo. `PruebaPlantilla` la apartó a `docs/plantilla/`; `proyecto_2` optó por dejarla donde estaba con un `README.md` que la marca como historia. Se porta la primera, que deja `docs/planificacion/` limpia, más el `README.md` de la segunda como índice de lo que hay y dónde:

- paso `Apartar la documentacion heredada`: mueve `docs/planificacion/*` a `docs/plantilla/`;
- paso `Escribir el indice…`: genera `docs/planificacion/README.md` explicando el reparto;
- `CLAUDE.md` añade la regla: `docs/plantilla/` es referencia de solo lectura, nunca se edita ni se mezcla.

### 4. `ARRANQUE.md`: el error real de Pages

Fricción 5. La plantilla decía solo que, sin cambiar Source, «el enlace muestra el README en vez del mock». El fallo real del primer run es `Create Pages site failed. Error: Resource not accessible by integration`, precedido de un *warning* `Get Pages site failed… Not Found`. Y el matiz que cuesta una sesión de diagnóstico: `enablement: true` en `configure-pages` **no** sustituye al paso manual, porque el `GITHUB_TOKEN` no puede crear el sitio. Ahora está los dos sitios: en `ARRANQUE.md` y en la tabla de errores de `docs/guia.html`.

### 5. `ARRANQUE.md`: el primer run en rojo es normal

Fricción 6, adaptada. *Use this template* dispara los workflows con el commit inicial, antes de los pasos manuales. Con la simplificación ya solo cae uno —`deploy.yml` termina en verde sin token—, así que el aviso se porta hablando de `pages.yml` en singular y añadiendo que `init-plantilla.yml` relanza Pages al terminar.

### 6. `ARRANQUE.md`: el nombre de app de Fly es único en todo Fly.io

Fricción 7. Sigue aplicando pese al nombre derivado: `flyctl apps create … || true` se traga el choque y el fallo asoma después, en el `deploy`, con un mensaje que no apunta a la causa. Se porta la advertencia, con la salida (definir la variable `FLY_APP`) y una fila en la tabla de errores de la guía.

### 7. `pages.yml` solo se verifica en `main`

Aprendizaje de `proyecto_2`: el entorno `github-pages` únicamente despliega desde la rama por defecto, así que un `workflow_dispatch` sobre una rama de trabajo no vale como verificación; `deploy.yml` sí acepta cualquier rama. Anotado en la sección «Verificación antes de avisar» de `CLAUDE.md` y en la guía. También justifica el comentario del paso que relanza Pages en `init-plantilla.yml`.

### 8. `actions/checkout@v4` → `@v5`

De `PruebaPlantilla`: quita el aviso de deprecación de Node 20 en los tres workflows. `actions/configure-pages@v5` sigue avisando; no hay versión posterior.

## Descartado

| Qué | Por qué |
|---|---|
| `tools/inicializar.sh` (`PruebaPlantilla`) | Su razón de ser era hacer a mano lo que ahora hace `init-plantilla.yml` sin intervención. Se porta su **lógica** (puntos 1–3), no el fichero: mantenerlo obligaría a un paso manual que la simplificación eliminó. |
| `deploy.yml` de los hijos: `env: FLY_APP` / `FLY_REGION` fijos y token obligatorio | Regresión. La plantilla ya deriva el nombre (`<repo>-<owner>`, o la variable `FLY_APP`), lee la región de `app/fly.toml` —única fuente— y trata Fly como opcional: sin secreto termina en verde en vez de dejar un check rojo permanente. |
| `app/fly.toml` con `app = "<proyecto>-npi"` | Mismo motivo: la plantilla quitó la clave a propósito para no arrastrar el nombre de otro proyecto. |
| `ARRANQUE.md` de los hijos: pasos 3 y 4 (Fly y Drive) como obligatorios, y el bloque de instrucciones con cuatro valores a rellenar | Contradice la simplificación: arrancar son dos pasos y ningún valor que teclear. Lo que sí valía de ese `ARRANQUE.md` está portado en los puntos 4–6. |
| `tools/aterrizar.ps1` y `tools/estado.ps1` de los hijos | Son la versión anterior: sin `Assert-Git` (git no lanza excepciones, hay que mirar `$LASTEXITCODE`) y sin la comprobación de `repo\` no vacía. La plantilla va por delante. |
| Mensaje de Drive «no existe. Crea la carpeta con Google Drive de escritorio…» | Drive es opcional desde la simplificación; la ausencia de carpeta no es un error que haya que explicar. |
| `CLAUDE.md` de los hijos | Anteriores a la sección **Parámetros** y a Fly/Drive opcionales. Lo único aprovechable era la regla de `docs/plantilla/`, portada en el punto 3. |
| `README.md` de los hijos | Son READMEs de proyecto (URLs vivas concretas); el de la plantilla describe el método. |
| `docs/planificacion/README.md` de `proyecto_2` tal cual | Su tabla enumera los documentos de las fases 0–3 con el nombre de la plantilla. Se porta la idea, generada por el workflow y apuntando a `docs/plantilla/`. |
| `app/src/main.rs`, `docs/index.html`, orden de `app/Cargo.lock`, textos de `README`/`CLAUDE.md` | Solo nombre de proyecto o reordenación cosmética. |
| `docs/plantilla/` de `PruebaPlantilla` (copia de la planificación) | Es la historia de este mismo repo; ya está aquí, en `docs/planificacion/`. |

## Verificación en la sesión

`init-plantilla.yml` no se puede ejecutar en la plantilla (la barrera `is_template == false` corta el job), así que se ensayó extrayendo los `run:` del propio YAML y ejecutándolos sobre copias del árbol con `GITHUB_REPOSITORY` ficticio:

| `GITHUB_REPOSITORY` | Slug | Usuario | Prefijo | Residuos |
|---|---|---|---|---|
| `miusuario/CasaVerde` | `casaverde` | `casaverde` | `CV-B1` | ninguno |
| `empresa/Mi-Proyecto_2026` | `mi-proyecto-2026` | `miproyecto2026` | `MP2-B1` | ninguno |
| `u/casa` | `casa` | `casa` | `CA-B1` | ninguno |
| `Org_X/Un.Proyecto_Con.Nombre.Muy.Largo` | `un-proyecto-con-nombre-muy-largo` | `unproyectoconnombremuylargo` | `UPC-B1` | ninguno |

En todos los casos: `docs/planificacion/` queda con su `README.md` y `sesiones/` vacía, `docs/plantilla/` con los siete documentos heredados, marcador y workflow borrados, y los dos únicos rastros de la plantilla son los queridos (el enlace *Use this template* de `ARRANQUE.md` y la cita de origen del `README.md` generado).

Además, sobre el árbol de `miusuario/CasaVerde`: `cargo build --release` en verde compilando `casaverde-backend`, los tres workflows pasan `yaml.safe_load` y `docs/guia.html` cierra todas las etiquetas.

## Runs de verificación (SHA `63c590a0fc42211aad56f8d3725985fda91c6efa`)

| Workflow | Run | Resultado |
|---|---|---|
| `pages.yml` | [34032563770](https://github.com/npiobject/DesdeMovil/actions/runs/34032563770) | **success** |
| `deploy.yml` | [34032563781](https://github.com/npiobject/DesdeMovil/actions/runs/34032563781) | **success** (app derivada `desdemovil-npiobject`, paso *Verificar /salud* en verde) |
| `init-plantilla.yml` | [34032563762](https://github.com/npiobject/DesdeMovil/actions/runs/34032563762) | **skipped** — la barrera `is_template == false` corta el job en la plantilla |

## Supuestos

- **[SUPUESTO]** El prefijo del `build` es libre por proyecto y derivarlo de las iniciales es aceptable. Plan B: `init-plantilla.yml` puede leerlo de una variable de repositorio, o dejarse fijo y renumerar.
- **[SUPUESTO]** Ningún proyecto generado quiere conservar la planificación de la plantilla dentro de `docs/planificacion/`. Plan B si estorba `docs/plantilla/`: borrarla en el repo hijo, que no la usa nadie más.
