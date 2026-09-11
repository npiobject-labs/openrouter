# Ideas de entrada — openrouter

Documento de recogida en bruto. Cada bloque es un prompt del usuario, transcrito
sin interpretar. La fase de planificación llega al final, cuando el usuario lo pida.

## Entrada 01 — 2026-09-11 — Idea general

- La aplicación es una **plantilla para la gestión de OpenRouter y sus modelos**.
- La plantilla está pensada para **reutilizarse en otras apps** (no es un producto final en sí).

Funcionalidades pedidas:

1. **Seleccionar modelos**: poder elegir un modelo de OpenRouter.
2. **Lanzar la consulta**: una vez seleccionado, la aplicación destino lanza una
   consulta contra ese modelo.
3. **Devolver la información de uso al terminar la consulta** (funcionalidad nueva):
   - tokens gastados,
   - coste de esos tokens,
   - y toda la información adicional que OpenRouter pueda proporcionar.

Resumen en sus términos: «plantilla para la gestión de la utilización de los modelos
LLM a través de OpenRouter, y luego devolución de la información de dicha utilización».

Nota: el usuario indica que llegará más información en prompts siguientes.

## Entrada 02 — 2026-09-11 — Propuesta de funcionalidades (Claude, a petición del usuario)

Petición: análisis crítico, de usabilidad y creativo sobre qué más debería hacer la app.
Son propuestas, no decisiones. Se filtrarán en la fase de planificación.

### Críticas al planteamiento actual

- **«Plantilla» y «app» son dos entregables distintos.** Si el valor es reutilizar en
  otras apps, lo que hay que congelar es el contrato (endpoints + cliente mínimo), y la
  UI debería ser solo un cliente de referencia. Riesgo de acabar con una demo bonita que
  no se pueda incrustar en ningún sitio.
- **La clave de OpenRouter nunca puede tocar el navegador.** `docs/` es público y va a
  Pages: toda llamada pasa por el backend como proxy. Esto condiciona la arquitectura
  entera desde el minuto uno.
- **El coste que llega en `usage` no siempre es el definitivo.** OpenRouter expone
  `/api/v1/generation?id=` con el coste real y los tokens nativos del proveedor.
  [SUPUESTO] hay un retardo de cientos de milisegundos hasta que ese dato está
  disponible. Plan B: registrar el `usage` de la respuesta y reconciliar después en
  segundo plano.
- **El catálogo de modelos cambia casi a diario.** Hace falta caché con TTL sobre
  `/api/v1/models` y un snapshot en disco como respaldo si la API no responde.
- **Fly es efímero.** Sin volumen no hay histórico de consumo. [SUPUESTO] se puede
  montar un volumen pequeño para SQLite; plan B: registro en memoria con exportación, o
  base externa.

### Funcionalidades que añadiría

Gestión de modelos:

- **Catálogo filtrable**: modalidad, ventana de contexto, precio de entrada y salida,
  soporte de herramientas y de salida estructurada, proveedor, gratuitos.
- **Comparador** de dos o tres modelos en paralelo, con precio y capacidades enfrentados.
- **Alias semánticos por rol** («redactor», «clasificador», «visión»), que la plantilla
  resuelve a un modelo concreto. La app destino no vuelve a escribir nunca un nombre de
  modelo en su código. Es, probablemente, la pieza de más valor para la reutilización.
- **Cadena de respaldo**: lista ordenada de modelos y preferencias de proveedor, que
  OpenRouter ya soporta de forma nativa.
- **Aviso de cambios en el catálogo**: modelo retirado o subida de precio.

Ejecución:

- **Streaming** con cancelación, y contador de tokens en vivo.
- **Disparo multi-modelo** del mismo prompt, para comparar calidad, latencia y coste.
- **Plantillas de prompt** con variables y versionado.
- **Parámetros completos**: temperatura, tope de salida, esfuerzo de razonamiento,
  esquema JSON de salida, herramientas.
- **Caché por huella** de prompt + modelo + parámetros. Ahorro directo de dinero.
- **Reintentos con backoff** y gestión de límites de tasa.

Medición y coste (el núcleo de lo pedido):

- **Por llamada**: tokens de entrada, salida, razonamiento y caché; coste; latencia
  total; tiempo hasta el primer token; tokens por segundo; proveedor real que sirvió;
  motivo de finalización; si hubo respaldo.
- **Agregados** por día, modelo y etiqueta de app, para que varias apps compartan la
  plantilla y cada una vea su propio gasto.
- **Presupuestos y alertas** con corte duro opcional, consultables por la app destino
  como interruptor.
- **Estimación previa al envío**: coste aproximado antes de pulsar enviar.
- **Exportación** CSV/JSON y endpoint de consumo.
- **Reconciliación** contra `/api/v1/credits` y `/api/v1/generation`.

Usabilidad:

- **Una sola pantalla** para lo esencial: modelo, prompt, enviar, respuesta con ficha de
  coste. Todo lo demás, detrás.
- **Ficha de coste siempre visible** tras cada respuesta.
- **Historial** con búsqueda y botón de repetir la misma llamada con otro modelo.
- **Crédito restante** en la cabecera.
- **Modo demo sin clave**, con datos simulados, porque el sitio es público.
- **Uso desde móvil** de primera clase, coherente con el flujo del proyecto.

Ideas diferenciales:

- **Semáforo de coste** antes de enviar si el prompt supera un umbral.
- **Sugerencia de modelo más barato** cuando la tarea parece trivial.
- **Modo torneo**: mismo prompt, varios modelos, evaluación a ciegas, ranking propio por
  tipo de tarea.
- **Cliente mínimo** en JavaScript y crate de Rust: es lo que convierte esto en plantilla
  de verdad.
- **Modo espejo local**: la plantilla en localhost sirve a la app en desarrollo y
  registra igual que en producción.
- **Webhook de finalización** para apps destino asíncronas.

### Lo que no haría

- **Un chat completo con hilos y carpetas.** Existen decenas; el valor aquí está en la
  medición y en la reutilización, no en la conversación.
- **Guardar el contenido de los prompts por defecto.** Solo metadatos; el contenido,
  opcional y nunca en el repositorio público.
