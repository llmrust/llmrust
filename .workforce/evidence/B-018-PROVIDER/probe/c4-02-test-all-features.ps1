param([Parameter(Mandatory=$true)][string]$EnvironmentEvidencePath)
$ErrorActionPreference = 'Stop'
$before = $env:CARGO_INCREMENTAL
$during = $null
$code = 1
try {
    $env:CARGO_INCREMENTAL = '0'
    $during = $env:CARGO_INCREMENTAL
    & 'cargo' 'test' '--locked' '--workspace' '--all-features'
    $code = $LASTEXITCODE
} finally {
    $env:CARGO_INCREMENTAL = $before
    $after = $env:CARGO_INCREMENTAL
    [ordered]@{
        variable = 'CARGO_INCREMENTAL'
        before = $before
        during = $during
        after = $after
        restored = ($after -ceq $before)
    } | ConvertTo-Json -Compress | Set-Content -LiteralPath $EnvironmentEvidencePath -Encoding utf8
}
exit $code
