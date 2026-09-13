#!/usr/bin/env bash
# Prepara el VPS para el servicio. Idempotente: se puede repetir sin efecto.
# Lo ejecuta .github/workflows/vps-preparar.yml con sudo. No instala ni toca
# nada que ya exista salvo: las carpetas de /srv/openrouter, la linea "import"
# del Caddyfile (una sola vez), el paquete sqlite3 (para las copias) y el
# timer de las copias.
set -euo pipefail

DOMINIO="${API_DOMINIO:-apisor.oracle402.com}"
RAIZ=/srv/openrouter
SITIO="/etc/caddy/sites.d/${DOMINIO}.caddy"
AQUI="$(cd "$(dirname "$0")" && pwd)"

echo "### Preparacion de ${DOMINIO}"
[ "$(id -u)" = 0 ] || { echo "::error::hay que ejecutarlo con sudo"; exit 1; }

# --- Lo que tiene que existir ya, y no se instala desde aqui ---------------
for orden in docker caddy ufw; do
  command -v "$orden" > /dev/null || { echo "::error::falta ${orden} en el VPS"; exit 1; }
done
docker compose version > /dev/null || { echo "::error::falta el plugin docker compose"; exit 1; }
systemctl is-active --quiet caddy || { echo "::error::el servicio caddy no esta activo"; exit 1; }
systemctl is-active --quiet docker || { echo "::error::el servicio docker no esta activo"; exit 1; }
systemctl is-enabled --quiet docker || { echo "::warning::docker no arranca solo al reiniciar"; }
[ -d /etc/caddy/sites.d ] || { echo "::error::no existe /etc/caddy/sites.d: la convencion del VPS ha cambiado"; exit 1; }
echo "- docker $(docker version --format '{{.Server.Version}}'), compose $(docker compose version --short), $(caddy version | cut -d' ' -f1)"

# Que 80/443 los sirva Caddy, y no otra cosa que haya aparecido desde la inspeccion.
for p in 80 443; do
  quien="$(ss -ltnpH "sport = :$p" | grep -o 'users:(("[^"]*"' | head -1 | cut -d'"' -f2 || true)"
  [ "$quien" = "caddy" ] || { echo "::error::el puerto ${p} lo sirve '${quien:-nadie}', no caddy"; exit 1; }
done
echo "- 80 y 443: caddy"

# Cortafuegos y fail2ban: se comprueban, no se cambian.
ufw status | grep -q "Status: active" && echo "- ufw activo" || echo "::warning::ufw no esta activo"
systemctl is-active --quiet fail2ban && echo "- fail2ban activo" || echo "::warning::fail2ban no esta activo"
timedatectl show -p NTPSynchronized --value | grep -q yes && echo "- reloj sincronizado" || echo "::warning::NTP sin sincronizar"

libres_gb="$(df -BG --output=avail / | tail -1 | tr -dc '0-9')"
[ "$libres_gb" -ge 5 ] || { echo "::error::solo ${libres_gb} GB libres en /; hacen falta 5"; exit 1; }
echo "- ${libres_gb} GB libres"

# --- Carpetas: datos del contenedor (uid 10001) y copias (solo root) --------
mkdir -p "$RAIZ/datos" "$RAIZ/copias" "$RAIZ/vps"
chown 10001:10001 "$RAIZ/datos"
chmod 750 "$RAIZ/datos"
chmod 700 "$RAIZ/copias"
echo "- carpetas en ${RAIZ}"

# --- sqlite3 para las copias consistentes -----------------------------------
if ! command -v sqlite3 > /dev/null; then
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q sqlite3 > /dev/null
  echo "- sqlite3 instalado"
else
  echo "- sqlite3 presente"
fi

# --- La unica linea que se toca de una configuracion ajena -----------------
if grep -qF "import ${SITIO}" /etc/caddy/Caddyfile; then
  echo "- el Caddyfile ya importa ${SITIO}"
else
  cp /etc/caddy/Caddyfile "/etc/caddy/Caddyfile.antes-de-${DOMINIO}"
  printf '\nimport %s\n' "${SITIO}" >> /etc/caddy/Caddyfile
  echo "- anadida la linea import al Caddyfile (copia en Caddyfile.antes-de-${DOMINIO})"
fi
# El sitio importado tiene que existir o Caddy no recarga: si aun no se ha
# desplegado, se deja uno vacio que solo declara el dominio.
if [ ! -f "$SITIO" ]; then
  printf '%s {\n\trespond "openrouter: pendiente de desplegar" 503\n}\n' "$DOMINIO" > "$SITIO"
  echo "- sitio provisional en ${SITIO}"
fi
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile > /dev/null
systemctl reload caddy
echo "- caddy validado y recargado"

# --- Copias diarias ---------------------------------------------------------
# vps/ ya esta en $RAIZ/vps (lo deja el workflow); si se ejecuta desde otro
# sitio, se copia. El timer apunta siempre a $RAIZ/vps/copia.sh.
[ "$AQUI" = "$RAIZ/vps" ] || cp "$AQUI/copia.sh" "$RAIZ/vps/copia.sh"
chmod 755 "$RAIZ/vps/copia.sh"
install -m 644 "$AQUI/openrouter-copia.service" /etc/systemd/system/openrouter-copia.service
install -m 644 "$AQUI/openrouter-copia.timer" /etc/systemd/system/openrouter-copia.timer
systemctl daemon-reload
systemctl enable --now openrouter-copia.timer > /dev/null 2>&1
echo "- timer de copias: $(systemctl is-active openrouter-copia.timer), proxima $(systemctl show openrouter-copia.timer -p NextElapseUSecRealtime --value)"

echo "Preparacion terminada."
