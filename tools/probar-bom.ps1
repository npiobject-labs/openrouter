# Prueba de punta a punta del servicio openrouter con un BOM real.
#
# Lee un fichero de materiales (.xlsx o .csv), manda solo la cabecera y unas
# pocas filas de muestra al servicio, y le pide que diga que columna es cada
# cosa para poder presupuestar. Luego consulta lo que costo esa llamada.
#
# No sube el fichero a ningun sitio: viajan la cabecera y N filas, nada mas.
#
# Uso: pwsh -File tools\probar-bom.ps1 -Fichero C:\ruta\BOM.xlsx -Clave <clave>
#      pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -SoloMuestra
#      pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Api http://localhost:8080
param(
  [Parameter(Mandatory = $true)][string]$Fichero,
  [string]$Clave = $env:SERVICIO_CLAVE,
  [string]$Api = 'https://openrouter-npiobject-labs.fly.dev',
  [string]$Modelo = '',
  [int]$Filas = 6,
  [switch]$SoloMuestra
)
$ErrorActionPreference = 'Stop'
$Api = $Api.TrimEnd('/')

function Aviso($texto)  { Write-Host "probar-bom : $texto" }
function Malo($texto)   { Write-Host "probar-bom : ERROR - $texto" -ForegroundColor Red }
function Bien($texto)   { Write-Host "probar-bom : $texto" -ForegroundColor Green }

# --- 1. leer el fichero -----------------------------------------------------

# Un .xlsx es un zip con XML dentro, asi que se lee sin Excel ni modulos.
function Leer-Xlsx($ruta) {
  Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
  $zip = [IO.Compression.ZipFile]::OpenRead($ruta)
  try {
    function Texto-De($entrada) {
      $lector = New-Object IO.StreamReader($entrada.Open(), [Text.Encoding]::UTF8)
      try { $lector.ReadToEnd() } finally { $lector.Dispose() }
    }

    # Las cadenas se guardan una sola vez en una tabla compartida.
    $compartidas = @()
    $tabla = $zip.Entries | Where-Object { $_.FullName -eq 'xl/sharedStrings.xml' }
    if ($tabla) {
      $xml = [xml](Texto-De $tabla)
      foreach ($si in $xml.sst.si) {
        if ($si.t -is [string]) { $compartidas += $si.t }
        elseif ($si.t.'#text')  { $compartidas += $si.t.'#text' }
        elseif ($si.r)          { $compartidas += (($si.r | ForEach-Object { $_.t.'#text'; $_.t }) -join '') }
        else                    { $compartidas += '' }
      }
    }

    $hoja = $zip.Entries | Where-Object { $_.FullName -like 'xl/worksheets/sheet*.xml' } | Select-Object -First 1
    if (-not $hoja) { throw "el fichero no tiene ninguna hoja legible" }
    $xml = [xml](Texto-De $hoja)

    $tabla = @()
    foreach ($fila in $xml.worksheet.sheetData.row) {
      $celdas = @{}
      foreach ($c in $fila.c) {
        # La referencia es tipo "B12": las letras dan la columna.
        $letras = ($c.r -replace '\d', '')
        $columna = 0
        foreach ($letra in $letras.ToCharArray()) {
          $columna = $columna * 26 + ([int][char]::ToUpper($letra) - 64)
        }
        $valor = switch ($c.t) {
          's'          { if ($c.v -ne $null) { $compartidas[[int]$c.v] } else { '' } }
          'inlineStr'  { $c.is.t }
          default      { if ($c.v -is [string]) { $c.v } else { $c.v.'#text' } }
        }
        $celdas[$columna] = "$valor"
      }
      if ($celdas.Count -gt 0) {
        $ancho = ($celdas.Keys | Measure-Object -Maximum).Maximum
        $plana = @(1..$ancho | ForEach-Object { if ($celdas.ContainsKey($_)) { $celdas[$_] } else { '' } })
        $tabla += ,$plana
      } else {
        $tabla += ,@()
      }
    }
    return $tabla
  } finally { $zip.Dispose() }
}

function Leer-Csv($ruta) {
  $tabla = @()
  foreach ($linea in (Get-Content -LiteralPath $ruta -Encoding UTF8)) {
    # Separador por coma o punto y coma, el que mas aparezca en la cabecera.
    if (-not $separador) {
      $separador = if (($linea -split ';').Count -gt ($linea -split ',').Count) { ';' } else { ',' }
    }
    $tabla += ,($linea -split $separador | ForEach-Object { $_.Trim('"') })
  }
  return $tabla
}

if (-not (Test-Path -LiteralPath $Fichero)) { Malo "no encuentro $Fichero"; exit 1 }
$extension = [IO.Path]::GetExtension($Fichero).ToLower()

Aviso "leyendo $([IO.Path]::GetFileName($Fichero))"
$tabla = switch ($extension) {
  '.xlsx' { Leer-Xlsx (Resolve-Path -LiteralPath $Fichero) }
  '.csv'  { Leer-Csv  (Resolve-Path -LiteralPath $Fichero) }
  default { Malo "solo se leen .xlsx y .csv, no $extension"; exit 1 }
}

# --- 2. quedarse con lo util ------------------------------------------------

if ($tabla.Count -lt 2) { Malo "el fichero no tiene ni cabecera ni datos"; exit 1 }
$cabecera = $tabla[0]
$conDatos = @($tabla | Select-Object -Skip 1 | Where-Object { ($_ -join '').Trim() -ne '' })

# Las columnas sin un solo dato son ruido de Excel: no se mandan.
$utiles = @()
for ($i = 0; $i -lt $cabecera.Count; $i++) {
  $tieneNombre = "$($cabecera[$i])".Trim() -ne ''
  $tieneDatos = $false
  foreach ($fila in $conDatos) { if ($i -lt $fila.Count -and "$($fila[$i])".Trim() -ne '') { $tieneDatos = $true; break } }
  if ($tieneNombre -or $tieneDatos) { $utiles += $i }
}

Aviso "$($conDatos.Count) filas con datos, $($utiles.Count) columnas con contenido de $($cabecera.Count)"

function Recorta($valor) {
  $texto = "$valor" -replace '\s+', ' '
  if ($texto.Length -gt 60) { $texto.Substring(0, 57) + '...' } else { $texto }
}

$lineas = @()
$lineas += (($utiles | ForEach-Object { Recorta $cabecera[$_] }) -join ' | ')
foreach ($fila in ($conDatos | Select-Object -First $Filas)) {
  $lineas += (($utiles | ForEach-Object { if ($_ -lt $fila.Count) { Recorta $fila[$_] } else { '' } }) -join ' | ')
}
$muestra = $lineas -join "`n"

Write-Host ""
Write-Host "--- lo que se envia (cabecera y $Filas filas) ---" -ForegroundColor Cyan
Write-Host $muestra
Write-Host ""

if ($SoloMuestra) { Aviso "-SoloMuestra: no se llama al servicio y no se gasta credito."; exit 0 }
if (-not $Clave) { Malo "falta la clave de servicio: usa -Clave o la variable SERVICIO_CLAVE"; exit 1 }

# --- 3. preguntar al servicio -----------------------------------------------

# Una peticion HTTP con la clave, devolviendo cuerpo y cabeceras.
function Llamar($ruta, $metodo = 'GET', $cuerpo = $null) {
  $parametros = @{
    Uri     = "$Api$ruta"
    Method  = $metodo
    Headers = @{ Authorization = "Bearer $Clave" }
    UseBasicParsing = $true
  }
  if ($cuerpo) {
    $parametros.ContentType = 'application/json'
    # A bytes para que los acentos del BOM no dependan de la consola.
    $parametros.Body = [Text.Encoding]::UTF8.GetBytes(($cuerpo | ConvertTo-Json -Depth 12 -Compress))
  }
  try {
    $r = Invoke-WebRequest @parametros
  } catch {
    $respuesta = $_.Exception.Response
    if ($respuesta) {
      $lector = New-Object IO.StreamReader($respuesta.GetResponseStream(), [Text.Encoding]::UTF8)
      $texto = $lector.ReadToEnd()
      $lector.Dispose()
      try {
        $fallo = ($texto | ConvertFrom-Json).error
        Malo "$($fallo.code): $($fallo.message)"
      } catch { Malo "respuesta ilegible: $texto" }
    } else { Malo $_.Exception.Message }
    exit 1
  }
  $texto = [Text.Encoding]::UTF8.GetString($r.RawContentStream.ToArray())
  return @{ datos = ($texto | ConvertFrom-Json); cabeceras = $r.Headers }
}

# Sin modelo elegido, el primero del catalogo que admita salida estructurada y
# no sea gratuito: los gratuitos de esta cuenta responden 404.
if (-not $Modelo) {
  Aviso "eligiendo modelo del catalogo..."
  $catalogo = (Llamar '/v1/models?texto=flash').datos.data
  $candidato = $catalogo | Where-Object { $_.json -and -not $_.gratis } | Select-Object -First 1
  if (-not $candidato) { Malo "ningun modelo del catalogo admite salida estructurada"; exit 1 }
  $Modelo = $candidato.id
  Aviso "modelo: $Modelo ($($candidato.entrada) entrada / $($candidato.salida) salida, dolares por millon de tokens)"
}

$esquema = @{
  type = 'json_schema'
  json_schema = @{
    name = 'mapeo_bom'
    strict = $true
    schema = @{
      type = 'object'
      additionalProperties = $false
      required = @('columnas', 'faltan_para_presupuestar', 'confianza', 'notas')
      properties = @{
        columnas = @{
          type = 'object'
          additionalProperties = $false
          description = 'Nombre exacto de la columna del fichero, o null si no existe.'
          required = @('referencia_fabricante', 'descripcion', 'cantidad', 'designadores', 'proveedor', 'precio_unitario', 'alternativo')
          properties = @{
            referencia_fabricante = @{ type = @('string', 'null') }
            descripcion           = @{ type = @('string', 'null') }
            cantidad              = @{ type = @('string', 'null') }
            designadores          = @{ type = @('string', 'null'); description = 'Posiciones en la placa, como C1, C2, R5.' }
            proveedor             = @{ type = @('string', 'null') }
            precio_unitario       = @{ type = @('string', 'null') }
            alternativo           = @{ type = @('string', 'null') }
          }
        }
        faltan_para_presupuestar = @{
          type = 'array'
          items = @{ type = 'string' }
          description = 'Datos imprescindibles para valorar el BOM que este fichero no trae.'
        }
        confianza = @{ type = 'number'; description = 'De 0 a 1.' }
        notas     = @{ type = 'string';  description = 'Una frase sobre lo dudoso del mapeo.' }
      }
    }
  }
}

$peticion = @{
  model = $Modelo
  response_format = $esquema
  messages = @(
    @{ role = 'system'; content = 'Eres un analista de listas de materiales de electronica. Recibes la cabecera y unas filas de un BOM y dices que columna corresponde a cada campo necesario para presupuestarlo. Responde solo con el nombre exacto de la columna tal como aparece en la cabecera, o null si esa informacion no esta en el fichero. No inventes columnas.' }
    @{ role = 'user'; content = "Cabecera y primeras filas de un BOM:`n`n$muestra" }
  )
}

Aviso "preguntando al servicio..."
$reloj = [Diagnostics.Stopwatch]::StartNew()
$respuesta = Llamar '/v1/chat/completions' 'POST' $peticion
$reloj.Stop()

$idUso = $respuesta.cabeceras['X-Uso-Id']
if ($idUso -is [array]) { $idUso = $idUso[0] }
$contenido = $respuesta.datos.choices[0].message.content
try {
  $mapeo = $contenido | ConvertFrom-Json
} catch {
  Malo "el modelo no devolvio JSON valido. Esto es lo que dijo:"
  Write-Host $contenido
  exit 1
}

# --- 4. ensenar el resultado ------------------------------------------------

Write-Host ""
Bien "mapeo propuesto (confianza $($mapeo.confianza))"
$mapeo.columnas.PSObject.Properties | ForEach-Object {
  $valor = if ($_.Value) { $_.Value } else { '(no esta en el fichero)' }
  "{0,-24} {1}" -f $_.Name, $valor
}

if ($mapeo.faltan_para_presupuestar) {
  Write-Host ""
  Aviso "falta para poder valorar el BOM:"
  $mapeo.faltan_para_presupuestar | ForEach-Object { "  - $_" }
}
if ($mapeo.notas) { Write-Host ""; Aviso "nota del modelo: $($mapeo.notas)" }

# --- 5. lo que costo --------------------------------------------------------

Write-Host ""
if (-not $idUso) {
  Aviso "el servicio no devolvio X-Uso-Id: backend anterior a la etapa 3."
  exit 0
}

function Coste($id) { (Llamar "/v1/uso/$id").datos }

function Dinero($valor) { if ($valor -eq $null) { 'sin calcular' } else { '{0:N6} $' -f $valor } }

$uso = Coste $idUso
Aviso ("registro {0}: {1} + {2} tokens, {3} ms, coste {4} ({5})" -f `
  $uso.id, $uso.tokens_entrada, $uso.tokens_salida, $uso.latencia_ms, (Dinero $uso.coste), $uso.coste_origen)

if ($uso.coste_origen -ne 'openrouter') {
  Aviso "esperando a que OpenRouter confirme el coste real..."
  Start-Sleep -Seconds 5
  $uso = Coste $idUso
  if ($uso.coste_origen -eq 'openrouter') {
    Bien ("coste confirmado por OpenRouter: {0} (proveedor {1})" -f (Dinero $uso.coste), $uso.proveedor)
  } else {
    Aviso "sigue con la estimacion del catalogo; vuelve a mirar /v1/uso/$idUso en un minuto."
  }
}

$hoy = (Get-Date).ToString('yyyy-MM-dd')
$resumen = (Llamar "/v1/uso/resumen?desde=$hoy&agrupar=modelo").datos
Aviso ("hoy llevas {0} llamadas y {1} en total" -f $resumen.totales.llamadas, (Dinero $resumen.totales.coste))
