# Runbook — el servicio en el VPS (apisor.oracle402.com)

Operaciones del día a día. Todo lo que toca el VPS pasa por un workflow o por
SSH desde el PC; la sesión de claude.ai/code no llega al VPS.

## Dónde está cada cosa en el VPS

| Qué | Dónde |
|---|---|
| Ficheros del despliegue (`compose.yml`, scripts) | `/srv/openrouter/vps/` (copia de `vps/` del SHA desplegado) |
| Secretos y configuración | `/srv/openrouter/.env` (root, 600) |
| Base SQLite | `/srv/openrouter/datos/uso.db` (uid 10001) |
| Copias diarias | `/srv/openrouter/copias/uso-AAAAMMDD.db.gz` (30 días) y `uso-AAAAMMDD.csv` los lunes (90 días) |
| Sitio de Caddy | `/etc/caddy/sites.d/apisor.oracle402.com.caddy`, importado por nombre desde `/etc/caddy/Caddyfile` |
| Contenedor | `openrouter`, imagen `openrouter-backend:<sha>`, publicado en `127.0.0.1:8081` |
| Timer de copias | `openrouter-copia.timer` (03:30, systemd) |

## Desplegar

1. Fusionar en `main` lo que haya que desplegar.
2. Cuando el usuario lo pida: `release` = `main` (fast-forward) y push.
3. Corre `deploy-vps.yml`. Termina en verde solo si `/salud` devuelve el SHA,
   `/v1/estado` dice `sqlite` y el resto de comprobaciones de
   `tools/verificar-servicio.sh` pasan. El resumen del run tiene las URLs.

Desde el PC:

```powershell
git fetch origin main
git push origin origin/main:release
```

## Rollback

Actions → **Desplegar backend en el VPS** → *Run workflow* → `sha` = el commit
al que volver. Reconstruye esa imagen (con caché, rápido), la transfiere y la
levanta. `release` no se mueve hacia atrás: el siguiente fast-forward vuelve
a desplegar lo último. Las cinco últimas imágenes quedan en el VPS.

Las migraciones de SQLite son aditivas, así que una base ya migrada sirve a la
versión anterior. Si algún día una migración dejara de serlo, el rollback
necesita además restaurar una copia (abajo).

## Ver qué está pasando

Por SSH, como `deployer`:

```bash
sudo docker compose --env-file /srv/openrouter/.env -f /srv/openrouter/vps/compose.yml ps
sudo docker logs --tail 200 openrouter
curl -s http://127.0.0.1:8081/salud
sudo systemctl status caddy --no-pager
sudo journalctl -u caddy --since "1 hour ago" --no-pager | tail -50
```

Sin SSH: `vps-inspeccionar.yml` a mano (solo lectura) deja un informe en el
resumen del run.

## Si `/v1/estado` dice `"almacen":"memoria"`

El contenedor no pudo abrir `/datos/uso.db`. Causas por orden de
probabilidad: `/srv/openrouter/datos` no es de uid 10001 (`vps-preparar.yml`
lo arregla), disco lleno (`df -h /`), fichero corrupto (restaurar copia). El
servicio sigue funcionando en memoria mientras tanto, pero cada reinicio
pierde el uso.

## Copias

- Se hacen solas cada noche. Comprobar: `sudo systemctl list-timers openrouter-copia.timer`
  y `sudo journalctl -u openrouter-copia.service --no-pager | tail`.
- A mano: `sudo /srv/openrouter/vps/copia.sh`.
- **Restaurar**:
  ```bash
  sudo docker compose --env-file /srv/openrouter/.env -f /srv/openrouter/vps/compose.yml stop
  sudo cp /srv/openrouter/datos/uso.db /srv/openrouter/datos/uso.db.roto
  sudo rm -f /srv/openrouter/datos/uso.db-wal /srv/openrouter/datos/uso.db-shm
  sudo gunzip -c /srv/openrouter/copias/uso-AAAAMMDD.db.gz > /tmp/uso.db
  sudo install -o 10001 -g 10001 -m 640 /tmp/uso.db /srv/openrouter/datos/uso.db
  sudo docker compose --env-file /srv/openrouter/.env -f /srv/openrouter/vps/compose.yml start
  ```
- **Probar que una copia sirve** (hacerlo una vez, y tras cualquier cambio de esquema):
  `gunzip -c copia.db.gz > /tmp/p.db && sqlite3 /tmp/p.db 'select count(*) from uso;'`
- Bajarlas al PC: `scp deployer@HOST:/srv/openrouter/copias/uso-*.db.gz .`
- Fuera del VPS: si existe `/srv/openrouter/rclone.conf` con una línea
  `# destino=remoto:carpeta`, `copia.sh` sube la carpeta con `rclone`.

## Rotar claves

| Clave | Cómo |
|---|---|
| SSH de despliegue | `tools/vps-clave.ps1` con la clave antigua borrada de `deploy_openrouter*` en el PC genera y sube una nueva; después quitar la línea antigua de `~/.ssh/authorized_keys` del VPS. |
| OpenRouter del VPS | Crear otra en OpenRouter (con límite), `gh secret set VPS_OPENROUTER_API_KEY`, relanzar `deploy-vps.yml` a mano, revocar la antigua en OpenRouter. |
| Administración (`VPS_SERVICIO_CLAVE`) | Generar, `gh secret set VPS_SERVICIO_CLAVE`, relanzar `deploy-vps.yml`. Las claves de aplicación no cambian. |
| De una aplicación | Crear nueva con `tools/apps.ps1 -Crear`, cambiarla en la aplicación, `-Baja` la antigua. Sin corte. |

Relanzar `deploy-vps.yml` a mano sin `sha` despliega el SHA de `release` con
los secretos actuales: es la forma de aplicar un secreto nuevo.

## Caddy

- Validar sin recargar: `sudo caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile`.
- Si no consigue certificado: `sudo journalctl -u caddy | grep -i acme`. Casi
  siempre es DNS (el registro A no apunta al VPS) o el 80 cerrado.
  `vps-inspeccionar.yml` comprueba las dos cosas.
- `desplegar.sh` valida antes de recargar y no recarga si el sitio nuevo no
  valida: los otros dominios del VPS no se ven afectados por un error nuestro.

## Tareas periódicas

| Cada | Qué |
|---|---|
| Semana | Mirar el gasto: `tools/apps.ps1 -Api https://apisor.oracle402.com -Gasto`. |
| Mes | `df -h /` y tamaño de `/srv/openrouter/copias`. Actualizar Docker y Caddy con `apt` (los parches de seguridad del sistema van solos). |
| Cambio de esquema | Probar la restauración de una copia. |

## Monitor externo

Un monitor gratuito (UptimeRobot, Better Stack, healthchecks.io) sobre
`https://apisor.oracle402.com/salud` cada 5 minutos, avisando por correo.
`/salud` no exige clave. [SUPUESTO] lo crea el usuario; no se automatiza.

## Fly

Fly sigue desplegando `main` como previsualización. Su base y sus claves de
aplicación son otras. Si se decide apagarlo: borrar `FLY_APP` de Fly, revocar
su clave de OpenRouter, y dejar `deploy.yml` como está (sin `FLY_API_TOKEN`
termina en verde sin hacer nada).
