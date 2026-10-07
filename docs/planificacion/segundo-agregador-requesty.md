# Segundo agregador para el gateway: Requesty

Fecha: 06/10/2026. Escrito en una sesión de maydom y traído aquí, que es donde va el trabajo: el segundo agregador vive en este gateway y mercamodels lo aprovecha después (enseñar y lanzar modelos de Requesty, con ofertas por proveedor y región). Pregunta: ¿es Requesty el agregador más parecido a OpenRouter y sirve como segundo agregador del gateway `npiobject-labs/openrouter`, para que las apps (maydom la primera) dejen de depender de uno solo? Contexto: [`alternativas-a-openrouter.md`](alternativas-a-openrouter.md) (investigación del 05/10/2026).

**Cómo se ha comprobado.** El sandbox de la sesión no alcanza ni a Requesty ni a OpenRouter, así que un workflow temporal en el repo de maydom (run 37436216507, 06/10/2026, sin claves) descargó desde el runner los dos catálogos públicos, los comparó modelo a modelo y leyó la documentación de Requesty (`docs.requesty.ai/llms.txt` y 16 páginas) y sus páginas de precios, UE, privacidad, seguridad, DPA y estado. Todo lo que sigue sin marca está leído así, en fuente primaria. Lo que exige una cuenta (una llamada real, la factura) va como [SUPUESTO].

## Veredicto

**Sí: Requesty es el que más se parece a OpenRouter y el mejor candidato a segundo agregador.** Es de los pocos que da las dos piezas que el gateway usa para medir el gasto —catálogo con precios por API y coste real de cada llamada—, no pasa por OpenRouter (va directo a Vertex, Bedrock, Azure, DeepInfra, Mistral…, así que no cae con él) y tiene un endpoint en Fráncfort sin coste extra. Además, su catálogo dice de cada modelo **dónde procesa, si retiene y si entrena**, algo que OpenRouter no publica así y que viene muy bien para los datos de finanzas y salud de maydom.

Hay tres cosas que no son como se esperaba y que condicionan el diseño:

1. **Los ids de modelo no coinciden.** Requesty nombra por *proveedor*, no por *fabricante*: `vertex/gemini-2.5-flash-lite@europe-west1`, `bedrock/claude-sonnet-5@eu-central-1`, `azure/gpt-5-mini@swedencentral`. De los 464 ids de OpenRouter solo 69 existen igual en Requesty, y por nombre canónico se encuentran 138 de 371 de pago (con diferencias de formato como `claude-sonnet-4.5` frente a `claude-sonnet-4-5` y sufijos de fecha en DeepSeek, la cobertura real es mayor). El gateway necesita una **tabla de equivalencias**, no basta con cambiar la URL.
2. **El endpoint UE no basta para que todo quede en la UE.** `router.eu.requesty.ai` garantiza que *Requesty* procesa en Fráncfort, pero el modelo corre donde diga su id: hay que pedir uno `@eu`/`@europe-…` (o restringir la clave a modelos UE). Con `google/…` o `anthropic/…` la inferencia sale de la UE.
3. **Los logs están encendidos por defecto.** En los planes de autoservicio Requesty guarda prompt y respuesta cifrados en la UE 30 días, salvo que se apague el log **por clave** (o se pida por escrito la retención cero de toda la organización). La web de la UE dice «0 data retention», pero la política de privacidad, la página de seguridad y el DPA dicen lo de los 30 días: manda lo segundo.

## Requesty frente a OpenRouter

| | OpenRouter | Requesty |
|---|---|---|
| Modelos en el catálogo público | 464 | 754 (los mismos por el endpoint global y el UE) |
| Precios del catálogo | Precio de lista | Precio de lista: en los 136 modelos emparejados la mediana es exactamente 1,000 |
| Comisión | 5,5 % al recargar con tarjeta (mín. 0,80 $) | 5 % sobre lo que se gasta («10 $/M de OpenAI cuestan 10,50 $»); sin mínimo ni cuota |
| Coste de cada llamada | `usage.cost` (y `GET /generation`) | `usage.cost` en toda respuesta sin pedirlo; en streaming, con `stream_options.include_usage` |
| Formato del catálogo | Precio por token en `pricing.prompt/completion` | Precio por token en `input_price/output_price`, más tramos por tamaño de prompt y caché |
| Datos por modelo | — | `geolocation` (eu 213, global 391, us 117, sg 21, uk 8, ap 4), `data_retention`, `data_retention_days`, `data_used_for_training` (9 entrenan), `quantization`, `supports_output_json_schema` |
| Región UE | `eu.openrouter.ai`, solo plan Business o Enterprise | `router.eu.requesty.ai` en todos los planes, incluido el gratuito |
| Retención | Solo metadatos por defecto; ZDR como filtro | Log de 30 días por defecto; se apaga por clave |
| Enrutado propio | `provider.order/only/zdr/…`, respaldo entre proveedores | Políticas `policy/<nombre>` de respaldo, reparto de carga y latencia, configuradas en su panel |
| Límites | Solo en modelos gratuitos | Peticiones simultáneas («in-flight»), no por minuto [cifra no publicada] |
| API | OpenAI chat completions (+ Messages) | OpenAI chat completions, Messages de Anthropic, `Authorization: Bearer` o `X-Api-Key` |
| Estado | ~99,97 % a 30 días según terceros | Página propia (Better Stack): 100 % a 30 días el 06/10/2026, sin incidencias listadas |
| Empresa | Comprada por Stripe (acuerdo del 19/08/2026) | Requesty Ltd (Londres), semilla de 3 M$ en 2025; SOC 2 Type II «en curso» |

¿Por qué no Vercel AI Gateway, que no cobra margen? Porque no consta que procese en la UE ni que su catálogo marque retención y región por modelo, y en septiembre tuvo quejas de 429 persistentes. Queda como plan B si Requesty falla en la prueba con clave. Cloudflare solo estima el coste; los demás (Eden AI, Opper, Cortecs) no se han podido comprobar en fuente primaria.

## Lo que encontraría maydom en Requesty (precios por millón de tokens, sin el 5 %)

| Uso | Hoy por OpenRouter | En Requesty, procesando en la UE |
|---|---|---|
| Extraer JSON (lo de casi todas las secciones) | `google/gemini-2.5-flash-lite` 0,10/0,40 | `vertex/gemini-2.5-flash-lite@europe-west1` **0,10/0,40, mismo precio**, sin retención, con `json_schema` · hasta que Vertex lo apague el 20/10/2026 |
| Sustitutos UE de la extracción | — | `nebius/openai/gpt-oss-120b` 0,15/0,60 · `mistral/mistral-small-2603` 0,165/0,66 · `scaleway/gpt-oss-120b` 0,17/0,70 · `azure/gpt-5.6-luna@swedencentral` 0,22/1,32 · `vertex/gemini-3.1-flash-lite@eu` 0,275/1,65 |
| Informes con modelo fuerte | el elegido en Ajustes | `bedrock/claude-sonnet-5@eu-central-1` 2,20/11 · `vertex/gemini-3.8-flash@eu` 0,825/4,125 · `azure/gpt-5.6-terra@swedencentral` 2,20/13,20 · `bedrock/claude-opus-5-5@eu-central-1` 4,40/22 |

La región UE cuesta un 10 % más en Bedrock, Vertex (salvo 2.5 Flash-Lite) y Azure. Con el volumen de maydom, céntimos al mes. Ojo: los `openai/…` directos de Requesty llevan `data_retention: true` (la retención de abuso de OpenAI); los `azure/…`, no. Para datos personales, siempre la variante `azure/…@eu`.

## Cómo encaja en el gateway

El cambio vive entero en el gateway (ADR-004 de maydom no cambia: las apps siguen llamando a `{LLM_BASE_URL}/chat/completions` con su clave y su `X-Operacion`). Piezas, sobre el código de `87cbca5`:

- **`config.rs`**: `REQUESTY_API_KEY` y `REQUESTY_BASE` (por defecto `https://router.eu.requesty.ai/v1`: el procesamiento en Fráncfort no cuesta más y deja la puerta abierta a lo UE).
- **`openrouter.rs` → un cliente por upstream**: lo común (`chat()`, errores con su código prefijado `openrouter_*`/`requesty_*`) y lo propio de cada uno (`generacion()` y `HTTP-Referer` solo en OpenRouter).
- **`catalogo.rs`**: descargar también el catálogo de Requesty (público, misma caché de una hora) y construir la **equivalencia** id de OpenRouter → id de Requesty: nombre canónico normalizado (puntos y guiones, sufijos de fecha) y, entre las variantes, la de la UE, sin retención ni entrenamiento, con `json_schema` si la petición lo usa, y la más barata. Una tabla a mano (`EQUIVALENCIAS`) manda sobre lo automático. Un modelo sin equivalente no tiene respaldo, y `/v1/models` lo dice.
- **`rutas/chat.rs`**: primero OpenRouter; si falla la conexión, tarda demasiado, o responde 5xx, 429 o 402, la misma petición va a Requesty con el id equivalente. Cualquier otro 4xx se devuelve tal cual (un `json_schema` que no vale fallaría igual). Nunca se conmuta a mitad de un streaming. Cabecera `X-Upstream` para saber quién respondió.
- **`uso.rs`**: hoy toma `usage.cost` y lo marca `coste_origen = "openrouter"`. Con Requesty, `coste_origen = "requesty"`, sin conciliación (su coste llega en la respuesta) y una columna `upstream`. `estima()` usa los precios del catálogo del upstream que respondió. Presupuestos y cortacircuitos (`guardia.rs`) no cambian: cuentan lo de los dos.
- **Más adelante, «solo UE» por aplicación u operación**: una app (o las operaciones `maydom-finanzas`, `maydom-sueno-relato`…) marcada así va directa a Requesty con un modelo `@eu`, sin pasar por OpenRouter ni conmutar fuera de la UE. Si falla, 502 y maydom sigue con su motor de reglas.

Las políticas de respaldo del propio Requesty (`policy/…`) sirven para encadenar regiones UE dentro de Requesty, pero no para el respaldo principal: dejarían la lógica en un panel externo y no en el repositorio.

## Supuestos y planes B

- [SUPUESTO] Los precios del catálogo no llevan el 5 % y `usage.cost` sí. Plan B: la primera llamada real lo dice (comparar `usage.cost` con tokens × precio).
- [SUPUESTO] `response_format` con `json_schema` funciona igual por Requesty en los modelos de Vertex y Azure de la UE que el catálogo marca con `supports_output_json_schema`. Plan B: la prueba con clave usa los cuerpos reales de `pedirJSON` de maydom.
- [SUPUESTO] El formato de error de Requesty permite traducirlo como el de OpenRouter. Plan B: tratar todo lo no reconocido como `requesty_rechaza` con el cuerpo original.
- [SUPUESTO] Medios de pago, IVA, factura a un particular en España y mínimo de recarga: la página de precios tiene la pregunta pero no la respuesta legible. Plan B: verlo al crear la cuenta.
- [SUPUESTO] `vertex/gemini-2.5-flash-lite@europe-*` desaparece del catálogo con el apagado de Vertex del 20/10/2026. Plan B: el sustituto de la extracción se elige ya entre los de la tabla, probándolo con casos reales de datos inventados.
- Requesty es una empresa pequeña, del tamaño de las que han cerrado este año. Por eso va de segundo y con la equivalencia dentro del gateway: cambiarlo por Vercel o por otro es configuración.

## Próximos pasos

1. **Usuario**: crear la cuenta en Requesty, recargar lo mínimo, crear una clave con el log de prompts **apagado** y, si se quiere ya, una lista de acceso solo con modelos UE; guardar la clave como secreto `REQUESTY_API_KEY` en el repo del gateway (y en el `.env` del VPS por `deploy-vps.yml`). La clave nunca se pega en una sesión.
2. **Ya, sin esperar a Requesty**: añadir `provider: {zdr: true, data_collection: "deny"}` a todas las llamadas a OpenRouter. Es una línea en el gateway.
3. **Sesión en el repo del gateway**: el segundo upstream tal como está arriba, con una comprobación en `deploy.yml` que haga una llamada real a Requesty con texto de prueba, imprima `usage` y fuerce la conmutación apuntando OpenRouter a un host inválido.
4. **maydom**: elegir el sustituto de `gemini-2.5-flash-lite` antes del 20/10/2026 y, cuando el gateway lo permita, marcar como «solo UE» las operaciones con datos personales.
