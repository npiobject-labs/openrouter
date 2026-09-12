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
$env:SERVICIO_CLAVE = "<tu clave>"
pwsh -File tools\probar-bom.ps1 -Fichero C:\ruta\BOM_Keyless3.3_Rev2.xlsx
```

La clave también se puede pasar con `-Clave`, pero por la variable de entorno no
queda en el historial de la consola.

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

## Cuánto cuesta ejecutarla

Con la cabecera y seis filas son del orden de 1.500 tokens de entrada. Con un
modelo barato sale por bastante menos de un céntimo. El script elige solo un
modelo del catálogo que admita salida estructurada y no sea gratuito, porque los
gratuitos están desactivados en esta cuenta. Puedes fijar otro con `-Modelo`.

## Si algo falla

| Qué ves | Qué pasa |
|---|---|
| `sin_clave` o `clave_invalida` | La clave de servicio no llegó o no es la buena. |
| `openrouter_rechaza` con 402 | Sin saldo en OpenRouter. |
| `el modelo no devolvio JSON valido` | El modelo se salió del esquema. Si se repite, toca añadir validación con reintento en el servicio. |
| `coste sin calcular` | El catálogo no tenía precio de ese modelo. La confirmación posterior lo arregla. |

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
