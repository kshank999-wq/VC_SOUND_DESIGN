# Signs one Windows file with Azure Trusted Signing (now "Artifact Signing").
# Tauri calls this for every file it signs (the app, the .msi/.exe installers and the
# update bundles); see the bundle.windows.signCommand that scripts/release-config.mjs
# writes. The release workflow sets up everything it reads:
#
#   VCSD_SIGNTOOL       signtool.exe from the Windows SDK (x64, to match the dlib)
#   VCSD_SIGN_DLIB      Azure.CodeSigning.Dlib.dll from Microsoft.ArtifactSigning.Client
#   VCSD_SIGN_METADATA  JSON naming the endpoint, signing account and certificate profile
#   AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET
#                       the app registration the dlib signs in as (read by the dlib itself)
#
# The private key never leaves Microsoft's HSM: nothing here is a key or a certificate.
param([Parameter(Mandatory = $true)][string]$File)
$ErrorActionPreference = 'Stop'

foreach ($name in 'VCSD_SIGNTOOL', 'VCSD_SIGN_DLIB', 'VCSD_SIGN_METADATA', 'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET') {
  if (-not [Environment]::GetEnvironmentVariable($name)) { throw "sign.ps1: $name is not set (the release workflow's signing step sets it up)" }
}
if (-not (Test-Path -LiteralPath $File)) { throw "sign.ps1: no file to sign: $File" }

Write-Host "Signing $File"
# /d names the content: Windows shows it as the program's name in the UAC prompt.
& $env:VCSD_SIGNTOOL sign /v /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 `
  /d 'VC Sound Design' /dlib $env:VCSD_SIGN_DLIB /dmdf $env:VCSD_SIGN_METADATA $File
if ($LASTEXITCODE -ne 0) {
  throw "sign.ps1: signtool failed ($LASTEXITCODE) for $File. If sign-in worked but signing was refused, the app registration needs the 'Trusted Signing Certificate Profile Signer' role on the signing account (RELEASING.md)."
}
