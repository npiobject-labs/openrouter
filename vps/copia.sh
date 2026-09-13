#!/usr/bin/env bash
# Copia diaria del historico. Lo lanza openrouter-copia.timer como root.
# - sqlite3 .backup: copia consistente aunque el servicio este escribiendo.
# - Rotacion: se borran las de mas de 30 dias.
# - Los lunes, ademas, el historico en CSV via la propia API.
# - Aviso si a la clave de OpenRouter le queda menos del 20 % del limite.
# - Si existe /srv/openrouter/rclone.conf, se sube la carpeta con rclone.
set -uo pipefail
RAIZ=/srv/openrouter
BD="$RAIZ/datos/uso.db"
COPIAS="$RAIZ/copias"
HOY="$(date +%Y%m%d)"

[ -f "$BD" ] || { echo "sin base en ${BD}; nada que copiar"; exit 0; }
mkdir -p "$COPIAS"

sqlite3 "$BD" ".backup '${COPIAS}/uso-${HOY}.db'" && gzip -f "${COPIAS}/uso-${HOY}.db" \
  && echo "copia ${COPIAS}/uso-${HOY}.db.gz ($(du -h "${COPIAS}/uso-${HOY}.db.gz" | cut -f1))" \
  || echo "ERROR: la copia de ${BD} fallo"

find "$COPIAS" -name 'uso-*.db.gz' -mtime +30 -delete
find "$COPIAS" -name 'uso-*.csv' -mtime +90 -delete

# Lo que sigue necesita la clave de administracion, que esta en el .env.
puerto="$(grep -E '^PUERTO_INTERNO=' "$RAIZ/.env" 2> /dev/null | cut -d= -f2)"
clave="$(grep -E '^SERVICIO_CLAVE=' "$RAIZ/.env" 2> /dev/null | cut -d= -f2-)"
if [ -n "$puerto" ] && [ -n "$clave" ]; then
  if [ "$(date +%u)" = 1 ]; then
    curl -fsS --max-time 60 -H "Authorization: Bearer ${clave}" \
      "http://127.0.0.1:${puerto}/v1/uso/exportar?formato=csv" -o "${COPIAS}/uso-${HOY}.csv" \
      && echo "exportado ${COPIAS}/uso-${HOY}.csv" || echo "aviso: la exportacion CSV fallo"
  fi
  estado="$(curl -fsS --max-time 30 -H "Authorization: Bearer ${clave}" "http://127.0.0.1:${puerto}/v1/estado" 2> /dev/null || true)"
  if [ -n "$estado" ] && command -v jq > /dev/null; then
    limite="$(printf '%s' "$estado" | jq -r '.clave.limit // empty')"
    restante="$(printf '%s' "$estado" | jq -r '.clave.limit_remaining // empty')"
    if [ -n "$limite" ] && [ -n "$restante" ] && [ "$limite" != "null" ]; then
      if awk -v r="$restante" -v l="$limite" 'BEGIN { exit !(l > 0 && r / l < 0.2) }'; then
        echo "AVISO: a la clave de OpenRouter le queda ${restante} de ${limite} (menos del 20 %)"
        printf 'Fecha %s: quedan %s de %s en la clave de OpenRouter.\n' "$(date -Is)" "$restante" "$limite" > "${COPIAS}/AVISO-saldo.txt"
      else
        rm -f "${COPIAS}/AVISO-saldo.txt"
      fi
    fi
  fi
fi

if [ -f "$RAIZ/rclone.conf" ] && command -v rclone > /dev/null; then
  destino="$(grep -E '^# destino=' "$RAIZ/rclone.conf" | cut -d= -f2)"
  [ -n "$destino" ] && rclone --config "$RAIZ/rclone.conf" copy "$COPIAS" "$destino" && echo "copias subidas a ${destino}"
fi
echo "copias en disco: $(ls "$COPIAS" | wc -l) ficheros, $(du -sh "$COPIAS" | cut -f1)"
