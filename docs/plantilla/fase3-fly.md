# DesdeMovil — Fase 3: backend en Fly.io

**Fecha:** 2026-09-06
**Repositorio:** https://github.com/npiobject/DesdeMovil
**Rama:** `main` (desarrollo en `claude/fase3-backend-flyio-ou08je`, PR [#1](https://github.com/npiobject/DesdeMovil/pull/1))

## Resultado

| Dato | Valor |
|---|---|
| URL | https://desdemovil-npi.fly.dev/ |
| Salud | https://desdemovil-npi.fly.dev/salud |
| App de Fly | `desdemovil-npi` |
| Región | `cdg` (París) — **no `mad`**, ver incidencias |
| SHA desplegado | `9086f77abe96f1e9da60b6ccedc6a17aae68c04f` |
| Respuesta verificada | `{"build":"9086f77abe96f1e9da60b6ccedc6a17aae68c04f","ok":true}` |
| Tamaño de la imagen | 26 MB |

## Qué se ha construido

### `app/` — servicio Rust (axum 0.8 + tokio 1)

- `GET /` → texto plano `DesdeMovil backend`.
- `GET /salud` → JSON `{"ok":true,"build":"<BUILD_ID>"}`; si no hay `BUILD_ID` en el entorno, devuelve `"dev"`.
- Escucha en `0.0.0.0:8080`. Apagado ordenado ante SIGINT, que es la señal que envía Fly al parar máquinas por inactividad.
- Perfil de release con `opt-level = "z"`, `lto`, `codegen-units = 1` y `strip`.

### `app/Dockerfile` — multi-stage

- Compilación en `rust:1-slim-bookworm`, con una capa previa que compila solo las dependencias (`Cargo.toml` + `Cargo.lock` y un `main.rs` vacío) para que el builder remoto de Fly las cachee entre despliegues.
- Ejecución en `debian:bookworm-slim` con `ca-certificates` y usuario sin privilegios (`desdemovil`, uid 10001).

### `app/fly.toml`

`app = "desdemovil-npi"`, `primary_region = "cdg"`, `http_service` interno en 8080 con `force_https = true`, `auto_stop_machines = true`, `auto_start_machines = true`, `min_machines_running = 0`, VM `shared-cpu-1x` / 256 MB. Con `min_machines_running = 0` la máquina se para sola sin tráfico y la primera petición la despierta.

### `.github/workflows/deploy.yml`

Se dispara en push a `main` que toque `app/**` o el propio workflow, y en `workflow_dispatch`. Pasos:

1. `actions/checkout@v4`.
2. Comprobación de que `secrets.FLY_API_TOKEN` existe (falla con mensaje claro si no; nunca imprime el valor).
3. `superfly/flyctl-actions/setup-flyctl@master`.
4. `flyctl apps create desdemovil-npi --org personal || true`.
5. `flyctl secrets set BUILD_ID=$GITHUB_SHA --app desdemovil-npi --stage` (`--stage` evita un reinicio extra: el valor se aplica con el deploy siguiente).
6. `flyctl deploy app --remote-only --ha=false`.
7. Verificación: `curl` a `https://desdemovil-npi.fly.dev/salud`, hasta 10 intentos con 15 s de espera; el run falla si la respuesta no contiene el SHA del commit.

El token viaja solo como variable de entorno del job y GitHub lo enmascara en los logs (`FLY_API_TOKEN: ***`).

## Runs

| Run | SHA | Resultado | Duración | Causa |
|---|---|---|---|---|
| [34021316020](https://github.com/npiobject/DesdeMovil/actions/runs/34021316020) | `798fccd` | ❌ failure | 1 min 23 s | Región `mad` deprecada en Fly.io |
| [34021419513](https://github.com/npiobject/DesdeMovil/actions/runs/34021419513) | `9086f77` | ✅ success | 32 s | — |

Desglose del run correcto: `flyctl deploy` 21 s (imagen ya cacheada en el registry por el intento anterior), verificación de `/salud` 1 s y con éxito al primer intento. En un despliegue en frío la compilación de la imagen ronda los 70 s.

## Incidencias

### 1. Fly.io ha deprecado la región `mad`

El primer despliegue construyó y subió la imagen sin problemas, provisionó las IPs (`2a09:8280:1::184:d227:0` dedicada IPv6 y `66.241.124.85` IPv4 compartida) y falló al crear la máquina:

```
error creating a new machine: failed to launch VM: Region mad is deprecated and cannot have
new resources provisioned. Please consider using an alternate region such as cdg
```

**Decisión:** se cambió `primary_region` a `cdg` (París), la alternativa que sugiere el propio Fly y la más cercana a Madrid de las disponibles. Alternativas si se prefiere otra: `ams` (Ámsterdam) o `lhr` (Londres); es un cambio de una línea en `app/fly.toml` más un redespliegue.

### 2. No se pudo validar la imagen Docker desde la sesión

El sandbox de la sesión en la nube no tiene daemon de Docker, así que el `docker build` solo se ejecutó en el builder remoto de Fly. Lo que sí se validó localmente antes del push: `cargo build --release` y ambos endpoints en ejecución (`GET /` devolvió el texto y `GET /salud` devolvió `{"build":"pruebalocal","ok":true}` con `BUILD_ID=pruebalocal`).

### 3. El sandbox bloquea `*.fly.dev`

`curl https://desdemovil-npi.fly.dev/salud` desde la sesión devuelve `CONNECT tunnel failed, response 403`, igual que ya ocurría con el VPS y con `*.github.io`. La verificación válida es la del paso 7 del workflow, que corre desde el runner de GitHub y cuya salida queda en el log del run.

## Qué queda pendiente

- Elegir si `cdg` es definitiva o se prefiere `ams`/`lhr`.
- Decidir el dominio propio (hoy solo `desdemovil-npi.fly.dev`) y su certificado en Fly.
- Conectar el mock de `docs/` con este backend: hoy son dos despliegues independientes (Pages y Fly) sin ninguna llamada entre ellos.
- Definir si el backend necesita persistencia; con `min_machines_running = 0` no hay estado entre paradas.
