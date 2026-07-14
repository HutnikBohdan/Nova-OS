<#
.SYNOPSIS
Bootstraps or verifies the pinned Nova OS Windows build environment.

.DESCRIPTION
Verify checks the Rust toolchain, locked dependency resolution, and optional
QEMU/firmware hashes recorded in build-support/nova-build-lock.psd1.
Bootstrap installs the pinned Rust components and fetches Cargo.lock sources.
It does not download QEMU, OVMF firmware, or vendor crates into the repository.

.PARAMETER Mode
Verify (default) performs read-only checks. Bootstrap installs/fetches pinned
Rust and Cargo dependencies before running the same checks.

.PARAMETER Offline
Requires the committed vendor tree and resolves metadata through an isolated
temporary CARGO_HOME with Cargo network access disabled.

.PARAMETER Build
Runs the root `cargo build --locked --offline` through the isolated CARGO_HOME.
Nested bootloader Cargo processes inherit the same absolute vendor source.

.PARAMETER RequireQemu
Treats a missing local tools/qemu bundle as a verification failure instead of
an expected warning for a clean source clone.

.PARAMETER Help
Prints a compact usage summary without changing or verifying the environment.

.EXAMPLE
pwsh -File scripts/Verify-NovaBuildEnvironment.ps1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/Verify-NovaBuildEnvironment.ps1 -Mode Bootstrap

.EXAMPLE
pwsh -File scripts/Verify-NovaBuildEnvironment.ps1 -Offline -RequireQemu

.EXAMPLE
pwsh -File scripts/Verify-NovaBuildEnvironment.ps1 -Build
#>
[CmdletBinding()]
param(
    [ValidateSet('Verify', 'Bootstrap')]
    [string]$Mode = 'Verify',

    [switch]$Offline,

    [switch]$Build,

    [switch]$RequireQemu,

    [string]$ManifestPath,

    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8

if ($Help) {
    Write-Output @'
Verify-NovaBuildEnvironment.ps1 [-Mode Verify|Bootstrap] [-Offline] [-Build] [-RequireQemu]
                                [-ManifestPath <path>] [-Help]

Verify is read-only. Bootstrap installs the pinned rustup toolchain and runs
cargo fetch --locked. Build performs a root build with network disabled.
'@
    exit 0
}

$script:Failures = 0
$script:Warnings = 0

function Write-Pass([string]$Message) {
    Write-Host "PASS  $Message" -ForegroundColor Green
}

function Write-WarningResult([string]$Message) {
    $script:Warnings++
    Write-Host "WARN  $Message" -ForegroundColor Yellow
}

function Write-Failure([string]$Message) {
    $script:Failures++
    Write-Host "FAIL  $Message" -ForegroundColor Red
}

function Find-Program([string]$Name) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\$Name.exe"
    if (Test-Path -LiteralPath $fallback -PathType Leaf) {
        return $fallback
    }
    return $null
}

function Invoke-Captured([string]$File, [string[]]$Arguments) {
    # Windows PowerShell 5.1 wraps native stderr as ErrorRecord objects and,
    # under Stop, can abort successful tools that print progress to stderr.
    $savedPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $text = & $File @Arguments 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $savedPreference
    }
    return @{
        ExitCode = $exitCode
        Text = $text.Trim()
    }
}

function Test-PinnedFile([string]$Root, [hashtable]$Entry, [string]$Label) {
    $path = Join-Path $Root $Entry.Path
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Write-Failure "$Label is missing: $($Entry.Path)"
        return
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
    if ($actual -ne $Entry.Sha256) {
        Write-Failure "$Label checksum mismatch: $($Entry.Path) expected=$($Entry.Sha256) actual=$actual"
        return
    }
    Write-Pass "$Label checksum: $($Entry.Path)"
}

function New-IsolatedCargoHome([string]$Root, [hashtable]$OfflinePolicy) {
    $base = Join-Path $Root $OfflinePolicy.TemporaryCargoHome
    $cargoHomePath = Join-Path $base ([Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $cargoHomePath | Out-Null
    $vendor = (Resolve-Path -LiteralPath (Join-Path $Root $OfflinePolicy.VendorDirectory)).Path
    $vendorToml = $vendor.Replace('\', '/').Replace('"', '\"')
    $config = @"
[unstable]
bindeps = true

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "$vendorToml"

[net]
offline = true

[env]
NOVA_VENDOR_DIR = { value = "$vendorToml", force = true }
"@
    [IO.File]::WriteAllText((Join-Path $cargoHomePath 'config.toml'), $config, (New-Object Text.UTF8Encoding($false)))
    return $cargoHomePath
}

function Invoke-WithIsolatedCargoHome(
    [string]$Cargo,
    [string]$Root,
    [hashtable]$OfflinePolicy,
    [string[]]$CargoArguments,
    [bool]$UseTemporaryTarget = $false
) {
    $cargoHomePath = New-IsolatedCargoHome $Root $OfflinePolicy
    $oldCargoHome = $env:CARGO_HOME
    $oldOffline = $env:CARGO_NET_OFFLINE
    $oldTarget = $env:CARGO_TARGET_DIR
    $temporaryTarget = $null
    try {
        $env:CARGO_HOME = $cargoHomePath
        $env:CARGO_NET_OFFLINE = 'true'
        if ($UseTemporaryTarget) {
            $temporaryTarget = Join-Path $env:TEMP ("nova-offline-target-" + [Guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Force -Path $temporaryTarget | Out-Null
            $env:CARGO_TARGET_DIR = $temporaryTarget
        }
        Push-Location $Root
        try {
            return Invoke-Captured -File $Cargo -Arguments $CargoArguments
        } finally {
            Pop-Location
        }
    } finally {
        $env:CARGO_HOME = $oldCargoHome
        $env:CARGO_NET_OFFLINE = $oldOffline
        $env:CARGO_TARGET_DIR = $oldTarget
        if (Test-Path -LiteralPath $cargoHomePath) {
            Remove-Item -LiteralPath $cargoHomePath -Recurse -Force
        }
        if ($null -ne $temporaryTarget -and (Test-Path -LiteralPath $temporaryTarget)) {
            Remove-Item -LiteralPath $temporaryTarget -Recurse -Force
        }
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ManifestPath)) {
    $ManifestPath = Join-Path $repoRoot 'build-support\nova-build-lock.psd1'
} elseif (-not [System.IO.Path]::IsPathRooted($ManifestPath)) {
    $ManifestPath = Join-Path $repoRoot $ManifestPath
}

if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
    throw "Pinned manifest is missing: $ManifestPath"
}
$manifest = Import-PowerShellDataFile -LiteralPath $ManifestPath
if ($manifest.SchemaVersion -ne 1) {
    throw "Unsupported manifest schema: $($manifest.SchemaVersion)"
}

$rustup = Find-Program 'rustup'
$cargo = Find-Program 'cargo'

if ($Mode -eq 'Bootstrap') {
    if ($null -eq $rustup) {
        throw 'rustup is required for Bootstrap mode.'
    }
    $installArgs = @('toolchain', 'install', $manifest.Rust.Channel, '--profile', 'minimal')
    foreach ($component in $manifest.Rust.Components) {
        $installArgs += @('--component', $component)
    }
    foreach ($target in $manifest.Rust.Targets) {
        $installArgs += @('--target', $target)
    }
    & $rustup @installArgs
    if ($LASTEXITCODE -ne 0) {
        throw 'Pinned Rust toolchain installation failed.'
    }
    if ($null -eq $cargo) {
        $cargo = Find-Program 'cargo'
    }
    if ($null -eq $cargo) {
        throw 'cargo was not found after Rust bootstrap.'
    }
    Push-Location $repoRoot
    try {
        & $cargo fetch --locked
        if ($LASTEXITCODE -ne 0) {
            throw 'cargo fetch --locked failed.'
        }
    } finally {
        Pop-Location
    }
}

foreach ($entryName in @('CargoLock', 'RustToolchain', 'CargoConfig')) {
    Test-PinnedFile $repoRoot $manifest.SourceInputs[$entryName] $entryName
}

if ($null -eq $rustup) {
    Write-Failure 'rustup was not found.'
} else {
    $rustc = Invoke-Captured -File $rustup -Arguments @('run', $manifest.Rust.Toolchain, 'rustc', '-Vv')
    if ($rustc.ExitCode -ne 0) {
        Write-Failure "Pinned rustc is unavailable: $($manifest.Rust.Toolchain)"
    } elseif ($rustc.Text -notmatch [regex]::Escape("release: $($manifest.Rust.Rustc)") -or
              $rustc.Text -notmatch [regex]::Escape("commit-hash: $($manifest.Rust.RustcCommit)") -or
              $rustc.Text -notmatch [regex]::Escape("host: $($manifest.Rust.Host)") -or
              $rustc.Text -notmatch [regex]::Escape("LLVM version: $($manifest.Rust.LLVM)")) {
        Write-Failure "rustc identity does not match the pinned manifest.`n$($rustc.Text)"
    } else {
        Write-Pass "rustc $($manifest.Rust.Rustc) commit $($manifest.Rust.RustcCommit)"
    }

    $cargoVersion = Invoke-Captured -File $rustup -Arguments @('run', $manifest.Rust.Toolchain, 'cargo', '-V')
    if ($cargoVersion.ExitCode -ne 0 -or $cargoVersion.Text -notmatch [regex]::Escape("cargo $($manifest.Rust.Cargo)")) {
        Write-Failure "cargo identity does not match: $($cargoVersion.Text)"
    } else {
        Write-Pass $cargoVersion.Text
    }

    $components = Invoke-Captured -File $rustup -Arguments @('component', 'list', '--installed', '--toolchain', $manifest.Rust.Toolchain)
    $componentLines = @($components.Text -split "`r?`n" | ForEach-Object { $_.Trim() })
    foreach ($component in $manifest.Rust.Components) {
        $installed = @($componentLines | Where-Object { $_ -eq $component -or $_.StartsWith("$component-") }).Count -ne 0
        if (-not $installed) {
            Write-Failure "Rust component is missing: $component"
        } else {
            Write-Pass "Rust component installed: $component"
        }
    }
    $targets = Invoke-Captured -File $rustup -Arguments @('target', 'list', '--installed', '--toolchain', $manifest.Rust.Toolchain)
    $targetLines = @($targets.Text -split "`r?`n" | ForEach-Object { $_.Trim() })
    foreach ($target in $manifest.Rust.Targets) {
        if ($targetLines -notcontains $target) {
            Write-Failure "Rust target is missing: $target"
        } else {
            Write-Pass "Rust target installed: $target"
        }
    }
}

if ($null -eq $cargo) {
    Write-Failure 'cargo was not found.'
} else {
    $metadataArgs = @('metadata', '--locked', '--offline', '--no-deps', '--format-version', '1')
    $vendorPath = Join-Path $repoRoot $manifest.Offline.VendorDirectory
    $vendorReady = Test-Path -LiteralPath $vendorPath -PathType Container
    if (-not $vendorReady) {
        Write-Failure "Committed vendor directory is missing: $($manifest.Offline.VendorDirectory)"
    } else {
        $vendorDirectories = @(Get-ChildItem -LiteralPath $vendorPath -Directory).Count
        Write-Pass "Committed vendor directory is present with $vendorDirectories crate directories."
    }
    if ($vendorReady -and $script:Failures -eq 0) {
        if ($Offline -or $Build) {
            $metadata = Invoke-WithIsolatedCargoHome $cargo $repoRoot $manifest.Offline $metadataArgs
        } else {
            Push-Location $repoRoot
            try {
                $metadata = Invoke-Captured -File $cargo -Arguments $metadataArgs
            } finally {
                Pop-Location
            }
        }
        if ($metadata.ExitCode -ne 0) {
            Write-Failure "Cargo.lock cannot resolve offline with the selected source policy.`n$($metadata.Text)"
        } else {
            Write-Pass 'Cargo metadata resolves with --locked --offline.'
        }
    }
    if ($Build -and $vendorReady -and $script:Failures -eq 0) {
        Write-Host 'INFO  Running root cargo build with isolated offline CARGO_HOME.'
        $buildResult = Invoke-WithIsolatedCargoHome -Cargo $cargo -Root $repoRoot -OfflinePolicy $manifest.Offline -CargoArguments @('build', '--locked', '--offline') -UseTemporaryTarget $false
        if ($buildResult.ExitCode -ne 0) {
            Write-Failure "Vendored root build failed.`n$($buildResult.Text)"
        } else {
            Write-Pass 'Root cargo build and nested bootloader Cargo stages completed offline.'
        }
    }
}

$qemuExe = Join-Path $repoRoot $manifest.Qemu.Executable
if (-not (Test-Path -LiteralPath $qemuExe -PathType Leaf)) {
    if ($RequireQemu) {
        Write-Failure "Pinned QEMU bundle is missing: $($manifest.Qemu.Executable)"
    } else {
        Write-WarningResult 'QEMU is not part of a source clone; provision the pinned external bundle before runtime gates.'
    }
} else {
    Test-PinnedFile $repoRoot @{ Path = $manifest.Qemu.Executable; Sha256 = $manifest.Qemu.ExecutableSha256 } 'QEMU executable'
    Test-PinnedFile $repoRoot @{ Path = $manifest.Qemu.VersionFile; Sha256 = $manifest.Qemu.VersionFileSha256 } 'QEMU version file'
    $qemuVersion = Invoke-Captured -File $qemuExe -Arguments @('--version')
    if ($qemuVersion.ExitCode -ne 0 -or $qemuVersion.Text -notmatch [regex]::Escape($manifest.Qemu.VersionBanner)) {
        Write-Failure "QEMU version mismatch: $($qemuVersion.Text)"
    } else {
        Write-Pass $manifest.Qemu.VersionBanner
    }
    foreach ($firmware in $manifest.Qemu.Firmware) {
        Test-PinnedFile $repoRoot $firmware 'QEMU firmware'
    }
}

if (-not $manifest.OvmfRuntime.Reproducible) {
    Write-WarningResult $manifest.OvmfRuntime.Note
}

Write-Host "Summary: failures=$script:Failures warnings=$script:Warnings"
if ($script:Failures -ne 0) {
    exit 1
}
exit 0
