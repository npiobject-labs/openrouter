# Gestiona las aplicaciones que pueden llamar al servicio.
#
# Cada aplicacion tiene su propia clave: asi se sabe quien gasta y se puede
# revocar una sin tocar el despliegue ni a las demas. Estas rutas exigen la
# clave de administracion, que es la del despliegue (SERVICIO_CLAVE).
#
# Uso: pwsh -File tools\apps.ps1                      (lista)
#      pwsh -File tools\apps.ps1 -Crear presupuestos
#      pwsh -File tools\apps.ps1 -Baja app_27b1d11e45
#      pwsh -File tools\apps.ps1 -Gasto
param(
  [string]$Crear = '',
  [string]$Baja = '',
  [switch]$Gasto,
  [string]$Clave = $env:SERVICIO_CLAVE,
  [string]$Api = 'https://openrouter-npiobject-labs.fly.dev'
)
$ErrorActionPreference = 'Stop'
$Api = $Api.TrimEnd('/')

function Aviso($texto) { Write-Host "apps : $texto" }
function Malo($texto)  { Write-Host "apps : ERROR - $texto" -ForegroundColor Red }
function Bien($texto)  { Write-Host "apps : $texto" -ForegroundColor Green }

if (-not $Clave) {
  Aviso "no hay clave en -Clave ni en SERVICIO_CLAVE."
  $Clave = Read-Host "clave de administracion"
}
if (-not $Clave) { Malo "sin clave no se puede administrar"; exit 1 }

function Llamar($ruta, $metodo = 'GET', $cuerpo = $null) {
  $parametros = @{
    Uri     = "$Api$ruta"
    Method  = $metodo
    Headers = @{ Authorization = "Bearer $Clave" }
    UseBasicParsing = $true
  }
  if ($cuerpo) {
    $parametros.ContentType = 'application/json'
    $parametros.Body = [Text.Encoding]::UTF8.GetBytes(($cuerpo | ConvertTo-Json -Compress))
  }
  try {
    $r = Invoke-WebRequest @parametros
  } catch {
    # El cuerpo del error vive en ErrorDetails en las dos versiones de
    # PowerShell; leerlo del stream solo funciona en 5.1 y ahi ya viene
    # consumido.
    $texto = $_.ErrorDetails.Message
    $fallo = $null
    if ($texto) { try { $fallo = ($texto | ConvertFrom-Json).error } catch {} }
    if ($fallo -and $fallo.message) { Malo "$($fallo.code): $($fallo.message)" }
    elseif ($texto) { Malo $texto.Trim() }
    else { Malo $_.Exception.Message }
    exit 1
  }
  return ([Text.Encoding]::UTF8.GetString($r.RawContentStream.ToArray()) | ConvertFrom-Json)
}

# --- crear ------------------------------------------------------------------
if ($Crear) {
  $r = Llamar '/v1/apps' 'POST' @{ nombre = $Crear }
  Bien "creada $($r.app.id) para «$($r.app.nombre)»"
  Write-Host ""
  Write-Host "  $($r.clave)" -ForegroundColor Yellow
  Write-Host ""
  Aviso "guarda esa clave ahora: el servicio solo conserva su hash y no se puede volver a consultar."
  exit 0
}

# --- dar de baja ------------------------------------------------------------
if ($Baja) {
  $confirma = Read-Host "Su clave dejara de funcionar al momento. Escribe el id para confirmar"
  if ($confirma -ne $Baja) { Aviso "cancelado"; exit 0 }
  $null = Llamar "/v1/apps/$Baja" 'DELETE'
  Bien "$Baja dada de baja. Su gasto sigue en el historico."
  exit 0
}

# --- gasto por aplicacion ---------------------------------------------------
if ($Gasto) {
  $resumen = Llamar '/v1/uso/resumen?agrupar=app'
  if (-not $resumen.data) { Aviso "todavia no hay llamadas registradas"; exit 0 }

  $nombres = @{}
  foreach ($a in (Llamar '/v1/apps').data) { $nombres[$a.id] = $a.nombre }

  $resumen.data | ForEach-Object {
    [pscustomobject]@{
      Aplicacion = if ($nombres[$_.grupo]) { $nombres[$_.grupo] } else { $_.grupo }
      Llamadas   = $_.llamadas
      Fallos     = $_.fallos
      Tokens     = "$($_.tokens_entrada)+$($_.tokens_salida)"
      Coste      = '{0:N6} $' -f $_.coste
    }
  } | Format-Table -AutoSize | Out-String -Width 200 | Write-Host

  Aviso ("total: {0} llamadas, {1:N6} `$" -f $resumen.totales.llamadas, $resumen.totales.coste)
  exit 0
}

# --- listar -----------------------------------------------------------------
$r = Llamar '/v1/apps'
if (-not $r.data) {
  Aviso "no hay ninguna aplicacion dada de alta. Crea una con -Crear <nombre>."
  exit 0
}
$r.data | ForEach-Object {
  [pscustomobject]@{
    Aplicacion = $_.nombre
    Id         = $_.id
    Creada     = if ($_.creada -is [datetime]) { $_.creada.ToString('yyyy-MM-dd') }
                 else { "$($_.creada)".Substring(0, 10) }
    Clave      = if ($_.activa) { 'activa' } else { 'dada de baja' }
  }
} | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
