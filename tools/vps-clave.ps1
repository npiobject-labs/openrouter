<#
.SYNOPSIS
  Prepara en un solo paso el acceso de GitHub Actions al VPS: clave SSH
  dedicada, autorizacion en el servidor, huella, y los secretos del repo.

.DESCRIPTION
  Se ejecuta en el PC, desde PowerShell. Hace, en orden:
    1. Averigua el servidor: el que pases con -Servidor, o el de ~/.ssh/config
       si solo hay uno, o lo pregunta.
    2. Genera la clave ed25519 dedicada (deploy_openrouter) si no existe.
    3. Autoriza su parte publica en el VPS entrando como entras hoy (con tu
       clave personal). Es idempotente: no duplica la linea.
    4. Comprueba que la clave nueva entra sin contrasena.
    5. Obtiene la huella del servidor (ssh-keyscan).
    6. Sube VPS_HOST, VPS_PUERTO, VPS_USUARIO, VPS_SSH_CLAVE y VPS_HOST_KEY
       como secretos del repositorio, y API_DOMINIO como variable. Si tienes
       la CLI "gh" autenticada lo hace sola; si no, te va copiando cada valor
       al portapapeles y abre la pagina de GitHub para pegarlo.

  Nunca imprime la clave privada. No toca nada del VPS salvo
  ~/.ssh/authorized_keys del usuario indicado.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\vps-clave.ps1
  powershell -ExecutionPolicy Bypass -File tools\vps-clave.ps1 -Servidor 203.0.113.10 -Usuario deployer
#>
param(
    [string]$Servidor,
    [string]$Usuario = "deployer",
    [int]$Puerto = 0,
    [string]$Clave = "$HOME\deploy_openrouter",
    [string]$Repo = "npiobject-labs/openrouter",
    [string]$Dominio = "apisor.oracle402.com",
    [switch]$SinSubir
)

# "Continue", no "Stop": ssh y ssh-keyscan escriben avisos por stderr y en
# PowerShell 5.1 eso pararia el script. Los fallos se miran por $LASTEXITCODE.
$ErrorActionPreference = "Continue"

function Paso($n, $texto) { Write-Host ""; Write-Host "== Paso $n. $texto" -ForegroundColor Cyan }
function Ok($texto) { Write-Host "   OK: $texto" -ForegroundColor Green }
function Fallo($texto) { Write-Host "   ERROR: $texto" -ForegroundColor Red; exit 1 }

# --- 1. Servidor -----------------------------------------------------------
Paso 1 "Servidor"

# Lee ~/.ssh/config y devuelve los bloques Host con su HostName real.
function Leer-ConfigSsh {
    $ruta = "$HOME\.ssh\config"
    if (-not (Test-Path $ruta)) { return @() }
    $bloques = @(); $actual = $null
    foreach ($linea in Get-Content $ruta) {
        $l = $linea.Trim()
        if ($l -eq "" -or $l.StartsWith("#")) { continue }
        $partes = $l -split "\s+", 2
        $clave = $partes[0].ToLower(); $valor = if ($partes.Count -gt 1) { $partes[1].Trim() } else { "" }
        switch ($clave) {
            "host"         { if ($actual) { $bloques += $actual }; $actual = @{ Alias = $valor; HostName = $valor; Port = 22; User = $null } }
            "hostname"     { if ($actual) { $actual.HostName = $valor } }
            "port"         { if ($actual) { $actual.Port = [int]$valor } }
            "user"         { if ($actual) { $actual.User = $valor } }
        }
    }
    if ($actual) { $bloques += $actual }
    # Fuera los comodines: "Host *" no es un servidor.
    return $bloques | Where-Object { $_.Alias -notmatch "[\*\?]" }
}

$config = Leer-ConfigSsh
$bloque = $null
if ($Servidor) {
    $bloque = $config | Where-Object { $_.Alias -eq $Servidor -or $_.HostName -eq $Servidor } | Select-Object -First 1
} elseif ($config.Count -eq 1) {
    $bloque = $config[0]
    Write-Host "   Encontrado en ~/.ssh/config: $($bloque.Alias) -> $($bloque.HostName)"
} elseif ($config.Count -gt 1) {
    Write-Host "   Hay varios servidores en ~/.ssh/config:"
    for ($i = 0; $i -lt $config.Count; $i++) { Write-Host "     [$($i+1)] $($config[$i].Alias) -> $($config[$i].HostName)" }
    $eleccion = Read-Host "   Cual es el VPS (numero, o escribe una IP)"
    if ($eleccion -match "^\d+$" -and [int]$eleccion -le $config.Count) { $bloque = $config[[int]$eleccion - 1] } else { $Servidor = $eleccion }
} else {
    $Servidor = Read-Host "   IP o nombre del VPS (el que usas con ssh)"
}

if ($bloque) {
    $HostReal = $bloque.HostName
    if ($Puerto -eq 0) { $Puerto = $bloque.Port }
    if ($bloque.User -and -not $PSBoundParameters.ContainsKey("Usuario")) { $Usuario = $bloque.User }
} else {
    $HostReal = $Servidor
    if ($Puerto -eq 0) { $Puerto = 22 }
}
if (-not $HostReal) { Fallo "sin servidor." }
Write-Host "   Servidor: $HostReal  puerto: $Puerto  usuario: $Usuario"

# --- 2. Clave dedicada -----------------------------------------------------
Paso 2 "Clave SSH dedicada"
if (Test-Path $Clave) {
    Ok "ya existe $Clave"
} else {
    & ssh-keygen -t ed25519 -f $Clave -N '""' -C "deploy-apisor" | Out-Null
    if ($LASTEXITCODE -ne 0) { Fallo "ssh-keygen fallo." }
    Ok "generada $Clave"
}
$Publica = (Get-Content "$Clave.pub" -Raw).Trim()
if (-not $Publica.StartsWith("ssh-ed25519 ")) { Fallo "$Clave.pub no parece una clave publica ed25519." }

# --- 3. Autorizar en el VPS ------------------------------------------------
Paso 3 "Autorizar la clave en el VPS (entrando como hoy)"
$remoto = "mkdir -p ~/.ssh && chmod 700 ~/.ssh && touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && (grep -qF '$Publica' ~/.ssh/authorized_keys || echo '$Publica' >> ~/.ssh/authorized_keys) && echo autorizada"
$salida = & ssh -p $Puerto "$Usuario@$HostReal" $remoto 2>&1
if ($LASTEXITCODE -ne 0 -or ($salida -join "`n") -notmatch "autorizada") {
    Write-Host ($salida -join "`n")
    Fallo "no se pudo autorizar la clave. Comprueba que 'ssh -p $Puerto $Usuario@$HostReal' te deja entrar."
}
Ok "clave publica en ~/.ssh/authorized_keys de $Usuario"

# --- 4. Probar la clave nueva ---------------------------------------------
Paso 4 "Probar la clave dedicada"
$prueba = & ssh -i $Clave -o IdentitiesOnly=yes -o BatchMode=yes -p $Puerto "$Usuario@$HostReal" "echo ok" 2>&1
if (($prueba -join "`n") -notmatch "^ok") { Write-Host ($prueba -join "`n"); Fallo "la clave dedicada no entra." }
Ok "entra sin contrasena"

# --- 5. Huella del servidor -----------------------------------------------
Paso 5 "Huella del servidor"
# Sin -t: se piden todas y se prefiere la ed25519. El comentario que manda por
# stderr se descarta (con ErrorActionPreference=Continue ya no para el script).
$lineas = @(& ssh-keyscan -p $Puerto $HostReal 2>$null | Where-Object { $_ -is [string] -and $_ -notmatch "^#" -and $_.Trim() -ne "" })
$huella = $lineas | Where-Object { $_ -match "ssh-ed25519" } | Select-Object -First 1
if (-not $huella) { $huella = $lineas | Select-Object -First 1 }
if (-not $huella) {
    Write-Host "   ssh-keyscan no devolvio nada. Prueba a mano: ssh-keyscan -p $Puerto $HostReal"
    Fallo "sin huella de $HostReal."
}
Ok "huella obtenida"

# --- 6. Subir a GitHub -----------------------------------------------------
Paso 6 "Secretos del repositorio $Repo"
$Privada = Get-Content $Clave -Raw
$secretos = [ordered]@{
    VPS_HOST      = $HostReal
    VPS_PUERTO    = "$Puerto"
    VPS_USUARIO   = $Usuario
    VPS_SSH_CLAVE = $Privada
    VPS_HOST_KEY  = $huella
}

if ($SinSubir) {
    Write-Host "   -SinSubir: no se sube nada. Valores listos en memoria; vuelve a ejecutar sin -SinSubir."
    exit 0
}

$gh = Get-Command gh -ErrorAction SilentlyContinue
$ghOk = $false
if ($gh) { & gh auth status 2>&1 | Out-Null; $ghOk = ($LASTEXITCODE -eq 0) }

if ($ghOk) {
    foreach ($nombre in $secretos.Keys) {
        $secretos[$nombre] | & gh secret set $nombre -R $Repo
        if ($LASTEXITCODE -ne 0) { Fallo "gh secret set $nombre fallo." }
        Ok "secreto $nombre"
    }
    & gh variable set API_DOMINIO -R $Repo -b $Dominio | Out-Null
    Ok "variable API_DOMINIO = $Dominio"
} else {
    Write-Host "   Sin CLI 'gh' autenticada: se hace por el navegador, un valor cada vez."
    Write-Host "   Para cada uno: el valor ya esta en el portapapeles; en la pagina que se abre,"
    Write-Host "   escribe el nombre en 'Name', pega (Ctrl+V) en 'Secret' y pulsa 'Add secret'."
    $urlSecreto = "https://github.com/$Repo/settings/secrets/actions/new"
    foreach ($nombre in $secretos.Keys) {
        Set-Clipboard -Value $secretos[$nombre]
        Write-Host ""
        Write-Host "   >>> Name: $nombre   (valor copiado al portapapeles)" -ForegroundColor Yellow
        Start-Process $urlSecreto
        Read-Host "   Pulsa Enter cuando lo hayas guardado"
    }
    Set-Clipboard -Value $Dominio
    Write-Host ""
    Write-Host "   >>> Variable (pestaña Variables, no Secrets): Name: API_DOMINIO   Value: $Dominio (copiado)" -ForegroundColor Yellow
    Start-Process "https://github.com/$Repo/settings/variables/actions/new"
    Read-Host "   Pulsa Enter cuando la hayas guardado"
    Set-Clipboard -Value ""
}

Write-Host ""
Write-Host "Listo. Avisa en la sesion para lanzar el workflow 'Inspeccionar el VPS'." -ForegroundColor Green
Write-Host "La clave privada sigue en $Clave; puedes borrarla o guardarla en tu gestor de contrasenas."
