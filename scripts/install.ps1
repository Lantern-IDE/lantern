# Lantern CLI installer for Windows. No admin rights needed.
#
#   Invoke-WebRequest https://raw.githubusercontent.com/Lantern-IDE/lantern/main/scripts/install.ps1 -OutFile install.ps1
#   powershell -ExecutionPolicy Bypass -File .\install.ps1
#
# Finds the newest cli-v* release through the GitHub API ("latest" belongs to the desktop app),
# downloads the archive and SHA256SUMS to a temp folder, verifies the checksum, and only then installs.
#
# Environment variables:
#   LANTERN_VERSION       install this version (0.1.0 or cli-v0.1.0) and skip the API lookup
#   LANTERN_INSTALL_DIR   install folder (default: %LOCALAPPDATA%\lantern-cli\bin)
#   LANTERN_INSTALL_BASE  https:// URL or local folder to use instead of GitHub. It must hold
#                         releases.json and download/<tag>/<file>. For mirrors and tests.
#
# Keep this file ASCII: Windows PowerShell 5.1 reads a script without a BOM in the system code page.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # the progress bar slows downloads badly on PowerShell 5.1

$LanternRepo = 'Lantern-IDE/lantern'

function Save-Source {
    param([string]$Source, [string]$Dest)
    if ($Source -match '^https://') {
        for ($attempt = 1; ; $attempt++) {
            try {
                Invoke-WebRequest -Uri $Source -OutFile $Dest -UseBasicParsing
                return
            } catch {
                # Retry dropped connections and server errors. A 404 will not change.
                $response = $_.Exception.Response
                $status = if ($response) { [int]$response.StatusCode } else { 0 }
                if ($attempt -ge 3 -or ($status -ge 400 -and $status -lt 500)) {
                    throw "Could not download ${Source}: $($_.Exception.Message)"
                }
                Start-Sleep -Seconds 2
            }
        }
    } elseif ($Source -match '^[A-Za-z][A-Za-z0-9+.-]+://') {
        throw "Refusing to download from ${Source}: only https:// or a local folder is allowed."
    } else {
        if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
            throw "Could not read ${Source}: file not found."
        }
        Copy-Item -LiteralPath $Source -Destination $Dest
    }
}

function Get-NewestTag {
    param([string]$ListSource, [string]$TempDir)
    $file = Join-Path $TempDir 'releases.json'
    try {
        Save-Source $ListSource $file
    } catch {
        if ($ListSource -notmatch '^https://api\.github\.com/') { throw }
        throw "$($_.Exception.Message)`nThe GitHub API allows 60 anonymous requests per hour. If you hit that limit, wait or set LANTERN_VERSION to skip this lookup."
    }
    # Assign first: on PowerShell 5.1 ConvertFrom-Json sends an array down the pipeline as one object.
    $releases = [IO.File]::ReadAllText($file, [Text.Encoding]::UTF8) | ConvertFrom-Json
    $stable = @($releases | Where-Object { -not $_.draft -and $_.tag_name -cmatch '^cli-v\d+\.\d+\.\d+$' })
    if ($stable.Count -eq 0) {
        throw "No published cli-v* release found at $ListSource."
    }
    ($stable | Sort-Object { [version]($_.tag_name -replace '^cli-v', '') } -Descending | Select-Object -First 1).tag_name
}

function Get-Targets {
    $arch = $null
    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    }
    switch -Regex ($arch) {
        '^(X64|AMD64)$' { return @('x86_64-pc-windows-msvc') }
        # Windows on ARM also runs the x64 build, so it is the fallback.
        '^ARM64$' { return @('aarch64-pc-windows-msvc', 'x86_64-pc-windows-msvc') }
        default { throw "No Lantern CLI build for this architecture: $arch." }
    }
}

function Read-Sums {
    param([string]$File)
    $sums = @{}
    foreach ($line in Get-Content -LiteralPath $File) {
        if ($line -match '^([0-9a-fA-F]{64})\s+\*?(\S+)\s*$') {
            $sums[$Matches[2]] = $Matches[1].ToLowerInvariant()
        }
    }
    $sums
}

function Install-Lantern {
    $base = if ($env:LANTERN_INSTALL_BASE) { $env:LANTERN_INSTALL_BASE.TrimEnd('/', '\') } else { $null }
    $listSource = if ($base) { "$base/releases.json" } else { "https://api.github.com/repos/$LanternRepo/releases?per_page=100" }
    $assetBase = if ($base) { "$base/download" } else { "https://github.com/$LanternRepo/releases/download" }

    $dir = if ($env:LANTERN_INSTALL_DIR) { $env:LANTERN_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'lantern-cli\bin' }
    $temp = Join-Path ([IO.Path]::GetTempPath()) ("lantern-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temp | Out-Null

    try {
        if ($env:LANTERN_VERSION) {
            $tag = 'cli-v' + ($env:LANTERN_VERSION -replace '^(cli-)?v', '')
            if ($tag -cnotmatch '^cli-v\d+\.\d+\.\d+$') {
                throw "LANTERN_VERSION must look like 0.1.0, got '$($env:LANTERN_VERSION)'."
            }
        } else {
            $tag = Get-NewestTag $listSource $temp
        }
        $version = $tag -replace '^cli-v', ''
        Write-Host "Installing Lantern CLI $version"

        $sumsFile = Join-Path $temp 'SHA256SUMS'
        Save-Source "$assetBase/$tag/SHA256SUMS" $sumsFile
        $sums = Read-Sums $sumsFile

        $name = $null
        foreach ($target in Get-Targets) {
            if ($sums.ContainsKey("lantern-$target.zip")) {
                $name = "lantern-$target.zip"
                break
            }
        }
        if (-not $name) {
            throw "Release $tag has no build for this PC (looked for: $((Get-Targets) -join ', '))."
        }

        $archive = Join-Path $temp $name
        Save-Source "$assetBase/$tag/$name" $archive
        $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -cne $sums[$name]) {
            throw "Checksum mismatch for ${name}.`n  SHA256SUMS: $($sums[$name])`n  downloaded: $actual`nNothing was installed. Try again, and report it if this keeps happening."
        }
        Write-Host "Checksum OK ($name)"

        $unpacked = Join-Path $temp 'unpacked'
        Expand-Archive -LiteralPath $archive -DestinationPath $unpacked
        $exe = Join-Path $unpacked 'lantern.exe'
        if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
            throw "$name does not contain lantern.exe."
        }

        # Run the new binary before touching an existing install.
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        $final = Join-Path $dir 'lantern.exe'
        $staged = Join-Path $dir 'lantern.new.exe'
        Copy-Item -LiteralPath $exe -Destination $staged -Force
        try {
            $got = & $staged --version
            if ($LASTEXITCODE -ne 0 -or $got -cne "lantern $version") {
                throw "The downloaded binary reported '$got' (exit code $LASTEXITCODE), expected 'lantern $version'."
            }
            try {
                Move-Item -LiteralPath $staged -Destination $final -Force
            } catch {
                throw "Could not replace ${final}. Is lantern still running (an MCP server, for example)? Close it and run this again."
            }
        } finally {
            Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue
        }
    } finally {
        Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host "Installed $final"

    $onPath = @($env:Path -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ieq $dir.TrimEnd('\') }).Count -gt 0
    if ($onPath) {
        $found = Get-Command lantern -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($found -and $found.Source -ine $final) {
            Write-Host "Note: 'lantern' currently runs $($found.Source), which comes earlier on your PATH."
        }
    } else {
        Write-Host ""
        Write-Host "$dir is not on your PATH. To add it for your user, run this and open a new terminal:"
        Write-Host "  [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$dir', 'User')"
    }
}

try {
    # 0 is SystemDefault, which already allows TLS 1.2 and newer. Older .NET defaults to TLS 1.0, which GitHub refuses.
    $protocol = [Net.ServicePointManager]::SecurityProtocol
    if ([int]$protocol -ne 0) {
        [Net.ServicePointManager]::SecurityProtocol = $protocol -bor [Net.SecurityProtocolType]::Tls12
    }
} catch {
    # PowerShell 7 negotiates TLS by itself.
}

try {
    Install-Lantern
} catch {
    [Console]::Error.WriteLine("Lantern install failed: $($_.Exception.Message)")
    # A script file exits; pasted into a session, keep the session open.
    if ($MyInvocation.MyCommand.Path) { exit 1 }
    throw
}
