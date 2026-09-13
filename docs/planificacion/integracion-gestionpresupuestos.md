# Integrar GestiónPresupuestos con el servicio openrouter

Guía para la primera aplicación consumidora. Vale igual para cualquier otra:
donde dice GestiónPresupuestos, léase «tu aplicación».

## Lo mínimo

| Qué | Valor |
|---|---|
| URL base | `https://apisor.oracle402.com/v1` |
| Autenticación | `Authorization: Bearer <clave de aplicación>` |
| Formato | El de la API de OpenAI donde hay equivalente (`/chat/completions`, `/models`) |
| Contrato completo | https://npiobject-labs.github.io/openrouter/api.html (`docs/openapi.json`) |
| Consola para probar a mano | https://npiobject-labs.github.io/openrouter/consola.html |

Con el SDK de OpenAI en Python es cambiar dos parámetros:

```python
from openai import OpenAI
cliente = OpenAI(base_url="https://apisor.oracle402.com/v1", api_key=CLAVE_DE_APLICACION)
r = cliente.chat.completions.create(
    model="google/gemini-2.5-flash-lite",
    messages=[{"role": "user", "content": "..."}],
    max_tokens=800,
    extra_headers={"X-Operacion": "presupuesto-2026-0417"},
)
```

Con `curl`:

```bash
curl https://apisor.oracle402.com/v1/chat/completions \
  -H "Authorization: Bearer $CLAVE" -H "Content-Type: application/json" \
  -H "X-Operacion: presupuesto-2026-0417" \
  -d '{"messages":[{"role":"user","content":"Di hola"}],"max_tokens":50}'
```

Con PowerShell, `tools/apps.ps1` y `tools/probar-bom.ps1` son ejemplos completos
(`Invoke-RestMethod` con la cabecera `Authorization`). Ambos aceptan `-Api
https://apisor.oracle402.com`.

## Alta de la aplicación (una vez, con la clave de administración)

Desde el PC:

```powershell
pwsh -File tools\apps.ps1 -Api https://apisor.oracle402.com -Crear GestionPresupuestos
```

Enseña la clave **una sola vez**. Va directa al gestor de secretos del servidor
de GestiónPresupuestos, nunca a un fichero del repo ni al navegador. Si se
pierde, se crea otra y se revoca la anterior (`-Baja <id>`); no hay forma de
recuperarla porque el servicio solo guarda su hash.

Ponle un presupuesto antes de la primera llamada real (con la clave de
administración; los valores son un punto de partida):

```bash
curl -X PUT https://apisor.oracle402.com/v1/apps/<id>/presupuesto \
  -H "Authorization: Bearer $ADMIN" -H "Content-Type: application/json" \
  -d '{"periodo":"mes","limite":5,"aviso":4,"cuota_minuto":30}'
```

Una clave por aplicación **y por entorno**: la GestiónPresupuestos de pruebas y
la real tienen claves y presupuestos distintos.

## Reglas que conviene seguir en el código de la aplicación

1. **La clave vive en el servidor de la aplicación.** Si la interfaz de
   GestiónPresupuestos corre en el navegador, el navegador llama al servidor de
   GestiónPresupuestos y ese servidor llama aquí. Una clave en el navegador la
   tiene cualquier usuario.
2. **`X-Operacion` en cada llamada** con el identificador del presupuesto que se
   está analizando. Después, `GET /v1/uso/resumen?agrupar=operacion` o
   `GET /v1/uso?operacion=<id>` dicen cuánto costó cada uno. Y el campo `user`
   del cuerpo con el usuario de la aplicación, que OpenRouter ya entiende.
3. **Antes de un trabajo grande, `GET /v1/presupuesto`**: devuelve el margen
   del periodo. Mejor no empezar un análisis de cien llamadas que se va a
   cortar a la mitad.
4. **Guarda `X-Uso-Id`** de cada respuesta junto al resultado: es el registro de
   uso con tokens, coste y latencia, consultable en `GET /v1/uso/{id}`.
5. **Lee `X-Presupuesto` si llega**: es el aviso de umbral. Enséñalo, no lo
   ocultes.
6. **Timeout del cliente por encima de 120 s** en `/v1/chat/completions`. El
   servicio concede 120 s a OpenRouter; el que corta tiene que ser el servicio,
   que anota la llamada.
7. **Salida estructurada**: `response_format` con esquema, `max_tokens`, y
   `maxLength` en los campos de texto. El modelo se elige por la prueba con
   datos reales, no por precio: para mapear columnas de un BOM, el modelo por
   defecto (`google/gemini-2.5-flash-lite`) igualó a los caros a una fracción
   del coste. Detalle en `prueba-bom.md`.

## Errores: todos con el mismo sobre

```json
{"ok": false, "error": {"message": "...", "code": "...", "type": "..."}}
```

| HTTP | `code` | Qué hacer |
|---|---|---|
| 401 | `sin_clave`, `clave_invalida` | Configuración: la clave falta, es otra o la aplicación está dada de baja. No reintentar. |
| 402 | `presupuesto_agotado` | Parar y avisar al usuario. No reintentar hasta el siguiente periodo o hasta que administración suba el tope. |
| 429 | `cuota_superada` | Esperar un minuto y reintentar. |
| 429 | `bucle` | Algo está repitiendo la misma petición (cinco iguales en un minuto). Parar: es un bug, no una carga. |
| 400 | `cuerpo_invalido`, `streaming_no_disponible` | Petición mal formada, o `stream: true` antes de la etapa 7; corregir. |
| 502/504 | `openrouter_rechaza`, `openrouter_inalcanzable`, `openrouter_tardo_demasiado` | Fallo del proveedor; con `openrouter_rechaza`, `error.upstream` trae su cuerpo original. Reintentar una o dos veces con espera creciente. |
| 503 | `sin_configurar` | El servicio no tiene sus claves. Avisar al administrador. |

**Sobre los reintentos**: el cortacircuitos mira el hash del cuerpo. Reintentar
la misma petición idéntica cinco veces en un minuto se corta con `429 bucle`
aunque cada intento fuera legítimo. Reintenta como mucho dos veces, con
espera creciente (2 s, 8 s), y si el fallo persiste, deja de insistir y
regístralo.

## Qué puede ver GestiónPresupuestos con su clave

- `GET /v1/models`: el catálogo entero, con precios.
- `GET /v1/presupuesto`: su margen.
- `GET /v1/uso`, `/v1/uso/{id}`, `/v1/uso/resumen`, `/v1/uso/exportar`: solo
  sus llamadas. Un registro de otra aplicación responde `404` como si no
  existiera.
- `GET /v1/estado`: el estado del servicio y el nombre de la clave de OpenRouter, sin su uso ni su límite, que son de la cuenta.

Lo que no puede: crear aplicaciones, ver el gasto de otras, cambiar su propio
presupuesto. Eso es de la clave de administración, que GestiónPresupuestos no
conoce.

## Rotación de la clave sin corte

1. Crear una clave nueva (`tools/apps.ps1 -Crear GestionPresupuestos-2`).
2. Cambiarla en el gestor de secretos de GestiónPresupuestos y desplegar.
3. Revocar la antigua (`-Baja <id antiguo>`). Su histórico se conserva.

## Prueba de punta a punta

Desde el PC, con la clave de aplicación (no la de administración) y un BOM
real:

```powershell
pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Api https://apisor.oracle402.com
```

Demuestra el circuito entero: clave de aplicación, presupuesto, `X-Operacion`,
salida estructurada, y la llamada medida en `GET /v1/uso/resumen?agrupar=app`.
