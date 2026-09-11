# Arranca la app entera en el PC: backend de Rust + docs/ servido por HTTP.
# No toca la nube ni necesita Docker. Ctrl+C para el backend y el servidor web.
# Uso: pwsh -File tools\arrancar.ps1
#      pwsh -File tools\arrancar.ps1 -PuertoApi 9080 -PuertoWeb 9081
#      pwsh -File tools\arrancar.ps1 -Release -SinNavegador
param(
  [int]$PuertoApi = 8080,
  [int]$PuertoWeb = 8081,
  [switch]$Release,
  [switch]$SinNavegador
)
$ErrorActionPreference = 'Stop'

$Raiz = Split-Path -Parent $PSScriptRoot
$App  = Join-Path $Raiz 'app'
$Docs = Join-Path $Raiz 'docs'

foreach ($carpeta in @($App, $Docs)) {
  if (-not (Test-Path $carpeta)) {
    Write-Host "arrancar : ERROR - no encuentro $carpeta. Ejecuta el script desde el repo." -ForegroundColor Red
    exit 1
  }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  Write-Host "arrancar : ERROR - falta cargo. Instala Rust desde https://rustup.rs y reabre la consola." -ForegroundColor Red
  exit 1
}

# --- compilar ---------------------------------------------------------------
$perfil = if ($Release) { 'release' } else { 'debug' }
Write-Host "arrancar : compilando el backend ($perfil)..."
$argumentos = @('build', '--manifest-path', (Join-Path $App 'Cargo.toml'))
if ($Release) { $argumentos += '--release' }
& cargo @argumentos
if ($LASTEXITCODE -ne 0) {
  Write-Host "arrancar : ERROR - cargo build fallo con codigo $LASTEXITCODE." -ForegroundColor Red
  exit 1
}

$binario = Join-Path $App "target\$perfil\desdemovil-backend.exe"
if (-not (Test-Path $binario)) {
  Write-Host "arrancar : ERROR - no encuentro el binario en $binario." -ForegroundColor Red
  exit 1
}

# --- arrancar el backend ----------------------------------------------------
$sha = & git -C $Raiz rev-parse --short HEAD 2>$null
if ($LASTEXITCODE -ne 0 -or -not $sha) { $sha = 'sin-git' }

$env:BUILD_ID = "local-$sha"
$env:PUERTO   = $PuertoApi
$backend = Start-Process -FilePath $binario -PassThru -NoNewWindow

# El puerto tarda un instante en abrirse: se espera antes de anunciar nada.
$listo = $false
foreach ($intento in 1..40) {
  Start-Sleep -Milliseconds 250
  if ($backend.HasExited) { break }
  try {
    $prueba = New-Object System.Net.Sockets.TcpClient
    $prueba.Connect('127.0.0.1', $PuertoApi)
    $prueba.Close()
    $listo = $true
    break
  } catch { }
}

if (-not $listo) {
  if (-not $backend.HasExited) { $backend | Stop-Process -Force }
  Write-Host "arrancar : ERROR - el backend no abrio el puerto $PuertoApi. Puerto ocupado?" -ForegroundColor Red
  exit 1
}

Write-Host "arrancar : backend en http://localhost:$PuertoApi/ (build local-$sha)"

# --- servidor estatico de docs/ ---------------------------------------------
$tipos = @{
  '.html' = 'text/html; charset=utf-8'; '.css' = 'text/css; charset=utf-8'
  '.js'   = 'text/javascript; charset=utf-8'; '.json' = 'application/json; charset=utf-8'
  '.md'   = 'text/markdown; charset=utf-8'; '.svg' = 'image/svg+xml'
  '.png'  = 'image/png'; '.jpg' = 'image/jpeg'; '.ico' = 'image/x-icon'
}

$servidor = New-Object System.Net.HttpListener
$servidor.Prefixes.Add("http://localhost:$PuertoWeb/")
$servidor.Start()

$url = "http://localhost:$PuertoWeb/?api=$PuertoApi"
Write-Host "arrancar : web en $url"
Write-Host "arrancar : Ctrl+C para parar los dos."
if (-not $SinNavegador) { Start-Process $url | Out-Null }

try {
  while ($servidor.IsListening) {
    $contexto = $servidor.GetContext()
    $ruta = [System.Uri]::UnescapeDataString($contexto.Request.Url.AbsolutePath).TrimStart('/')
    if ($ruta -eq '' -or $ruta.EndsWith('/')) { $ruta += 'index.html' }

    $fichero = Join-Path $Docs ($ruta -replace '/', '\')
    $completa = [System.IO.Path]::GetFullPath($fichero)

    # Nada fuera de docs/, aunque la peticion venga con ..\ dentro.
    if (-not $completa.StartsWith([System.IO.Path]::GetFullPath($Docs), [StringComparison]::OrdinalIgnoreCase)) {
      $contexto.Response.StatusCode = 403
      $contexto.Response.Close()
      continue
    }

    if (Test-Path -LiteralPath $completa -PathType Leaf) {
      $cuerpo = [System.IO.File]::ReadAllBytes($completa)
      $extension = [System.IO.Path]::GetExtension($completa).ToLowerInvariant()
      $contexto.Response.ContentType = if ($tipos.ContainsKey($extension)) { $tipos[$extension] } else { 'application/octet-stream' }
      $contexto.Response.Headers.Add('Cache-Control', 'no-store')
      $contexto.Response.ContentLength64 = $cuerpo.Length
      $contexto.Response.OutputStream.Write($cuerpo, 0, $cuerpo.Length)
    }
    else {
      $contexto.Response.StatusCode = 404
      $mensaje = [System.Text.Encoding]::UTF8.GetBytes("404: $ruta")
      $contexto.Response.OutputStream.Write($mensaje, 0, $mensaje.Length)
    }
    $contexto.Response.Close()
  }
}
finally {
  if ($servidor.IsListening) { $servidor.Stop() }
  $servidor.Close()
  if (-not $backend.HasExited) { $backend | Stop-Process -Force }
  Write-Host "arrancar : parado."
}
