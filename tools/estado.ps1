# Esta mi copia local al dia? Solo lectura.
# Uso: pwsh -File tools\estado.ps1
#      pwsh -File tools\estado.ps1 -Proyecto MiProyecto -Root 'D:\dev\MiProyecto'
param(
  [string]$Proyecto = 'DesdeMovil',

  [string]$Root = (Join-Path $env:USERPROFILE "C - Desarrollo\$Proyecto"),
  [string]$Rama = 'main'
)
$ErrorActionPreference = 'Stop'

# git no lanza excepciones: se mira $LASTEXITCODE y se aborta sin imprimir resumen.
function Assert-Git([string]$Que) {
  if ($LASTEXITCODE -ne 0) {
    Write-Host "$Proyecto : ERROR - $Que fallo con codigo $LASTEXITCODE. Se aborta." -ForegroundColor Red
    exit 1
  }
}

$Repo = Join-Path $Root 'repo'
if (-not (Test-Path (Join-Path $Repo '.git'))) {
  if (Test-Path $Repo) {
    $contenido = @(Get-ChildItem -LiteralPath $Repo -Force)
    if ($contenido.Count -gt 0) {
      Write-Host "$Proyecto : $Repo existe, no es un clon de git y NO esta vacia. Contiene:"
      foreach ($item in $contenido) { Write-Host "  $($item.Name)" }
      Write-Host "$Proyecto : ERROR - vacia o aparta esa carpeta antes de ejecutar tools\aterrizar.ps1." -ForegroundColor Red
      exit 1
    }
  }
  Write-Host "$Proyecto : sin copia local en $Repo -> ejecuta tools\aterrizar.ps1"
  exit 1
}

& git -C $Repo fetch --quiet origin
Assert-Git 'git fetch'
$local = & git -C $Repo rev-parse HEAD
Assert-Git 'git rev-parse HEAD'
$remote = & git -C $Repo rev-parse "origin/$Rama"
Assert-Git "git rev-parse origin/$Rama"

if ($local -eq $remote) {
  Write-Host "$Proyecto : AL DIA ($($local.Substring(0,7)))"
} else {
  Write-Host "$Proyecto : DESACTUALIZADO -> local $($local.Substring(0,7)) / nube $($remote.Substring(0,7)) -> ejecuta tools\aterrizar.ps1"
}

$Drive = Join-Path $Root 'drive'
if (Test-Path $Drive) {
  $f = Get-ChildItem -LiteralPath $Drive -Recurse -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($f) { Write-Host "  drive/: ultimo fichero $($f.Name) $($f.LastWriteTime)" }
} else {
  Write-Host "  drive/: no configurado (opcional)"
}
