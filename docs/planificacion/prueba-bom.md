# Prueba con un BOM real desde el PC

Esta prueba usa el servicio como lo usaría una aplicación de presupuestos: le
das un fichero de materiales y te dice qué columna es cada cosa, para poder
valorarlo. De paso comprueba las cuatro etapas entregadas, porque toca el
catálogo, la consulta, la medición y el histórico.

## Qué hace exactamente

1. Abre el BOM (`.xlsx` o `.csv`) y descarta las columnas sin un solo dato.
2. Manda **solo la cabecera y unas pocas filas**. El fichero no se sube a
   ningún sitio: en el BOM de Keyless viajan 7 líneas de 105.
3. Pide al modelo el mapeo de columnas con salida estructurada, así que la
   respuesta es un JSON con forma fija, no texto libre.
4. Enseña el mapeo, lo que falta para poder presupuestar, y lo que costó la
   llamada, primero estimado y luego confirmado por OpenRouter.

## Lo que necesitas

- **PowerShell**, el que ya trae Windows sirve.
- **La clave de servicio**, la misma que usas en la consola web.
- El repositorio en el PC. Si no lo tienes al día: `tools/aterrizar.ps1`.

## Paso 1: mirar sin gastar

Antes de pagar nada, comprueba qué se enviaría:

```powershell
pwsh -File tools\probar-bom.ps1 -Fichero C:\ruta\BOM_Keyless3.3_Rev2.xlsx -SoloMuestra
```

Verás cuántas filas y columnas tienen datos, y las líneas exactas que viajarían.
No se llama al servicio, así que no cuesta nada. Úsalo también para afinar
cuántas filas mandar con `-Filas`.

## Paso 2: la prueba de verdad

```powershell
pwsh -File tools\probar-bom.ps1 -Fichero C:\ruta\BOM_Keyless3.3_Rev2.xlsx
```

Si no hay clave, el script la pide y la tecleas ahí. Así no queda en el
historial de PowerShell, cosa que sí pasa al escribirla con `-Clave`. También
vale la variable `SERVICIO_CLAVE`.

Sin `-Modelo`, se usa el modelo por defecto del servicio, el mismo que entra
cuando una petición no elige ninguno.

## Qué deberías ver

Tres bloques. Primero el mapeo:

```
referencia_fabricante    Referencia principal
descripcion              Description
cantidad                 Quantity
designadores             Designator
proveedor                Proveedor
precio_unitario          (no esta en el fichero)
```

Segundo, lo que falta para valorar el BOM. En este fichero, el precio: hay
referencia y proveedor, pero ningún importe, así que presupuestar exige cruzar
con una tarifa o una API de distribuidor.

Tercero, el coste de la llamada. Aparece dos veces a propósito: la primera es la
estimación con los precios del catálogo, y unos segundos después el importe que
factura OpenRouter. Es el dato que convierte esto en negocio: **cuánto cuesta
analizar un BOM**.

## Paso 3: comparar modelos

```powershell
pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Comparar
```

Coge tres escalones de precio del catálogo, entre los que admiten salida
estructurada: el modelo por defecto del servicio, uno intermedio y el más caro.
Después enseña una tabla con cuántos campos mapeó cada uno, lo que tardó y lo
que costó, y otra con lo que dijo cada modelo en cada campo.

Con `-Modelo a/uno,b/dos` comparas los que quieras.

**Por qué existe esta comparación.** La primera versión de la prueba elegía sola
el primer modelo del catálogo que admitiera salida estructurada, y le tocó uno
diminuto: mapeó **una** columna de siete, dijo que `Designator`, `Description` y
`Quantity` no estaban en el fichero, y todo ello con confianza `0.99`. El JSON
era válido, así que ninguna validación de esquema lo habría pillado. La calidad
del mapeo depende del modelo, y por eso conviene medirla antes de elegir.

## Cuánto cuesta ejecutarla

Con la cabecera y seis filas son del orden de 700 tokens de entrada y 200 de
salida. En una prueba real con un modelo barato salió por 0,000182 dólares. Aun
comparando tres modelos, incluido el más caro del catálogo, la prueba entera
cuesta céntesimas de céntimo.

## Si algo falla

| Qué ves | Qué pasa |
|---|---|
| `sin_clave` o `clave_invalida` | La clave de servicio no llegó o no es la buena. |
| `openrouter_rechaza` con 402 | Sin saldo en OpenRouter. |
| `el modelo no devolvio JSON valido` | El modelo se salió del esquema. Si se repite, toca añadir validación con reintento en el servicio. |
| `coste sin calcular` | El catálogo no tenía precio de ese modelo. La confirmación posterior lo arregla. |
| Mapeo lleno de «no esta en el fichero» | El modelo es demasiado pequeño para la tarea. Compara con `-Comparar` y fija uno mejor con `-Modelo`. |

## Contra el backend local

Si estás probando cambios del servicio en el PC, con `tools/arrancar.ps1`
levantado:

```powershell
pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Api http://localhost:8080
```

## Cómo encaja esto en la aplicación de presupuestos

El reparto que propone esta prueba es el que debería tener la aplicación real:

- **La aplicación** lee el fichero, saca la cabecera y unas filas, y decide.
- **El servicio** solo habla con el modelo, mide y cobra. No procesa ficheros ni
  guarda BOM: no es lo suyo y encarecería cada análisis.

Lo que falta para llevarlo a producción es la etapa 5, una clave propia para esa
aplicación en vez de la única compartida de hoy, y la etapa 6, un presupuesto
que corte si un BOM mal troceado dispara cientos de llamadas.
