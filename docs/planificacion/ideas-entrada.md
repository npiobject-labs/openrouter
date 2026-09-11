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

## Entrada 03 — 2026-09-11 — Decisión de naturaleza del producto

El usuario acepta las sugerencias de la entrada 02 como material a tener en cuenta en el
desarrollo, y fija el rumbo: **esto no es una app, es un conjunto de servicios pensados
para ser llamados por otras apps**. Confirmado por ambas partes.

Consecuencias que arrastra esa decisión (a resolver en planificación):

- **El contrato manda.** Lo primero que se congela es la forma de los endpoints y su
  versionado (`/v1/...`). El código de dentro puede cambiar; la superficie, no.
- **La interfaz web deja de ser el producto** y pasa a ser consola de operación y cliente
  de referencia: sirve para probar, ver el catálogo y leer el consumo, no para «usar la
  app».
- **Autenticación entre app y servicio.** Cada app consumidora necesita su propia
  credencial, distinta de la clave de OpenRouter, que nunca sale del servicio.
- **Identidad de app en cada llamada.** Sin una etiqueta por app no hay consumo por app,
  ni presupuestos, ni facturación interna.
- **El presupuesto es un servicio más**, consultable: la app destino puede preguntar si
  le queda margen antes de gastar.
- **El cliente mínimo** (JavaScript y crate de Rust) sube de prioridad: es la forma real
  en que otras apps consumirán esto.
- **Estabilidad por encima de funcionalidad.** Si una app depende del servicio, un cambio
  incompatible rompe a terceros; toca disciplina de versiones desde el principio.

## Entrada 04 — 2026-09-11 — Segundo análisis (a petición del usuario, con otro modelo)

Petición: repetir el análisis buscando funcionalidades que no estén ya en la entrada 02.
Solo se listan las nuevas o las que cambian de forma sustancial. Siguen siendo propuestas.

### La sugerencia grande: exponer una API compatible con OpenAI

- El servicio publica `/v1/chat/completions` (y `/v1/models`) con el mismo contrato que
  la API de OpenAI, que es también el que usa OpenRouter.
- Consecuencia: **no hace falta escribir ningún cliente**. Cualquier SDK existente
  (Python, JS, Rust, Go…) funciona cambiando solo la URL base y la clave. La adopción en
  una app nueva es una línea.
- Lo propio del servicio (alias, etiqueta de app, presupuesto, política) va en cabeceras
  `X-...` o en campos extra del cuerpo, que los SDK dejan pasar.
- El «cliente mínimo» de la entrada 02 queda reducido a un envoltorio opcional de
  comodidad, no a una pieza necesaria.
- Publicar además la especificación OpenAPI del contrato completo, para generar clientes
  de los endpoints propios (consumo, presupuesto, catálogo).

### Dinero: hacer que OpenRouter aplique el presupuesto por nosotros

- OpenRouter permite **crear claves hijas por programa con límite de crédito**
  (provisioning keys). [SUPUESTO] disponible en la cuenta del usuario; plan B: aplicar
  el presupuesto en el propio servicio como ya estaba previsto.
- Una clave hija por app consumidora: el tope lo aplica OpenRouter aunque el servicio
  tenga un fallo, y el consumo por app sale directamente de su panel.
- Modo **BYOK**: una app puede traer su propia clave de OpenRouter y el servicio solo
  mide y enruta. Dos modos de operación: clave central o clave por app.
- **Simulador «qué pasaría si»**: con el histórico de uso propio, calcular qué habría
  costado el mes pasado si el alias «redactor» apuntara a otro modelo. Convierte el
  cambio de modelo en una decisión con número delante.
- **Histórico de precios del catálogo**: snapshot diario para ver la evolución y
  detectar subidas antes de que duelan.
- **Cortacircuitos por bucle**: la misma app repitiendo el mismo prompt muchas veces en
  poco tiempo casi siempre es un bug; cortar y avisar antes de que se vacíe el crédito.
  Complementa al presupuesto: el presupuesto protege del gasto legítimo excesivo, esto
  protege del gasto accidental.
- **Detección de anomalías** de gasto por app frente a su media.

### Trazabilidad: coste por operación de negocio, no solo por llamada

- **Identificador de operación** que la app propaga: una operación de negocio (procesar
  un documento, atender a un cliente) puede implicar varias llamadas. Sin esto solo se
  sabe cuánto cuesta una llamada, no cuánto cuesta «un documento».
- **Identificador de usuario final** de la app destino, para saber qué clientes cuestan
  más. OpenRouter ya acepta un campo `user` que se puede aprovechar.
- **Clave de idempotencia** por llamada: si la app reintenta por un corte de red, no
  paga dos veces. Distinto de la caché por huella: la caché es por contenido y opcional;
  la idempotencia es por clave del cliente y siempre activa.
- **Modo simulación (dry-run)**: la llamada devuelve modelo resuelto, política aplicada
  y coste estimado sin ejecutar nada. Sirve para probar integraciones sin gastar.
- **Metadatos libres** por llamada y búsqueda facetada sobre ellos.

### Alias con despliegue progresivo

- **Canario por alias**: enrutar un porcentaje del tráfico del alias al modelo
  candidato y comparar coste, latencia y errores antes de cambiar del todo.
- **Conjunto de pruebas dorado por alias**: prompts con respuesta esperada; al cambiar
  el modelo detrás de un alias se ejecuta el conjunto y se comparan resultados. Hace
  seguro lo que la entrada 02 proponía (alias semánticos).
- **Estrategia por alias**: más barato, más rápido o equilibrado. OpenRouter permite
  ordenar proveedores por precio o latencia para un mismo modelo; el alias fija la
  estrategia y el servicio la aplica.
- **Perfil = alias + parámetros por defecto + prompt de sistema.** Unifica alias y
  plantillas de prompt en una sola cosa registrable.

### Robustez del servicio

- **Salida estructurada garantizada**: la app registra un esquema JSON; el servicio
  valida la respuesta y reintenta o repara automáticamente si no cumple. La app recibe
  siempre algo tipado o un error claro, nunca texto a medio parsear.
- **Trabajos diferidos**: la app manda un lote, recibe un identificador, consulta o
  recibe webhook. Concurrencia controlada y prioridades. Para procesar cientos de
  documentos sin que la app gestione la cola.
- **Cuota de peticiones por app** (por minuto), independiente del presupuesto en dinero.
- **Estado de proveedores**: OpenRouter publica disponibilidad por proveedor; el
  servicio puede consultarla y evitar enrutar a uno caído.
- **Último respaldo fuera de OpenRouter**: la cadena de respaldo puede terminar en un
  endpoint local (Ollama u otro) para que la app no se quede muda si OpenRouter cae.
- **Versión exacta de modelo y proveedor** registrada por llamada, para reproducir.

### Datos y cumplimiento

- **Política de proveedores por app**: solo proveedores sin retención de datos (ZDR),
  solo región concreta, o excluir proveedores concretos. OpenRouter lo soporta por
  petición; el servicio lo fija por app o por alias.
- **Retención configurable y purga** por app: días que se guarda cada cosa, y un
  endpoint de borrado.
- **Registro de qué proveedor procesó cada llamada**, imprescindible si un cliente
  pregunta dónde acabaron sus datos.

### Crítica a la entrada 02 con ojos nuevos

- **Sobra ambición para una primera versión hecha por una persona.** Modo torneo,
  evaluación a ciegas, plantillas de prompt versionadas y comparador visual son buenos,
  pero son capas superiores. El núcleo defendible es: proxy compatible con OpenAI +
  medición por llamada + alias + presupuesto por app. Todo lo demás se cuelga de ahí.
- **La caché por huella es peligrosa por defecto.** Dos apps distintas con el mismo
  prompt no deberían compartir respuesta salvo que lo pidan; y con temperatura alta la
  caché cambia el comportamiento esperado. Debe ser opt-in por app y por llamada.
- **«Estimación previa» necesita un tokenizador local.** [SUPUESTO] OpenRouter no
  ofrece un endpoint de conteo de tokens; plan B: aproximar con un tokenizador tipo
  tiktoken y marcar el resultado como estimado.
- **Medir en streaming no es gratis.** Para devolver coste al terminar hay que
  interceptar el flujo completo, contar y esperar el bloque final de `usage`. Hay que
  diseñarlo desde el principio, no añadirlo después.
