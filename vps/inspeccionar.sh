#!/usr/bin/env bash
# Inspeccion de solo lectura del VPS, pensada para ejecutarse por SSH desde
# .github/workflows/vps-inspeccionar.yml. No instala, no cambia ni escribe nada.
# Responde a: ¿que proxy sirve 80/443 (Caddy, nginx, otro)? ¿que dominios
# atiende ya? ¿que puerto interno queda libre para el backend? ¿resuelve el
# subdominio a esta maquina?
#
# No imprime contenido de ficheros de configuracion, solo sus cabeceras de
# sitio: un Caddyfile puede llevar hashes de basic_auth o rutas internas.

set -u
DOMINIO="${API_DOMINIO:-apisor.oracle402.com}"

titulo() { printf '\n### %s\n\n' "$1"; }
tiene() { command -v "$1" > /dev/null 2>&1; }

# sudo sin contrasena si lo hay; si no, lo mismo sin privilegios (ss no
# ensena entonces el nombre del proceso, pero si el puerto).
if sudo -n true 2> /dev/null; then SUDO="sudo -n"; else SUDO=""; fi

titulo "Maquina"
echo "- host: $(hostname)"
echo "- sistema: $(. /etc/os-release 2> /dev/null && echo "${PRETTY_NAME:-?}")"
echo "- nucleo: $(uname -r)"
echo "- arranque: $(uptime -p 2> /dev/null || uptime)"
echo "- sudo sin contrasena: $([ -n "$SUDO" ] && echo si || echo no)"
echo "- disco en /: $(df -h / | awk 'NR==2 {print $4 " libres de " $2}')"
echo "- memoria: $(free -m | awk '/^Mem:/ {print $7 " MB disponibles de " $2}')"
echo "- hora: $(date -Is) · NTP: $(timedatectl show -p NTPSynchronized --value 2> /dev/null || echo '?')"

titulo "Puertos a la escucha"
if tiene ss; then
  $SUDO ss -ltnpH 2> /dev/null | awk '{print $4, $6}' | sed 's/users:((//; s/))//' | sort -u | sed 's/^/- /'
else
  $SUDO netstat -ltnp 2> /dev/null | awk 'NR>2 {print "- " $4, $7}'
fi

titulo "Quien sirve 80 y 443"
for p in 80 443; do
  proceso="$($SUDO ss -ltnpH "sport = :$p" 2> /dev/null | grep -o 'users:(("[^"]*"' | head -1 | cut -d'"' -f2)"
  if [ -n "$proceso" ]; then
    echo "- $p: $proceso"
  elif $SUDO ss -ltnH "sport = :$p" 2> /dev/null | grep -q .; then
    echo "- $p: ocupado (proceso no visible sin sudo)"
  else
    echo "- $p: libre"
  fi
done

titulo "Proxies conocidos"
for s in caddy nginx apache2 httpd traefik haproxy; do
  if tiene "$s" || systemctl list-unit-files "$s.service" 2> /dev/null | grep -q "$s.service"; then
    echo "- $s: binario $(tiene "$s" && echo si || echo no), servicio $(systemctl is-active "$s" 2> /dev/null || echo 'sin unidad')"
  fi
done

titulo "Caddy"
if tiene caddy; then
  echo "- version: $(caddy version 2> /dev/null | head -1)"
fi
for f in /etc/caddy/Caddyfile /etc/caddy/conf.d/*.caddy /etc/caddy/sites/*; do
  [ -e "$f" ] || continue
  echo "- fichero: $f"
  # Solo las cabeceras de bloque de sitio (lineas sin sangria que abren llave):
  # los dominios que ya atiende, nada de lo que hay dentro.
  $SUDO grep -E '^[^[:space:]#].*\{[[:space:]]*$' "$f" 2> /dev/null | sed 's/[[:space:]]*{[[:space:]]*$//; s/^/    - sitio: /'
done
# La API de administracion de Caddy, si escucha, da la lista real de hosts.
if tiene curl && curl -fs --max-time 3 http://127.0.0.1:2019/config/ > /tmp/caddy-config.json 2> /dev/null; then
  echo "- API de administracion en 2019: si"
  if tiene jq; then
    echo "- hosts en la configuracion cargada:"
    jq -r '.. | objects | select(has("host")) | .host[]?' /tmp/caddy-config.json 2> /dev/null | sort -u | sed 's/^/    - /'
    echo "- puertos de escucha de Caddy:"
    jq -r '.apps.http.servers // {} | to_entries[] | "    - \(.key): \(.value.listen | join(", "))"' /tmp/caddy-config.json 2> /dev/null
  else
    grep -o '"host":\[[^]]*\]' /tmp/caddy-config.json | sort -u | sed 's/^/    - /'
  fi
  rm -f /tmp/caddy-config.json
else
  echo "- API de administracion en 2019: no responde (o Caddy no esta)"
fi
for d in /var/lib/caddy/.local/share/caddy/certificates /root/.local/share/caddy/certificates "$HOME/.local/share/caddy/certificates"; do
  if $SUDO test -d "$d"; then
    echo "- certificados ya emitidos ($d):"
    $SUDO find "$d" -mindepth 2 -maxdepth 2 -type d -printf '    - %f\n' 2> /dev/null
  fi
done

titulo "Docker"
if tiene docker; then
  echo "- version: $(docker version --format '{{.Server.Version}}' 2> /dev/null || $SUDO docker version --format '{{.Server.Version}}' 2> /dev/null || echo 'sin acceso al daemon')"
  echo "- compose: $(docker compose version --short 2> /dev/null || echo no)"
  echo "- contenedores:"
  { docker ps --format '    - {{.Names}} ({{.Image}}) {{.Ports}}' 2> /dev/null || $SUDO docker ps --format '    - {{.Names}} ({{.Image}}) {{.Ports}}' 2> /dev/null; } || echo "    - (sin acceso)"
else
  echo "- no instalado"
fi

titulo "Puerto interno libre para el backend"
ocupados="$($SUDO ss -ltnH 2> /dev/null | awk '{print $4}' | sed 's/.*://' | sort -un)"
libre=""
for p in 8080 8081 8082 8083 8084 8085 8090 8100 8180 8280 9080; do
  if ! printf '%s\n' "$ocupados" | grep -qx "$p"; then libre="$p"; break; fi
done
echo "- ocupados en el rango 8000-9999: $(printf '%s\n' "$ocupados" | awk '$1>=8000 && $1<=9999' | tr '\n' ' ')"
echo "- primer candidato libre: ${libre:-ninguno de la lista}"

titulo "DNS de $DOMINIO visto desde el VPS"
ip_publica="$(curl -4fs --max-time 5 https://api.ipify.org 2> /dev/null || curl -4fs --max-time 5 https://ifconfig.me 2> /dev/null || echo '?')"
echo "- IP publica de esta maquina: $ip_publica"
resuelto="$(getent ahostsv4 "$DOMINIO" 2> /dev/null | awk '{print $1}' | sort -u | tr '\n' ' ')"
echo "- $DOMINIO resuelve a: ${resuelto:-nada}"
if [ -n "$resuelto" ] && printf '%s' "$resuelto" | grep -qw "$ip_publica"; then
  echo "- veredicto DNS: apunta a esta maquina"
elif [ -z "$resuelto" ]; then
  echo "- veredicto DNS: sin registro A todavia"
else
  echo "- veredicto DNS: apunta a otra IP (¿proxy de Cloudflare u otro servidor?)"
fi
raiz="$(getent ahostsv4 "${DOMINIO#*.}" 2> /dev/null | awk '{print $1}' | sort -u | tr '\n' ' ')"
echo "- ${DOMINIO#*.} resuelve a: ${raiz:-nada}"

titulo "Cortafuegos"
if tiene ufw; then
  $SUDO ufw status 2> /dev/null | sed 's/^/- /' || echo "- ufw presente, estado no legible sin sudo"
else
  echo "- ufw no instalado"
fi
if tiene fail2ban-client; then
  echo "- fail2ban: $(systemctl is-active fail2ban 2> /dev/null)"
fi

echo
echo "Fin de la inspeccion. Nada se ha modificado."
