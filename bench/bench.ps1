# Benchmark Impulse: costo en CPU del estilo durmiente vs polling, y throughput.
# Uso:  powershell -File bench\bench.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$exe  = "C:\Users\frang\AppData\Local\impulse-target\x86_64-pc-windows-gnu\release\impulsec.exe"

Write-Host "Compilando en release..."
cargo +stable-x86_64-pc-windows-gnu build -q --release --manifest-path "$root\compiler\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw "fallo la compilacion" }

# Mide el % de CPU de un programa que queda vivo, durante $seconds tras $warmup.
function Measure-IdleCpu($file, $seconds, $warmup) {
    $out = Join-Path $env:TEMP "imp_bench_out.txt"
    $err = Join-Path $env:TEMP "imp_bench_err.txt"
    $p = Start-Process -FilePath $exe -ArgumentList "`"$file`"" -PassThru -NoNewWindow `
        -RedirectStandardOutput $out -RedirectStandardError $err
    Start-Sleep -Seconds $warmup
    $p.Refresh(); $t1 = $p.TotalProcessorTime
    Start-Sleep -Seconds $seconds
    $p.Refresh(); $t2 = $p.TotalProcessorTime
    if ($p.HasExited) { throw "el proceso de $file murio durante la medicion (salida: $(Get-Content $err -Raw))" }
    Stop-Process -Id $p.Id -Force
    $cpuSec = ($t2 - $t1).TotalSeconds
    [math]::Round(100 * $cpuSec / $seconds, 2)
}

Write-Host "`n--- Throughput: 100.000 senales ---"
& $exe "$root\bench\throughput.imp" | Out-Null   # calentamiento (arranque + cache)
$t = Measure-Command { & $exe "$root\bench\throughput.imp" | Out-Host }
$sigPerSec = [math]::Round(100000 / $t.TotalSeconds)
Write-Host ("tiempo: {0:n2}s  ->  {1:n0} senales/seg" -f $t.TotalSeconds, $sigPerSec)

Write-Host "`n--- CPU en reposo (10s de medicion, 2s de calentamiento) ---"
$dormant = Measure-IdleCpu "$root\bench\dormant.imp" 10 2
Write-Host ("dormant.imp (reactivo durmiente): {0}% de un nucleo" -f $dormant)

$polling = Measure-IdleCpu "$root\bench\polling.imp" 10 2
Write-Host ("polling.imp (bucle de sondeo):    {0}% de un nucleo" -f $polling)

Write-Host "`n=== Resumen ==="
Write-Host ("Mismo lenguaje, mismo trabajo. Esperando eventos:")
Write-Host ("  estilo Impulse (senales):  {0}% CPU" -f $dormant)
Write-Host ("  estilo polling:            {0}% CPU" -f $polling)
