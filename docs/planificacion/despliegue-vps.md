# Despliegue en VPS

Destino alternativo (o adicional) a Fly.io para publicar las APIs del servicio.
Pendiente de etapa: aquí solo está lo decidido, no hay nada implementado.

## Parámetros

| Qué | Valor |
|---|---|
| Subdominio público | `apisor.oracle402.com` |
| Proveedor | Contabo (VPS) |
| Usuario de despliegue | `deployer` |
| Puerto SSH | 22 |
| Alias en el `~/.ssh/config` del PC | `contabo-vps` |
| Host / IP | **fuera del repo**, en el secreto `VPS_HOST` |
| Clave privada | **fuera del repo**, en el secreto `VPS_SSH_KEY` |

**Este repositorio es público.** El host y la clave no se escriben aquí ni en
ningún fichero versionado: van como secretos de repositorio y los lee el
workflow, igual que `FLY_API_TOKEN`. Publicar a qué máquina y con qué usuario se
entra es regalar la mitad del acceso.

En el PC, la conexión ya está en `~/.ssh/config` como `contabo-vps`, con
`ServerAliveInterval 60` y `ServerAliveCountMax 3`.

## Lo que hace falta antes de empezar

- [ ] Definir los secretos de repositorio `VPS_HOST` y `VPS_SSH_KEY`, y la
      variable `VPS_RUTA` con el directorio de despliegue.
- [ ] Decidir si el VPS **sustituye** a Fly o convive con él. Si convive, hay que
      decidir de quién es el histórico: la base SQLite no se puede compartir entre
      dos máquinas, así que o una de las dos es la buena o hay que cambiar de
      almacén.
- [ ] Terminador TLS delante (Caddy o nginx) para `apisor.oracle402.com`, con
      certificado automático. El backend seguirá escuchando en texto plano en su
      puerto, detrás del proxy.
- [ ] Unidad de systemd para el binario, con reinicio automático y las variables
      de entorno (`OPENROUTER_API_KEY`, `SERVICIO_CLAVE`, `BD_RUTA`) leídas de un
      fichero con permisos `600` que no está en el repo.
- [ ] Verificación desde el runner, como en `deploy.yml`: el sandbox de la sesión
      no alcanza el VPS (`CONNECT tunnel failed, response 403`), así que la
      comprobación de que el despliegue sirve la tiene que hacer el workflow.

## Nota sobre la rama

Según `CLAUDE.md`, si el proyecto usa un VPS con rama `release`, esa rama solo se
toca cuando el usuario lo pida explícitamente. Queda así hasta nueva orden.
