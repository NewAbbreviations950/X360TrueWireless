<#
Put Windows' WinUSB driver on the Xbox 360 Wireless Receiver, so X360TrueWireless can drive it,
or (-Uninstall) give the receiver back to Windows' own Xbox driver. Run as administrator.

This is what Zadig does: WinUSB ships with Windows, but a driver package has to be signed, so
we sign a two-line package for the receiver with a certificate made on this PC. The certificate
is trusted on this PC only, and its private key is deleted right after signing, so it can't
sign anything else. Windows ranks its own Xbox driver higher, so the update is forced.

Exit codes: 0 done, 2 the package is installed but no receiver is plugged in (plug it in and
run this again), 3 done but Windows' Xbox driver still holds the receiver until it's unplugged
and plugged back in, 1 failed.
#>
param([switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$Name = 'x360truewireless-receiver'
$Subject = 'CN=X360TrueWireless receiver driver (this PC only)'
$Dir = Join-Path $env:ProgramData 'X360TrueWireless\driver'
$Inf = Join-Path $Dir "$Name.inf"
$Cat = Join-Path $Dir "$Name.cat"
$HardwareIds = @('USB\VID_045E&PID_0719', 'USB\VID_045E&PID_0291', 'USB\VID_045E&PID_02A9')

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NewDev {
    [DllImport("newdev.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool UpdateDriverForPlugAndPlayDevices(
        IntPtr hwnd, string hardwareId, string infPath, uint flags, out bool reboot);
}
'@

function Remove-Certificates {
    # -DeleteKey only exists for Cert: paths, so it can't bind to piped input: pass the path.
    foreach ($store in 'Root', 'TrustedPublisher', 'My') {
        Get-ChildItem "Cert:\LocalMachine\$store" |
            Where-Object Subject -eq $Subject |
            ForEach-Object { Remove-Item -LiteralPath $_.PSPath -DeleteKey -ErrorAction SilentlyContinue }
    }
}

# Our packages in the driver store, by their published (oemNN.inf) names.
function Get-Packages {
    $published = $null
    foreach ($line in (pnputil /enum-drivers)) {
        if ($line -match '^\s*Published Name\s*:\s*(\S+)') { $published = $Matches[1] }
        elseif ($line -match '^\s*Original Name\s*:\s*(\S+)' -and $Matches[1] -eq "$Name.inf") {
            $published
        }
    }
}

if ($Uninstall) {
    foreach ($package in Get-Packages) {
        pnputil /delete-driver $package /uninstall /force | Out-Null
    }
    # The receiver goes back to the next best driver: Windows' own.
    pnputil /scan-devices | Out-Null
    Remove-Certificates
    Remove-Item -Recurse -Force $Dir -ErrorAction SilentlyContinue
    exit 0
}

try {
    New-Item -ItemType Directory -Force $Dir | Out-Null
    $models = ($HardwareIds | ForEach-Object { "%DeviceName% = Receiver, $_" }) -join "`r`n"
    $guid = '{0BB7C9A5-5C3B-4F2E-9A47-3D7A2B61C0F1}'
    @"
[Version]
Signature   = "`$Windows NT`$"
Class       = USBDevice
ClassGUID   = {88BAE032-5A81-49f0-BC3D-A4FF138216D6}
Provider    = %Provider%
CatalogFile = $Name.cat
DriverVer   = 10/27/2026,1.0.0.0

[Manufacturer]
%Provider% = Models,NTamd64,NTarm64

[Models.NTamd64]
$models

[Models.NTarm64]
$models

[Receiver]
Include = winusb.inf
Needs   = WINUSB.NT

[Receiver.Services]
Include = winusb.inf
Needs   = WINUSB.NT.Services

[Receiver.HW]
AddReg = Receiver.AddReg

[Receiver.AddReg]
HKR,,DeviceInterfaceGUIDs,0x10000,"$guid"

[Strings]
Provider   = "X360TrueWireless"
DeviceName = "Xbox 360 Wireless Receiver (X360TrueWireless)"
"@ | Set-Content -Encoding Unicode $Inf

    # Sign the package with a fresh certificate, trust it, and throw the key away.
    Remove-Certificates
    Remove-Item $Cat -ErrorAction SilentlyContinue
    New-FileCatalog -Path $Inf -CatalogFilePath $Cat -CatalogVersion 2.0 | Out-Null
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $Subject `
        -CertStoreLocation Cert:\LocalMachine\My -NotAfter (Get-Date).AddYears(50)
    $public = Join-Path $Dir "$Name.cer"
    Export-Certificate -Cert $cert -FilePath $public | Out-Null
    foreach ($store in 'Root', 'TrustedPublisher') {
        Import-Certificate -FilePath $public -CertStoreLocation "Cert:\LocalMachine\$store" | Out-Null
    }
    $signed = Set-AuthenticodeSignature -FilePath $Cat -Certificate $cert -HashAlgorithm SHA256
    Remove-Item "Cert:\LocalMachine\My\$($cert.Thumbprint)" -DeleteKey
    if ($signed.Status -ne 'Valid') {
        throw "signing the catalog failed: $($signed.StatusMessage)"
    }

    # Replace an older package, then add this one to the driver store.
    foreach ($package in Get-Packages) {
        pnputil /delete-driver $package /force | Out-Null
    }
    pnputil /add-driver $Inf | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "pnputil couldn't add the driver package ($LASTEXITCODE)" }

    # INSTALLFLAG_FORCE: use it even though Windows' Xbox driver ranks higher.
    $installed = $false
    $replug = $false
    foreach ($id in $HardwareIds) {
        $reboot = $false
        if ([NewDev]::UpdateDriverForPlugAndPlayDevices([IntPtr]::Zero, $id, $Inf, 1, [ref]$reboot)) {
            $installed = $true
            $replug = $replug -or $reboot
            Write-Output "WinUSB is now the driver for $id"
        }
    }
    if (-not $installed) {
        Write-Output 'No receiver is plugged in. Plug it in, then run "Set up receiver" again.'
        exit 2
    }
    # Windows asks for a restart when its Xbox driver won't let go; replugging is enough.
    if ($replug) {
        Write-Output 'Unplug the receiver and plug it back in to finish.'
        exit 3
    }
    exit 0
}
catch {
    # Continue, or with ErrorActionPreference Stop this would throw instead of reporting.
    Write-Error $_ -ErrorAction Continue
    exit 1
}
