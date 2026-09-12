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
#      pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Comparar
#      pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Modelo a/uno,b/dos
#      pwsh -File tools\probar-bom.ps1 -Fichero BOM.xlsx -Api http://localhost:8080
param(
  [Parameter(Mandatory = $true)][string]$Fichero,
  [string]$Clave = $env:SERVICIO_CLAVE,
  [string]$Api = 'https://openrouter-npiobject-labs.fly.dev',
  [string[]]$Modelo = @(),
  [int]$Filas = 6,
  [switch]$Comparar,
  [switch]$SoloMuestra
)
$ErrorActionPreference = 'Stop'
$Api = $Api.TrimEnd('/')
# Invocado con -File, "uno,dos" llega como un solo texto en vez de como lista.
$Modelo = @($Modelo | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

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
if (-not $Clave) {
  Aviso "no hay clave en -Clave ni en SERVICIO_CLAVE."
  $Clave = Read-Host "clave de servicio"
  # Tecleada aqui no queda en el historial de PowerShell, a diferencia de -Clave.
}
if (-not $Clave) { Malo "sin clave no se puede llamar al servicio"; exit 1 }

# --- 3. preguntar al servicio -----------------------------------------------

# Una peticion HTTP con la clave, devolviendo cuerpo y cabeceras.
function Llamar($ruta, $metodo = 'GET', $cuerpo = $null, [switch]$Tolerante) {
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
    if ($Tolerante) { return $null }
    exit 1
  }
  $texto = [Text.Encoding]::UTF8.GetString($r.RawContentStream.ToArray())
  return @{ datos = ($texto | ConvertFrom-Json); cabeceras = $r.Headers }
}

# Sin modelo elegido, el que el servicio usa por defecto. Rebuscar en el
# catalogo salia barato y malo: el primer "flash" de la lista puede ser un
# modelo diminuto que rellena nulos con toda la confianza del mundo.
$porDefecto = (Llamar '/v1/estado').datos.modelo_defecto

if (-not $Modelo -and -not $Comparar) {
  $Modelo = @($porDefecto)
  Aviso "sin -Modelo: se usa el del servicio, $porDefecto"
}

# -Comparar elige del catalogo tres escalones de precio en vez de fiarse de
# nombres escritos a mano, que cambian cada pocos meses. Con una cabecera y
# unas filas, hasta el modelo caro cuesta centesimas de centimo.
if ($Comparar -and -not $Modelo) {
  $catalogo = @((Llamar '/v1/models').datos.data |
    Where-Object { $_.json -and -not $_.gratis } |
    Sort-Object entrada)
  if ($catalogo.Count -lt 3) { Malo "el catalogo no trae modelos suficientes con salida estructurada"; exit 1 }

  $medio = $catalogo[[math]::Floor($catalogo.Count / 2)]
  $caro  = $catalogo[-1]
  $Modelo = @($porDefecto, $medio.id, $caro.id) | Select-Object -Unique
  Aviso "comparando: el del servicio, uno intermedio ($($medio.entrada) `$/M) y el mas caro ($($caro.entrada) `$/M)"
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

# Los nombres exactos de las columnas, delante: sin esto un modelo pequeño
# responde null a campos que estan a la vista.
$columnas = ($utiles | ForEach-Object { Recorta $cabecera[$_] }) -join ' | '

$sistema = @'
Eres un analista de listas de materiales de electronica. Recibes la cabecera y
unas filas de un BOM y dices que columna corresponde a cada campo necesario
para presupuestarlo.

Reglas:
- Responde con el nombre EXACTO de una columna de la lista de columnas
  disponibles, copiado tal cual, con sus espacios.
- Usa null solo si esa informacion no esta en ninguna columna. Antes de poner
  null, comprueba la lista entera: casi siempre hay una columna equivalente
  aunque no se llame igual.
- No inventes columnas que no esten en la lista.
'@

function Analiza($modelo) {
  Aviso "preguntando a $modelo..."
  $peticion = @{
    model = $modelo
    response_format = $esquema
    messages = @(
      @{ role = 'system'; content = $sistema }
      @{ role = 'user'; content = "Columnas disponibles:`n$columnas`n`nCabecera y primeras filas:`n`n$muestra" }
    )
  }

  $respuesta = Llamar '/v1/chat/completions' 'POST' $peticion -Tolerante
  if (-not $respuesta) { return $null }

  $idUso = $respuesta.cabeceras['X-Uso-Id']
  if ($idUso -is [array]) { $idUso = $idUso[0] }
  $contenido = $respuesta.datos.choices[0].message.content
  try {
    $mapeo = $contenido | ConvertFrom-Json
  } catch {
    Malo "$modelo no devolvio JSON valido. Esto es lo que dijo:"
    Write-Host $contenido
    return $null
  }
  return @{ modelo = $modelo; mapeo = $mapeo; uso = $idUso }
}

function Dinero($valor) { if ($valor -eq $null) { 'sin calcular' } else { '{0:N6} $' -f $valor } }

# --- 4. preguntar a cada modelo y ensenar lo que dice ------------------------

$resultados = @()
foreach ($unModelo in $Modelo) {
  $r = Analiza $unModelo
  if (-not $r) { continue }

  Write-Host ""
  Bien "$($r.modelo) · confianza $($r.mapeo.confianza)"
  $r.mapeo.columnas.PSObject.Properties | ForEach-Object {
    $valor = if ($_.Value) { $_.Value } else { '(no esta en el fichero)' }
    "  {0,-24} {1}" -f $_.Name, $valor
  }
  if ($r.mapeo.faltan_para_presupuestar) {
    "  falta: " + ($r.mapeo.faltan_para_presupuestar -join ', ')
  }
  if ($r.mapeo.notas) { "  nota: $($r.mapeo.notas)" }

  # El coste tarda unos segundos en reconciliarse con lo que factura OpenRouter.
  if ($r.uso) {
    $uso = (Llamar "/v1/uso/$($r.uso)" -Tolerante).datos
    if ($uso -and $uso.coste_origen -ne 'openrouter') {
      Start-Sleep -Seconds 5
      $uso = (Llamar "/v1/uso/$($r.uso)" -Tolerante).datos
    }
    $r.registro = $uso
  }
  $resultados += $r
}

if (-not $resultados) { Malo "ningun modelo devolvio un mapeo utilizable"; exit 1 }

# --- 5. comparar ------------------------------------------------------------

Write-Host ""
Aviso "resumen"
$tabla = foreach ($r in $resultados) {
  $puestos = @($r.mapeo.columnas.PSObject.Properties | Where-Object { $_.Value }).Count
  $total   = @($r.mapeo.columnas.PSObject.Properties).Count
  $tokens = '?'; $tardo = '?'; $coste = '?'
  if ($r.registro) {
    $tokens = "$($r.registro.tokens_entrada)+$($r.registro.tokens_salida)"
    $tardo  = "{0} s" -f [math]::Round($r.registro.latencia_ms / 1000, 1)
    $coste  = Dinero $r.registro.coste
  }
  [pscustomobject]@{
    Modelo    = $r.modelo
    Campos    = "$puestos/$total"
    Confianza = $r.mapeo.confianza
    Tokens    = $tokens
    Tardo     = $tardo
    Coste     = $coste
  }
}
$tabla | Format-Table -AutoSize | Out-String -Width 200 | Write-Host

if ($resultados.Count -gt 1) {
  Aviso "que dijo cada uno"
  $campos = @($resultados[0].mapeo.columnas.PSObject.Properties.Name)
  $campos | ForEach-Object {
    $campo = $_
    $fila = [ordered]@{ Campo = $campo }
    foreach ($r in $resultados) {
      # El nombre corto del modelo basta para distinguirlos en la tabla.
      $corto = ($r.modelo -split '/')[-1]
      $valor = $r.mapeo.columnas.$campo
      $fila[$corto] = if ($valor) { $valor } else { '-' }
    }
    [pscustomobject]$fila
  } | Format-Table -AutoSize -Wrap | Out-String -Width 200 | Write-Host
}

$hoy = (Get-Date).ToString('yyyy-MM-dd')
$resumen = (Llamar "/v1/uso/resumen?desde=$hoy&agrupar=modelo" -Tolerante).datos
if ($resumen) {
  Aviso ("hoy llevas {0} llamadas y {1} en total" -f $resumen.totales.llamadas, (Dinero $resumen.totales.coste))
}
