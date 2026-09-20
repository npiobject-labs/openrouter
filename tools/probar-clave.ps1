# Prueba una clave de aplicacion contra el servicio, desde el PC y sin
# escribir la clave en ningun sitio: la pide por pantalla, sin eco.
#
# Es lo que hace el paso 4 de docs/conectar.html, para quien prefiera la
# terminal o quiera comprobar una clave ya creada. Hace tres cosas:
#   1. /v1/estado      -> como quien habla la clave (nombre de la aplicacion)
#   2. /v1/presupuesto -> su margen
#   3. /v1/chat/completions con "Di hola" -> respuesta, coste y X-Uso-Id
# La tercera gasta una fraccion de centimo y queda en el historico. Con
# -SinGasto se salta.
#
# Uso: pwsh -File tools\probar-clave.ps1                          (VPS)
#      pwsh -File tools\probar-clave.ps1 -Api https://openrouter-npiobject-labs.fly.dev
#      pwsh -File tools\probar-clave.ps1 -SinGasto
#
# Vale en PowerShell 5.1 y 7. Lee los bytes en UTF-8 porque 5.1 decodifica
# en Latin-1 lo que no declara charset, y aunque el servicio ya lo declara,
# no cuesta nada no depender de ello.
param(
  [string]$Api = 'https://apisor.oracle402.com',
  [switch]$SinGasto
)
$ErrorActionPreference = 'Stop'
$Api = $Api.TrimEnd('/')

function Aviso($texto) { Write-Host "probar : $texto" }
function Malo($texto)  { Write-Host "probar : ERROR - $texto" -ForegroundColor Red }
function Bien($texto)  { Write-Host "probar : $texto" -ForegroundColor Green }

$segura = Read-Host "clave de aplicacion (no se ve al escribir)" -AsSecureString
$clave = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($segura))
if (-not $clave) { Malo "sin clave no hay nada que probar"; exit 1 }

# Devuelve el JSON decodificado en UTF-8 y las cabeceras de la respuesta.
function Llamar($ruta, $metodo = 'GET', $cuerpo = $null, $extra = @{}) {
  $cabeceras = @{ Authorization = "Bearer $clave" } + $extra
  $parametros = @{ Uri = "$Api$ruta"; Method = $metodo; Headers = $cabeceras; UseBasicParsing = $true; TimeoutSec = 130 }
  if ($cuerpo) {
    $parametros.ContentType = 'application/json'
    $parametros.Body = [Text.Encoding]::UTF8.GetBytes($cuerpo)
  }
  try {
    $r = Invoke-WebRequest @parametros
  } catch {
    $texto = $_.ErrorDetails.Message
    $fallo = $null
    if ($texto) { try { $fallo = ($texto | ConvertFrom-Json).error } catch {} }
    if ($fallo -and $fallo.message) { Malo "$($fallo.code): $($fallo.message)" }
    elseif ($texto) { Malo $texto.Trim() }
    else { Malo $_.Exception.Message }
    exit 1
  }
  $json = [Text.Encoding]::UTF8.GetString($r.RawContentStream.ToArray()) | ConvertFrom-Json
  return @{ datos = $json; cabeceras = $r.Headers }
}

Aviso "servidor $Api"

# 1. Identidad: si esto falla, la clave es de otro servidor o esta dada de baja.
$estado = (Llamar '/v1/estado').datos
$quien = $estado.identidad
if ($quien.administracion) {
  Malo "esa es la clave de administracion, no la de una aplicacion. Vale, pero no es lo que se prueba aqui."
} else {
  Bien "hablas como «$($quien.nombre)» ($($quien.app_id)) · build $($estado.build.Substring(0, [Math]::Min(7, $estado.build.Length)))"
}

# 2. Margen.
$p = (Llamar '/v1/presupuesto').datos
if ($p.periodo) {
  $texto = "{0:N4} $ de {1:N2} $ por {2}" -f $p.consumido, $p.limite, $p.periodo
  if ($p.cuota_minuto) { $texto += ", $($p.cuota_minuto)/min" }
  if ($p.en_aviso) { $texto += " (EN AVISO)" }
  Bien "presupuesto: $texto"
} elseif (-not $quien.administracion) {
  Aviso "sin topes: conviene ponerselos desde docs/conectar.html o la consola antes de la primera llamada real."
}

if ($SinGasto) { Aviso "sin consulta al modelo (-SinGasto)"; exit 0 }

# 3. La llamada de verdad.
$cuerpo = '{"messages":[{"role":"user","content":"Di hola"}],"max_tokens":50}'
$r = Llamar '/v1/chat/completions' 'POST' $cuerpo @{ 'X-Operacion' = 'probar-clave' }
$j = $r.datos
Bien "respuesta: $($j.choices[0].message.content)"
$u = $j.usage
$coste = if ($u.cost -ne $null) { "{0:N6} $" -f $u.cost } else { '?' }
Aviso "coste $coste · tokens $($u.prompt_tokens)+$($u.completion_tokens) · modelo $($j.model)"
Aviso "registro de uso: $($r.cabeceras['X-Uso-Id'])"
$aviso = $r.cabeceras['X-Presupuesto']
if ($aviso) { Write-Host "probar : AVISO de presupuesto: $aviso" -ForegroundColor Yellow }
