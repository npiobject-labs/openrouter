#!/usr/bin/env bash
# Despliega la imagen openrouter-backend:<sha>, que deploy-vps.yml ya ha
# cargado en el docker del VPS, y actualiza el sitio de Caddy.
#
#   sudo vps/desplegar.sh <sha>
#
# Lee PUERTO_INTERNO de /srv/openrouter/.env y anota IMAGEN_TAG=<sha> en el
# mismo fichero, que es lo que hace posible el rollback: el mismo comando con
# el sha anterior.
set -euo pipefail

SHA="${1:?sha de la imagen}"
DOMINIO="${API_DOMINIO:-apisor.oracle402.com}"
RAIZ=/srv/openrouter
ENV="$RAIZ/.env"
SITIO="/etc/caddy/sites.d/${DOMINIO}.caddy"
AQUI="$(cd "$(dirname "$0")" && pwd)"

[ "$(id -u)" = 0 ] || { echo "::error::hay que ejecutarlo con sudo"; exit 1; }
[ -f "$ENV" ] || { echo "::error::falta ${ENV}: escribir-env.sh va antes"; exit 1; }
docker image inspect "openrouter-backend:${SHA}" > /dev/null 2>&1 \
  || { echo "::error::la imagen openrouter-backend:${SHA} no esta cargada"; exit 1; }

puerto="$(grep -E '^PUERTO_INTERNO=' "$ENV" | cut -d= -f2)"
[ -n "$puerto" ] || { echo "::error::PUERTO_INTERNO no esta en ${ENV}"; exit 1; }

# IMAGEN_TAG=<sha> en el .env: compose lo interpola en la imagen.
if grep -qE '^IMAGEN_TAG=' "$ENV"; then
  sed -i "s/^IMAGEN_TAG=.*/IMAGEN_TAG=${SHA}/" "$ENV"
else
  printf 'IMAGEN_TAG=%s\n' "$SHA" >> "$ENV"
fi
docker tag "openrouter-backend:${SHA}" openrouter-backend:release

# --- Caddy: validar antes de recargar; si falla, la configuracion anterior sigue ---
sed "s/__PUERTO_INTERNO__/${puerto}/g" "$AQUI/apisor.caddy" > "${SITIO}.nuevo"
if ! caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile > /dev/null 2>&1; then
  rm -f "${SITIO}.nuevo"
  echo "::error::el Caddyfile actual no valida; no se toca"; exit 1
fi
mv "${SITIO}.nuevo" "$SITIO"
if ! caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile; then
  echo "::error::el sitio nuevo no valida; Caddy sigue con el anterior cargado"; exit 1
fi
systemctl reload caddy
echo "- caddy recargado con ${DOMINIO} -> 127.0.0.1:${puerto}"

# --- El contenedor: up con espera al healthcheck ---
cd "$RAIZ/vps"
docker compose --env-file "$ENV" -f compose.yml up -d --wait --remove-orphans --wait-timeout 90
docker compose --env-file "$ENV" -f compose.yml ps
echo "- desplegado openrouter-backend:${SHA} en 127.0.0.1:${puerto}"

# --- Imagenes viejas: se guardan las cinco ultimas para poder volver atras ---
docker images openrouter-backend --format '{{.Tag}} {{.CreatedAt}}' \
  | grep -v '^release ' | sort -k2 -r | awk 'NR > 5 {print $1}' \
  | xargs -r -I{} docker rmi "openrouter-backend:{}" > /dev/null 2>&1 || true
echo "Despliegue terminado."
