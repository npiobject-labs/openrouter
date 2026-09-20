#!/usr/bin/env bash
# Verifica un despliegue del servicio desde fuera, con curl y jq. Lo llaman
# deploy.yml (Fly) y deploy-vps.yml (VPS): la verificacion es la misma o un
# dia diran cosas distintas.
#
#   tools/verificar-servicio.sh <url base> <sha esperado>
#
# Entorno opcional:
#   SERVICIO_CLAVE   clave de administracion; sin ella solo se comprueban las
#                    rutas publicas y el sobre de error.
#   CON_OPENROUTER   "1" si el despliegue tiene clave de OpenRouter: entonces
#                    se comprueban /v1/estado, /v1/models y /v1/uso, que
#                    hablan con OpenRouter sin gastar credito.
#
# Ninguna comprobacion gasta credito ni crea nada en el servicio.

set -u
base="${1:?url base}"; base="${base%/}"
sha="${2:?sha esperado}"
clave="${SERVICIO_CLAVE:-}"
con_openrouter="${CON_OPENROUTER:-0}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fallo() { echo "::error::$*"; exit 1; }
ok() { echo "OK: $*"; }
resumen() { [ -n "${GITHUB_STEP_SUMMARY:-}" ] && echo "$*" >> "$GITHUB_STEP_SUMMARY"; true; }

# --- /salud con el SHA del commit: es la prueba de que corre lo desplegado ---
echo "Verificando ${base}/salud (se espera build=${sha})"
for intento in $(seq 1 10); do
  respuesta="$(curl -fsS --max-time 20 "${base}/salud" || true)"
  echo "Intento ${intento}/10: ${respuesta:-<sin respuesta>}"
  if printf '%s' "${respuesta}" | grep -q "${sha}"; then
    ok "/salud responde con el build ${sha}"
    break
  fi
  [ "${intento}" -lt 10 ] && sleep 15
done
printf '%s' "${respuesta}" | grep -q "${sha}" || fallo "/salud no devolvio el SHA ${sha} tras 10 intentos."

# --- /holamundo: cuerpo exacto y CORS, lo que consume docs/holamundo.html ---
cabeceras="$(curl -fsS --max-time 20 -D - -o "$tmp/cuerpo" -H 'Origin: https://npiobject-labs.github.io' "${base}/holamundo")"
cuerpo="$(cat "$tmp/cuerpo")"
[ "${cuerpo}" = "holamundo" ] || fallo "/holamundo devolvio '${cuerpo}'"
printf '%s' "${cabeceras}" | grep -qi '^access-control-allow-origin: ' || fallo "/holamundo sin cabecera CORS"
ok "/holamundo responde 'holamundo' con CORS"

# --- Cabeceras que los clientes leen y las verificaciones no miraban ---
# Las dos las encontro el primer consumidor real: PowerShell 5.1 decodifica en
# Latin-1 lo que no declara charset, y el navegador rechaza en el preflight
# cualquier cabecera del contrato que el CORS no permita.
cabeceras_salud="$(curl -fsS --max-time 20 -D - -o /dev/null "${base}/salud")"
printf '%s' "${cabeceras_salud}" | grep -qi '^content-type: application/json; charset=utf-8' \
  || fallo "/salud no declara charset=utf-8 en el Content-Type: los clientes en PowerShell 5.1 leeran los acentos rotos."
ok "/salud declara application/json; charset=utf-8"

preflight="$(curl -fsS --max-time 20 -D - -o /dev/null -X OPTIONS "${base}/v1/chat/completions" \
  -H 'Origin: https://npiobject-labs.github.io' -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: authorization,content-type,x-operacion' || true)"
printf '%s' "${preflight}" | grep -i '^access-control-allow-headers:' | grep -qi 'x-operacion' \
  || fallo "el preflight de CORS no permite X-Operacion: docs/conectar.html y cualquier cliente web que siga el contrato quedan bloqueados."
ok "el preflight de CORS admite X-Operacion"

# --- El sobre de error es contrato publicado: si cambia, el run falla ---
comprueba_error() {
  ruta="$1"; esperado="$2"; codigo="$3"; tipo="$4"
  http="$(curl -s -o "$tmp/error.json" -w '%{http_code}' --max-time 20 "${base}${ruta}" || true)"
  echo "${ruta} -> HTTP ${http}"
  [ "${http}" = "${esperado}" ] || { cat "$tmp/error.json"; fallo "${ruta} respondio ${http} y se esperaba ${esperado}."; }
  jq -e --arg c "${codigo}" --arg t "${tipo}" \
    '.ok == false and (.error.message | type == "string" and length > 0) and .error.code == $c and .error.type == $t' \
    "$tmp/error.json" > /dev/null || { cat "$tmp/error.json"; fallo "${ruta} no devolvio el sobre de error esperado (${codigo}/${tipo})."; }
  ok "sobre de error $(jq -r '.error.code + " / " + .error.type' "$tmp/error.json")"
}
if [ -n "${clave}" ]; then
  comprueba_error "/v1/models" 401 sin_clave authentication_error
else
  # Sin SERVICIO_CLAVE configurada el servicio cierra /v1 con 503, y eso es lo correcto.
  comprueba_error "/v1/models" 503 sin_configurar api_error
fi
comprueba_error "/v1/ruta-que-no-existe" 404 ruta_desconocida not_found_error

if [ -z "${clave}" ]; then
  echo "Sin SERVICIO_CLAVE: no se comprueban las rutas autenticadas."
  resumen "Sin clave de servicio: rutas /v1 sin comprobar"
  exit 0
fi
auth=(-H "Authorization: Bearer ${clave}")

# --- /v1/apps: lista, y sin claves ni hashes ---
http="$(curl -s -o "$tmp/apps.json" -w '%{http_code}' --max-time 20 "${auth[@]}" "${base}/v1/apps" || true)"
[ "${http}" = "200" ] && jq -e '.object == "list" and (.data | type == "array")' "$tmp/apps.json" > /dev/null \
  || { cat "$tmp/apps.json"; fallo "/v1/apps no respondio la lista esperada (HTTP ${http})."; }
jq -e '[.data[] | keys] | flatten | map(select(. == "clave" or . == "hash")) | length > 0' "$tmp/apps.json" > /dev/null \
  && fallo "/v1/apps devolvio claves o hashes; eso no puede salir del servicio."
ok "$(jq -r '.total' "$tmp/apps.json") aplicaciones, sin claves en la respuesta"

# --- /v1/presupuesto: la administracion sigue sin topes ---
http="$(curl -s -o "$tmp/presupuesto.json" -w '%{http_code}' --max-time 20 "${auth[@]}" "${base}/v1/presupuesto" || true)"
[ "${http}" = "200" ] && jq -e '.administracion == true' "$tmp/presupuesto.json" > /dev/null \
  || { cat "$tmp/presupuesto.json"; fallo "/v1/presupuesto no reconocio la clave de administracion (HTTP ${http})."; }
ok "la administracion sigue sin limites"

# --- /v1/uso: solo la forma, el historico puede estar vacio ---
http="$(curl -s -o "$tmp/uso.json" -w '%{http_code}' --max-time 20 "${auth[@]}" "${base}/v1/uso?n=5" || true)"
[ "${http}" = "200" ] && jq -e '.object == "list" and (.data | type == "array") and (.total | type == "number")' "$tmp/uso.json" > /dev/null \
  || { cat "$tmp/uso.json"; fallo "/v1/uso no devolvio la lista esperada (HTTP ${http})."; }
ok "registro de uso con $(jq -r '.total' "$tmp/uso.json") llamadas"

if [ "${con_openrouter}" != "1" ]; then
  echo "Sin clave de OpenRouter en el despliegue: no se comprueban /v1/estado ni /v1/models."
  resumen "Sin clave de OpenRouter: /v1/estado y /v1/models sin comprobar"
  exit 0
fi

# --- /v1/estado: la conexion real con OpenRouter, sin gastar credito ---
for intento in $(seq 1 6); do
  # -o para no volcar el cuerpo: lleva la etiqueta y el saldo de la cuenta.
  http="$(curl -s -o "$tmp/estado.json" -w '%{http_code}' --max-time 30 "${auth[@]}" "${base}/v1/estado" || true)"
  echo "Intento ${intento}/6: HTTP ${http}"
  [ "${http}" = "200" ] && grep -q '"ok":true' "$tmp/estado.json" && break
  [ "${intento}" -lt 6 ] && sleep 10
done
[ "${http}" = "200" ] && grep -q '"ok":true' "$tmp/estado.json" || fallo "/v1/estado no confirmo la conexion con OpenRouter (ultimo HTTP ${http})."
almacen="$(jq -r '.almacen' "$tmp/estado.json")"
[ "${almacen}" = "sqlite" ] || fallo "el historico esta en '${almacen}', no en disco: revisa el montaje de /datos."
ok "el servicio habla con OpenRouter; historico en sqlite con $(jq -r '.registros' "$tmp/estado.json") registros"

# --- /v1/models: el catalogo trae modelos de verdad ---
http="$(curl -s -o "$tmp/modelos.json" -w '%{http_code}' --max-time 40 "${auth[@]}" "${base}/v1/models" || true)"
[ "${http}" = "200" ] || fallo "/v1/models respondio ${http}."
n="$(jq -r '.data | length' "$tmp/modelos.json" 2> /dev/null || echo 0)"
[ "${n}" -gt 0 ] || fallo "el catalogo vino vacio o ilegible"
ok "el catalogo trae ${n} modelos"

resumen "Verificacion completa en verde: /salud=${sha}, OpenRouter conectado, ${n} modelos"
