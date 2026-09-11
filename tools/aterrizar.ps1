# Aterrizaje nube -> PC. Idempotente y unidireccional. Sobrescribe la copia local sin preguntar.
# Uso: pwsh -File tools\aterrizar.ps1
#      pwsh -File tools\aterrizar.ps1 -Proyecto MiProyecto -Owner miusuario
#      pwsh -File tools\aterrizar.ps1 -Root 'D:\dev\MiProyecto'
param(
  [string]$Proyecto = 'DesdeMovil',
  [string]$Owner    = 'npiobject-labs',

  [string]$Remote   = "https://github.com/$Owner/$Proyecto.git",
  [string]$Root     = (Join-Path $env:USERPROFILE "C - Desarrollo\$Proyecto"),
  [string]$Rama     = 'main'
)
$ErrorActionPreference = 'Stop'

# $ErrorActionPreference no detiene a los ejecutables externos: tras cada git hay
# que mirar $LASTEXITCODE y abortar antes de imprimir el resumen.
function Assert-Git([string]$Que) {
  if ($LASTEXITCODE -ne 0) {
    Write-Host "$Proyecto : ERROR - $Que fallo con codigo $LASTEXITCODE. Se aborta." -ForegroundColor Red
    exit 1
  }
}

$Repo = Join-Path $Root 'repo'
New-Item -ItemType Directory -Force -Path $Root | Out-Null

if (Test-Path (Join-Path $Repo '.git')) {
  & git -C $Repo fetch --prune origin
  Assert-Git 'git fetch'
  & git -C $Repo reset --hard "origin/$Rama"
  Assert-Git 'git reset --hard'
  & git -C $Repo clean -fdx
  Assert-Git 'git clean'
}
else {
  # repo\ sin .git: solo se clona si no existe o esta vacia.
  if (Test-Path $Repo) {
    $contenido = @(Get-ChildItem -LiteralPath $Repo -Force)
    if ($contenido.Count -gt 0) {
      Write-Host "$Proyecto : $Repo existe, no es un clon de git y NO esta vacia. Contiene:"
      foreach ($item in $contenido) { Write-Host "  $($item.Name)" }
      Write-Host "$Proyecto : ERROR - no se toca nada. Vacia o aparta esa carpeta y vuelve a ejecutar." -ForegroundColor Red
      exit 1
    }
  }
  & git clone --branch $Rama $Remote $Repo
  Assert-Git 'git clone'
}

$sha = & git -C $Repo rev-parse --short HEAD
Assert-Git 'git rev-parse'

Write-Host "$Proyecto : repo/ = origin/$Rama @ $sha"
Write-Host "  local  : $Repo"
Write-Host "  remoto : $Remote"

$Drive = Join-Path $Root 'drive'
if (Test-Path $Drive) {
  Write-Host "  drive/ : $(@(Get-ChildItem -LiteralPath $Drive -Recurse -File).Count) ficheros"
} else {
  Write-Host "  drive/ : no configurado (opcional)"
}
