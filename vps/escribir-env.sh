#!/usr/bin/env bash
# Escribe /srv/openrouter/.env con lo que llega por la entrada estandar, solo
# legible por root. Los valores no pasan por argumentos ni por el log.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "hay que ejecutarlo con sudo" >&2; exit 1; }
umask 077
mkdir -p /srv/openrouter
cat > /srv/openrouter/.env.nuevo
mv /srv/openrouter/.env.nuevo /srv/openrouter/.env
echo "env escrito: $(grep -c '=' /srv/openrouter/.env) variables (valores ocultos)"
