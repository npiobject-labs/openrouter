# DesdeMovil — Fase 0: resultado (FINAL, OK)

**Fecha:** 2026-09-05
**Repositorio:** https://github.com/npiobject/DesdeMovil
**Rama:** main

## Qué se ha hecho

- `docs/index.html` — mock mínimo con `<meta name="build" content="DM-B3-20260905-001">` y título "DesdeMovil · mock 0".
- `.github/workflows/pages.yml` — despliegue de `docs/` a GitHub Pages con
  `actions/configure-pages@v5` + `actions/upload-pages-artifact@v3` + `actions/deploy-pages@v4`,
  permisos `contents: read`, `pages: write`, `id-token: write`, disparo en `push` a `main` y `workflow_dispatch`.

## Commits

| SHA | Descripción |
|---|---|
| `82c817e` | Mock `docs/index.html` + workflow de Pages |
| `1810265` | `configure-pages` con `enablement: true` |

## Estado del workflow

| Run | SHA | Evento | Resultado |
|---|---|---|---|
| [#1](https://github.com/npiobject/DesdeMovil/actions/runs/33987990711) | `82c817e` | push | ❌ failure — `Get Pages site failed… Not Found` (Pages no habilitado) |
| [#2](https://github.com/npiobject/DesdeMovil/actions/runs/33988024125) | `1810265` | push | ❌ failure — `Create Pages site failed. Resource not accessible by integration` |
| [#3](https://github.com/npiobject/DesdeMovil/actions/runs/33988504572) | `1810265` | workflow_dispatch | ✅ **success** — tras activar Settings → Pages → Source: GitHub Actions |

Pasos del run #3, todos en verde: Checkout → Configure Pages → Upload docs artifact → Deploy to GitHub Pages.

## URL de Pages (publicada)

https://npiobject.github.io/DesdeMovil/

Debe servir el mock con `build = DM-B3-20260905-001`.

## Conclusión

Fase 0 cerrada. El pipeline "push a `main` → publica `docs/` en Pages" queda operativo:
cualquier commit posterior sobre `main` que toque `docs/` se despliega automáticamente.

`enablement: true` ya no es necesario (el sitio existe), pero es inocuo y protege ante un repositorio nuevo.

## Adenda (sesión desde el móvil, 2026-09-05 ~23:00)

Incógnita 5 resuelta: una sesión nueva creada desde la app móvil (pestaña Code, repo seleccionado) hizo commit y push a `main` de `tools/aterrizar.ps1`, `tools/estado.ps1` y `docs/planificacion/*` tomados de Drive. Fase 0 cerrada sin suposiciones pendientes.
