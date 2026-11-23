[CmdletBinding()]
param(
    [string] $OutputDirectory
)

$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repo 'dist/npm'
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null

$package = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'package.json') | ConvertFrom-Json
$toml = Get-Content -Raw -LiteralPath (Join-Path $repo 'Cargo.toml')
if ($toml -notmatch '(?m)^version\s*=\s*"([^"]+)"' -or $Matches[1] -ne $package.version) {
    throw 'Cargo.toml and npm package versions differ.'
}

$targetTriple = 'x86_64-pc-windows-msvc'
$nativeFlags = 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS'
$previousFlags = [Environment]::GetEnvironmentVariable($nativeFlags, 'Process')
Push-Location -LiteralPath $repo
try {
    # Link the C runtime into the packaged Windows binary.
    [Environment]::SetEnvironmentVariable($nativeFlags,
        ("$previousFlags -C target-feature=+crt-static").Trim(), 'Process')
    cargo build --locked --release --target $targetTriple
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed with exit code $LASTEXITCODE." }
    $metadata = cargo metadata --locked --format-version 1 --filter-platform $targetTriple | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Could not read licenses from Cargo metadata.' }
    $sysroot = rustc --print sysroot
    if ($LASTEXITCODE -ne 0) { throw 'Could not locate the Rust toolchain notices.' }
}
finally {
    [Environment]::SetEnvironmentVariable($nativeFlags, $previousFlags, 'Process')
    Pop-Location
}

$binary = Join-Path $repo "target/$targetTriple/release/visioncache-mcp.exe"
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw 'The release executable was not found under the workspace target directory.'
}

$staging = Join-Path $repo 'target/npm/win32-x64'
[IO.Directory]::CreateDirectory((Join-Path $staging 'native')) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $staging 'skill')) | Out-Null
foreach ($file in @('package.json', 'cli.cjs', 'README.md')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $file) -Destination (Join-Path $staging $file)
}
Copy-Item -LiteralPath (Join-Path $repo 'INSTALL.md') -Destination (Join-Path $staging 'INSTALL.md')
Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination (Join-Path $staging 'LICENSE')
Copy-Item -LiteralPath (Join-Path $repo 'integrations/codex/visioncache-desktop/SKILL.md') -Destination (Join-Path $staging 'skill/SKILL.md')
Copy-Item -LiteralPath $binary -Destination (Join-Path $staging 'native/visioncache-mcp.exe')
$licenseDirectory = Join-Path $staging 'licenses'
[IO.Directory]::CreateDirectory($licenseDirectory) | Out-Null
$notices = [Text.StringBuilder]::new()
[void] $notices.AppendLine('Third-party notices for the packaged VisionCache Windows binary')
[void] $notices.AppendLine()
$documents = [ordered]@{}
foreach ($dependency in ($metadata.packages | Where-Object { $_.source } | Sort-Object name, version)) {
    [void] $notices.AppendLine(("{0} {1} - {2}" -f $dependency.name, $dependency.version, $dependency.license))
    $directory = Split-Path -Parent $dependency.manifest_path
    $files = @(Get-ChildItem -LiteralPath $directory -File |
        Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE|COPYRIGHT|UNLICENSE)([._ -]|$)' } |
        Sort-Object Name)
    if ($files.Count -eq 0) {
        throw "License text was not found for $($dependency.name) $($dependency.version)."
    }
    foreach ($file in $files) {
        $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        if (-not $documents.Contains($digest)) {
            $documents[$digest] = @{ ID = $documents.Count + 1; Path = $file.FullName }
        }
        [void] $notices.AppendLine(("  {0} - document {1}" -f $file.Name, $documents[$digest].ID))
    }
}
foreach ($document in $documents.Values) {
    [void] $notices.AppendLine()
    [void] $notices.AppendLine(("Document {0}" -f $document.ID))
    [void] $notices.AppendLine([IO.File]::ReadAllText($document.Path))
}
[IO.File]::WriteAllText((Join-Path $licenseDirectory 'Cargo-THIRD-PARTY-NOTICES.txt'), $notices.ToString())
$rustNotice = Join-Path $sysroot 'share/doc/rust/COPYRIGHT-library.html'
if (-not (Test-Path -LiteralPath $rustNotice -PathType Leaf)) {
    throw 'Install the rust-docs component to include the Rust standard library notices.'
}
Copy-Item -LiteralPath $rustNotice -Destination (Join-Path $licenseDirectory 'Rust-COPYRIGHT-library.html')
$npm = (Get-Command npm.cmd).Source
Push-Location -LiteralPath $staging
try {
    & $npm pack --ignore-scripts --json --pack-destination $OutputDirectory
    if ($LASTEXITCODE -ne 0) { throw "npm pack failed with exit code $LASTEXITCODE." }
}
finally {
    Pop-Location
}
