<#
  Installs rxmltv as a real Windows service with NSSM (https://nssm.cc/), which turns a console program into a service.
  Run it in an elevated PowerShell (Run as administrator).

    winget install NSSM.NSSM            # or: choco install nssm
    .\install-service.ps1 -Exe 'C:\Program Files\rxmltv\rxmltv.exe'

  By default the service runs as the built-in Local Service account, with its files in C:\ProgramData\rxmltv.
  To run it as a limited user of your own instead (recommended if it should use that user's settings):

    .\install-service.ps1 -Exe '...' -Account '.\rxmltv-svc' -Password 'secret' -DataDir 'C:\Users\rxmltv-svc'

  NSSM gives the account the "Log on as a service" right. The settings file is <DataDir>\.rxmltv\rxmltv.conf.
  Remove the service with:  nssm remove rxmltv confirm
#>
param(
    [string]$Exe = 'C:\Program Files\rxmltv\rxmltv.exe',
    [string]$DataDir = 'C:\ProgramData\rxmltv',
    [string]$Account = 'NT AUTHORITY\LocalService',
    [string]$Password = ''
)

$ErrorActionPreference = 'Stop'
if (-not (Get-Command nssm -ErrorAction SilentlyContinue)) { throw 'nssm was not found. Install it first (winget install NSSM.NSSM).' }
if (-not (Test-Path $Exe)) { throw "rxmltv.exe not found at $Exe" }

New-Item -ItemType Directory -Force -Path $DataDir, "$DataDir\logs" | Out-Null

nssm install rxmltv $Exe --service
nssm set rxmltv DisplayName 'rxmltv'
nssm set rxmltv Description 'XMLTV guide updater, HTTP server and notifications'
nssm set rxmltv AppDirectory $DataDir
nssm set rxmltv AppEnvironmentExtra "HOME=$DataDir"
nssm set rxmltv AppStdout "$DataDir\logs\rxmltv.log"
nssm set rxmltv AppStderr "$DataDir\logs\rxmltv.log"
nssm set rxmltv AppRotateFiles 1
nssm set rxmltv AppRotateOnline 1
nssm set rxmltv AppRotateBytes 5242880
nssm set rxmltv AppExit Default Restart
nssm set rxmltv AppRestartDelay 30000
nssm set rxmltv Start SERVICE_AUTO_START

if ($Account -ne 'NT AUTHORITY\LocalService') {
    nssm set rxmltv ObjectName $Account $Password
} else {
    nssm set rxmltv ObjectName $Account
}

# the account must be able to write its own folder
icacls $DataDir /grant "${Account}:(OI)(CI)M" | Out-Null

nssm start rxmltv
Write-Host "Started. Log: $DataDir\logs\rxmltv.log   Settings: $DataDir\.rxmltv\rxmltv.conf"
