# DesdeMovil

Plantilla del método "PC arranca, móvil continúa":

- El repositorio es la **única fuente de verdad**: código, planificación y decisiones viven aquí.
- El trabajo se hace en **sesiones de claude.ai/code** con este repo seleccionado, desde el móvil o desde la web, con el PC apagado.
- **GitHub Pages siempre**: `docs/` se publica en cada push a `main`. Es lo único que hace falta para arrancar.
- **Fly.io opcional**: se activa a mitad de proyecto añadiendo el secreto `FLY_API_TOKEN`; sin él, el workflow de despliegue termina en verde y no hace nada.
- **Google Drive opcional**: solo destino de copias de `docs/planificacion/`, y solo si pones su id en `CLAUDE.md`.
- **Nada se verifica desde la sesión**: el sandbox no alcanza internet, así que los despliegues los comprueban los propios workflows.
- El **PC solo sirve para aterrizar** una copia de lectura con `tools/aterrizar.ps1`; nunca es origen de cambios.
- Un proyecto nuevo son **dos pasos manuales**: crear el repo desde la plantilla y activar Pages. El resto lo hace `init-plantilla.yml`.
- Esa inicialización deja el repo **entero** con su nombre —hasta el paquete Rust, el `GET /` del backend y el prefijo del `build`—, aparta la historia de la plantilla a `docs/plantilla/` y **falla el run** si se deja algo sin sustituir.

Empieza por [`ARRANQUE.md`](ARRANQUE.md) · Reglas para los agentes en [`CLAUDE.md`](CLAUDE.md) · Planificación en [`docs/planificacion/`](docs/planificacion/).

Tres guías en `docs/`, con enfoques distintos y ninguna sustituye a las otras:

| Guía | Para qué | Cómo es |
|---|---|---|
| [`docs/guiaFableMini.html`](docs/guiaFableMini.html) | Arrancar un proyecto con el móvil en la mano | Checklist de tres pasos, con el nombre del proyecto rellenando los enlaces y el prompt listo para copiar |
| [`docs/guiaNew.html`](docs/guiaNew.html) | El recorrido entero, paso a paso | Manual operativo con casillas, progreso, diagnóstico y cómo borrar un proyecto sin restos |
| [`docs/guiaFable.html`](docs/guiaFable.html) | Entender el método y decidir si compensa | Prosa y tablas: cuándo este flujo gana, qué vale como verificación y qué no |
