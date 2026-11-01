# Smoke-launch the packaged Windows build: install the NSIS setup silently for
# the current user, start PR Nexus, and check it is still running after a few
# seconds. Also checks the MSI is a valid Windows Installer package.
#
# Usage: pwsh scripts/smoke-windows.ps1 <bundle-dir> [seconds]
param(
  [Parameter(Mandatory = $true)][string]$Bundle,
  [int]$Seconds = 15
)
$ErrorActionPreference = 'Stop'

$setup = Get-ChildItem -Path (Join-Path $Bundle 'nsis') -Filter '*-setup.exe' | Select-Object -First 1
$msi = Get-ChildItem -Path (Join-Path $Bundle 'msi') -Filter '*.msi' | Select-Object -First 1
if (-not $setup) { throw "no NSIS setup in $Bundle\nsis" }
if (-not $msi) { throw "no MSI in $Bundle\msi" }

$installer = New-Object -ComObject WindowsInstaller.Installer
$db = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @($msi.FullName, 0))
$view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db, @("SELECT Value FROM Property WHERE Property='ProductVersion'"))
$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
$record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
$msiVersion = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
Write-Host "MSI $($msi.Name) ProductVersion $msiVersion"

$proc = Start-Process -FilePath $setup.FullName -ArgumentList '/S' -Wait -PassThru
if ($proc.ExitCode -ne 0) { throw "NSIS setup exited with $($proc.ExitCode)" }

$exe = Get-ChildItem -Path (Join-Path $env:LOCALAPPDATA 'PR Nexus'), (Join-Path $env:ProgramFiles 'PR Nexus') -Filter 'pr-nexus.exe' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $exe) { throw 'pr-nexus.exe not found after install' }
Write-Host "Installed at $($exe.FullName)"

$app = Start-Process -FilePath $exe.FullName -PassThru
Start-Sleep -Seconds $Seconds
$app.Refresh()
if ($app.HasExited) { throw "pr-nexus.exe exited within ${Seconds}s (code $($app.ExitCode))" }
Write-Host "pr-nexus.exe still running after ${Seconds}s (pid $($app.Id))"
Stop-Process -Id $app.Id -Force
Write-Host 'Windows smoke test passed.'
