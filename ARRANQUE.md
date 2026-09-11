# Arranque

## Por proyecto (2 pasos)

1. **Crear el repo**: [Use this template](https://github.com/new?template_name=DesdeMovil&template_owner=npiobject-labs). En **Owner** elige **`npiobject-labs`**, no tu cuenta personal, y déjalo **Public**: de ahí hereda el repo el secreto `FLY_API_TOKEN` de la organización.
2. **Pages**: en el repo nuevo, **Settings → Pages → Build and deployment → Source**: cambia el desplegable de **Deploy from a branch** a **GitHub Actions**.
   Si lo dejas como está tendrás un enlace que funciona pero que muestra el README en vez del mock; y si el workflow corre antes de este cambio, falla con `Create Pages site failed. Error: Resource not accessible by integration` (precedido de un *warning* `Get Pages site failed… Not Found`). El `enablement: true` de `configure-pages` **no** sustituye a este paso: el `GITHUB_TOKEN` no tiene permiso para crear el sitio.
   Cómo saber si se te olvidó: en **Actions** aparece un run `pages build and deployment` con `Build with Jekyll` tras cada push. Ese constructor solo corre en el modo antiguo; con Source en «GitHub Actions» no existe.

> **El primer run en rojo es normal.** *Use this template* dispara `pages.yml` con el commit inicial, antes de que hayas tocado Settings, así que ese run falla. El historial de Actions arranca en rojo y no es un problema: `init-plantilla.yml` relanza Pages al terminar. (`deploy.yml` no falla: con el token de la organización a mano, despliega; sin él termina en verde con el aviso "Fly no configurado".)

Y ya. Abre una sesión en [claude.ai/code](https://claude.ai/code) con el repo seleccionado y pide:

```
verifica que el proyecto quedó inicializado y que Pages responde
```

No hay nada que rellenar. `.github/workflows/init-plantilla.yml` deja el repo entero con el nombre del proyecto:

| Qué | Dónde |
|---|---|
| Nombre y owner | `CLAUDE.md`, `README.md`, `ARRANQUE.md`, `docs/*.html`, `tools/*.ps1`, `app/src/main.rs` |
| Paquete del backend (`<slug>-backend`) | `app/Cargo.toml`, `app/Cargo.lock`, `app/Dockerfile` |
| Usuario del runtime | `app/Dockerfile` |
| Prefijo del `build` (iniciales: `CasaVerde` → `CV-B1`) | `CLAUDE.md`, `docs/*.html` |
| Sección **Parámetros** rellenada, id de Drive vaciado | `CLAUDE.md` |
| Documentación heredada apartada a `docs/plantilla/` | `docs/planificacion/` queda limpia, con su `README.md` |

Al final hace `grep` de todo lo que huela a plantilla fuera de `docs/plantilla/` y **falla el run si encuentra algo**: ese grep es el checklist real, no una lista de ficheros que revisar a mano. Después borra el marcador `.plantilla-pendiente`, se deshabilita a sí mismo y relanza `pages.yml`. Si al crear el repo no llegó a lanzarse, la sesión lo lanza desde **Actions → Inicializar plantilla → Run workflow**.

No se borra a sí mismo porque no puede: `GITHUB_TOKEN` no tiene permiso para modificar nada bajo `.github/workflows/`, y un commit que lo intente hace que GitHub **rechace el push entero**. Por eso el workflow no toca ningún fichero de ahí y se apaga por la API en su lugar. Queda en el repo, deshabilitado e inerte —sin el marcador no haría nada aunque se relanzara—; bórralo a mano si te molesta.

Resultado: https://npiobject-labs.github.io/DesdeMovil/ sirviendo el mock de `docs/`.

## Trabajo diario: la bitácora

Cada sesión de Code termina con una entrada nueva en `docs/bitacora/AAAAMMDD-HHMM.json` y queda publicada, sin cuenta de Claude ni de GitHub, en:

https://npiobject-labs.github.io/DesdeMovil/bitacora.html

Sesiones en orden inverso, con buscador: lo que se pidió (con botón de copiar), qué cambió, y enlaces al commit, al run que lo publicó, al mock archivado de ese día y al servidor si estaba activo. El índice lo genera `pages.yml`; nadie edita la página a mano. Formato en [`docs/bitacora/README.md`](docs/bitacora/README.md). Prompt de cierre:

```
Cierra la sesión: resumen de 5 líneas en docs/planificacion/sesiones/ y entrada
nueva en docs/bitacora/AAAAMMDD-HHMM.json con objetivo, los prompts de esta
sesión, qué cambió, SHA, run de Actions, mock archivado y URL de Fly si aplica.
Push, verifica que Pages queda en verde, y sube la copia a Drive.
```

## Si a mitad del proyecto necesitas Fly

No hay nada que activar: **`FLY_API_TOKEN` es un secreto de organización de `npiobject-labs`** y lo heredan todos sus repos públicos. Ya no se crea un token de Fly por proyecto ni se guarda como secreto del repo.

> El repo tiene que ser **público**. En el plan Free de GitHub los secretos de organización no llegan a los repos privados: ahí `deploy.yml` terminaría en verde con el aviso "Fly no configurado" y no desplegaría nada.

El siguiente push que toque `app/**` despliega; o lánzalo a mano desde **Actions → Desplegar backend en Fly.io → Run workflow**. Como el commit inicial de un repo creado desde la plantilla ya toca `app/**`, el primer despliegue sale solo. El primero tarda varios minutos porque compila Rust.

Opcional: define la variable de repositorio (**Settings → Secrets and variables → Actions → Variables**) **`FLY_APP`** si quieres un nombre concreto de app. Sin ella, la app se llama `<repo>-<owner>` en minúsculas, recortado a 30 caracteres.

El **nombre de app es único en todo Fly.io**, no solo en tu cuenta. Si el que toca ya está cogido por otra cuenta, `flyctl apps create` no protesta —el workflow lo ignora con `|| true`— y el fallo aparece más tarde, en el paso de `deploy`, con un mensaje que no apunta a la causa. El nombre derivado lleva el owner de sufijo justamente para que eso no pase; si aun así choca, define `FLY_APP`.

Queda `https://<APP>.fly.dev/` (texto plano), `https://<APP>.fly.dev/salud` devolviendo `{"ok":true,"build":"<SHA>"}` y `https://<APP>.fly.dev/holamundo` devolviendo `holamundo`. La prueba de que **frontend y backend hablan entre sí** es https://npiobject-labs.github.io/DesdeMovil/holamundo.html: Pages llama a las dos rutas desde el navegador y muestra la respuesta y el build desplegado. La verificación no la haces tú: el propio workflow hace `curl` a `/salud` y falla el run si la respuesta no contiene el SHA del commit desplegado.

## Si quieres copias en Drive

Crea una carpeta normal en **Mi unidad** (no un "Proyecto" de Drive: el conector no puede escribir en esos), ábrela y copia el id de la URL `https://drive.google.com/drive/folders/<ID>`. Pégalo en la fila **Carpeta de Drive (id)** de la sección **Parámetros** de `CLAUDE.md`. Con la fila vacía, Drive se omite sin más.

## Aterrizar en el PC

En PowerShell:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/npiobject-labs/DesdeMovil/main/tools/aterrizar.ps1)))
```

Crea `%USERPROFILE%\C - Desarrollo\DesdeMovil\repo` con un clon de `main`. Es idempotente y **sobrescribe** la copia local sin preguntar (`reset --hard` + `clean -fdx`): el PC es un espejo de solo lectura. Para saber si estás al día, `tools\estado.ps1`.

Sirve igual la primera vez y las siguientes: no hace falta tener el repo para bajarlo, porque el comando se descarga el script suelto y este hace el clon.

### Probar la app sin desplegar

Con el repo ya en el PC y [Rust](https://rustup.rs) instalado:

```powershell
pwsh -File tools\arrancar.ps1
```

Compila el backend, lo levanta en `localhost:8080`, sirve `docs/` en `localhost:8081` y abre el navegador; Ctrl+C para los dos. Servidas desde `localhost`, las páginas llaman a ese backend en vez de al de Fly. Acepta `-PuertoApi`, `-PuertoWeb`, `-Release` y `-SinNavegador`.

## Eliminar el proyecto

Con el repo aterrizado, en PowerShell:

```powershell
pwsh -File tools\eliminar.ps1            # enseña el plan, no borra nada
pwsh -File tools\eliminar.ps1 -Confirmar # borra, pidiendo el nombre del proyecto
```

Hace tres de los cuatro pasos, en el orden que toca: la app de Fly (con `flyctl`), el repositorio (con `gh`, que necesita el permiso `delete_repo`) y la copia local entera. Lo que falte por no tener esas herramientas lo deja anotado como pendiente con su enlace. Acepta `-SinFly`, `-SinRepo` y `-SinLocal`. La carpeta de Drive y las sesiones de `claude.ai/code` se borran a mano: el script no tiene credenciales para eso.

---

Guías: [`docs/guiaFableMini.html`](docs/guiaFableMini.html) (checklist de arranque, para el móvil) · [`docs/guiaNew.html`](docs/guiaNew.html) (el recorrido entero, paso a paso) · [`docs/guiaFable.html`](docs/guiaFable.html) (el método explicado y cuándo compensa) · Reglas para los agentes: [`CLAUDE.md`](CLAUDE.md)
