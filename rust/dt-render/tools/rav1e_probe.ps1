# Run from any directory; all generated manifests, lockfiles and output stay under rust/target.
param([string]$MeasureDirectory)
$ErrorActionPreference = 'Stop'
$crateDirectory = Split-Path -Parent $PSScriptRoot
$rustDirectory = Split-Path -Parent $crateDirectory
$probeDirectory = Join-Path $rustDirectory 'target/rav1e-probe'
if (-not $MeasureDirectory) { $MeasureDirectory = Join-Path $rustDirectory 'target/render-measure' }
New-Item -ItemType Directory -Force -Path $probeDirectory | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'rav1e-probe.toml') -Destination (Join-Path $probeDirectory 'Cargo.toml')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'rav1e_probe.rs') -Destination (Join-Path $probeDirectory 'main.rs')
$ErrorActionPreference = 'Continue'
& ffmpeg -v error -f rawvideo -pixel_format rgb24 -video_size 640x360 -i (Join-Path $MeasureDirectory 'input.rgb') -frames:v 8 -vf 'scale=out_range=full' -pix_fmt yuv420p -f rawvideo -y (Join-Path $MeasureDirectory 'input.yuv')
if ($LASTEXITCODE -ne 0) { throw 'Could not generate benchmark YUV input' }
$env:CARGO_BUILD_JOBS = '4'
& cargo run --manifest-path (Join-Path $probeDirectory 'Cargo.toml') --target-dir (Join-Path $probeDirectory 'build') --release -- $MeasureDirectory
exit $LASTEXITCODE
