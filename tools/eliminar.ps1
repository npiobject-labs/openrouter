# Elimina el proyecto sin dejar restos: app de Fly, repositorio de GitHub y copia local.
# Por defecto SOLO SIMULA: sin -Confirmar no borra nada. Con -Confirmar hay que
# escribir el nombre del proyecto para seguir.
# Uso: pwsh -File tools\eliminar.ps1
#      pwsh -File tools\eliminar.ps1 -Confirmar
#      pwsh -File tools\eliminar.ps1 -Confirmar -SinLocal
param(
  [string]$Proyecto = 'DesdeMovil',
  [string]$Owner    = 'npiobject-labs',

  [string]$FlyApp   = '',
  [string]$Root     = (Join-Path $env:USERPROFILE "C - Desarrollo\$Proyecto"),
  [switch]$Confirmar,
  [switch]$SinFly,
  [switch]$SinRepo,
  [switch]$SinLocal
)
$ErrorActionPreference = 'Stop'

# Si el repo define la variable FLY_APP, esa manda: es lo que hace deploy.yml.
if (-not $FlyApp -and (Get-Command gh -ErrorAction SilentlyContinue)) {
  $variable = (& gh variable get FLY_APP --repo "$Owner/$Proyecto" 2>$null)
  if ($LASTEXITCODE -eq 0 -and $variable) {
    $FlyApp = $variable.Trim()
    Write-Host "eliminar : la app sale de la variable de repositorio FLY_APP."
  }
}

# Sin variable, el mismo derivado que deploy.yml: minusculas, todo lo que no sea
# [a-z0-9-] a guion, recorte a 30 y sin guiones al final. Ni colapsa guiones
# repetidos ni quita los del principio, igual que el workflow.
if (-not $FlyApp) {
  $FlyApp = ("$Proyecto-$Owner").ToLowerInvariant() -replace '[^a-z0-9-]', '-'
  if ($FlyApp.Length -gt 30) { $FlyApp = $FlyApp.Substring(0, 30) }
  $FlyApp = $FlyApp -replace '-*$', ''
}

$pendientes = @()

Write-Host ""
Write-Host "Eliminar el proyecto $Proyecto" -ForegroundColor Yellow
Write-Host "  1. Fly.io  : app $FlyApp$(if ($SinFly)   { '  (omitida por -SinFly)' })"
Write-Host "  2. GitHub  : $Owner/$Proyecto$(if ($SinRepo)  { '  (omitido por -SinRepo)' })"
Write-Host "  3. Local   : $Root$(if ($SinLocal) { '  (omitida por -SinLocal)' })"
Write-Host "  4. A mano  : la carpeta de Drive y las sesiones de claude.ai/code"
Write-Host ""

if (-not $Confirmar) {
  Write-Host "Simulacion: no se ha borrado nada. Repite con -Confirmar para hacerlo de verdad." -ForegroundColor Cyan
  exit 0
}

Write-Host "Esto NO se puede deshacer. Escribe el nombre del proyecto para seguir:" -ForegroundColor Red
$respuesta = Read-Host "  nombre"
if ($respuesta -ne $Proyecto) {
  Write-Host "eliminar : el nombre no coincide. No se ha borrado nada." -ForegroundColor Yellow
  exit 1
}

# --- 1. Fly.io -------------------------------------------------------------
# Va primero: una vez borrado el repo ya no queda a mano el nombre de la app.
if (-not $SinFly) {
  if (Get-Command flyctl -ErrorAction SilentlyContinue) {
    & flyctl apps destroy $FlyApp --yes
    if ($LASTEXITCODE -eq 0) {
      Write-Host "eliminar : app de Fly $FlyApp borrada." -ForegroundColor Green
    } else {
      $pendientes += "Fly: borra la app $FlyApp en https://fly.io/dashboard (Settings -> Delete app)."
      Write-Host "eliminar : flyctl fallo con codigo $LASTEXITCODE. Queda pendiente." -ForegroundColor Yellow
    }
  } else {
    $pendientes += "Fly: borra la app $FlyApp en https://fly.io/dashboard (Settings -> Delete app)."
    Write-Host "eliminar : no hay flyctl. Fly queda pendiente." -ForegroundColor Yellow
  }
}

# --- 2. GitHub -------------------------------------------------------------
# Con el repo se van su sitio de Pages, sus variables y sus workflows. El
# secreto FLY_API_TOKEN es de la organizacion y sigue ahi para los demas.
if (-not $SinRepo) {
  if (Get-Command gh -ErrorAction SilentlyContinue) {
    & gh repo delete "$Owner/$Proyecto" --yes
    if ($LASTEXITCODE -eq 0) {
      Write-Host "eliminar : repositorio $Owner/$Proyecto borrado." -ForegroundColor Green
    } else {
      $pendientes += "GitHub: borra el repo en https://github.com/$Owner/$Proyecto/settings (Danger Zone). Puede faltarle a gh el permiso delete_repo: gh auth refresh -h github.com -s delete_repo."
      Write-Host "eliminar : gh fallo con codigo $LASTEXITCODE. Queda pendiente." -ForegroundColor Yellow
    }
  } else {
    $pendientes += "GitHub: borra el repo en https://github.com/$Owner/$Proyecto/settings (Danger Zone)."
    Write-Host "eliminar : no hay gh. El repositorio queda pendiente." -ForegroundColor Yellow
  }
}

# --- 3. Copia local --------------------------------------------------------
# El script vive dentro de esa carpeta: se sale de ella antes de borrarla.
if (-not $SinLocal) {
  if (Test-Path -LiteralPath $Root) {
    $completa = (Resolve-Path -LiteralPath $Root).Path
    if ($completa -notlike "*$Proyecto*") {
      $pendientes += "Local: $completa no lleva el nombre del proyecto; borrala tu si es la correcta."
      Write-Host "eliminar : $completa no lleva el nombre del proyecto. No se toca." -ForegroundColor Yellow
    } else {
      Set-Location $env:USERPROFILE
      Remove-Item -LiteralPath $completa -Recurse -Force
      Write-Host "eliminar : copia local $completa borrada." -ForegroundColor Green
    }
  } else {
    Write-Host "eliminar : no habia copia local en $Root."
  }
}

# --- 4. Lo que no puede hacer un script ------------------------------------
$pendientes += "Drive: manda la carpeta $Proyecto de Mi unidad a la papelera, si el proyecto la tenia."
$pendientes += "Claude: archiva o borra las sesiones del proyecto en https://claude.ai/code."

Write-Host ""
Write-Host "Queda por hacer a mano:" -ForegroundColor Yellow
foreach ($p in $pendientes) { Write-Host "  - $p" }
