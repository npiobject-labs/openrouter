# Plan flujo móvil — v2 (adenda a plan-flujo-movil.md)

## Resultado de la fase 0 — CERRADA (2026-09-05, 22:00)

Opción verificada con todos sus pasos funcionando, con el PC fuera del circuito:

1. **Dónde se arranca:** `claude.ai/code` (icono `</>` del panel lateral; en el móvil, pestaña "Code"). Botón "Seleccionar repositorio…" → `npiobject/DesdeMovil`, rama `main`, entorno `Santa_Claude`. La cuenta GitHub vinculada es `npiobject` y lista todos sus repos. Esa sesión tiene conector GitHub (Actions, logs, dispatch) **y** conector Google Drive. [VERIFICADO]
2. **Push desde la nube:** commits `82c817e` y `1810265` en `main`. [VERIFICADO]
3. **Despliegue B3:** `.github/workflows/pages.yml` (configure-pages + upload-pages-artifact + deploy-pages). Requiere una acción manual única: Settings → Pages → Source: *GitHub Actions* (el token de Actions no puede crear el sitio: `Resource not accessible by integration`). Run #3 `success`. [VERIFICADO]
4. **Verificación antes de avisar:** por API de Actions desde la sesión; y desde fuera `https://npiobject.github.io/DesdeMovil/` sirve título "DesdeMovil · mock 0" y `build=DM-B3-20260905-001`. [VERIFICADO]
5. **Drive con PC apagado:** `fase0-resultado.md` subido desde la sesión de la nube a `Mi unidad/DesdeMovil`. [VERIFICADO]

Correcciones al plan derivadas de la fase 0:
- Las sesiones de **Cowork** creadas desde una carpeta local no tienen acceso a GitHub. Para este flujo, la sesión se crea en **claude.ai/code con el repo seleccionado**; ahí conviven GitHub y Drive. Las "instrucciones del proyecto" pasan a vivir en `CLAUDE.md` en la raíz del repo (se carga en cada sesión de Code) además del proyecto de Cowork.
- Incógnita 5 (sesión nueva desde el móvil): la pestaña Code del móvil ofrece el mismo selector de repositorio [SUPUESTO por documentación; Fran lo confirma en la primera sesión desde el móvil].
- Incógnita 3 (Drive desde `claude.ai/code`): resuelta afirmativamente.

La versión completa del plan (fases 1–4, riesgos, instrucciones) está en `plan-flujo-movil.md` en esta carpeta, en el proyecto de Cowork y en el PC (`doc/plan/`).
