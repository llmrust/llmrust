$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = 'D:\llmrust-0.1.5'
$probe = Join-Path $repo '.workforce\evidence\B-AUD-3\probe'
$jsonlPath = Join-Path $probe 'runs.jsonl'
$cargo = @(Get-Command cargo -CommandType Application | Sort-Object Source)[0].Source
$specSha256 = 'f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7'
$handoffId = '79b1a1c78436c5d212c854a3'

$base = @(Get-Content -LiteralPath $jsonlPath | ForEach-Object { $_ | ConvertFrom-Json })
$baseIds = @('required-01-build','required-02-test-default','required-03-test-all-features','required-04-fmt','required-05-clippy','required-06-rustdoc','required-07-cargo-deny','c4-01-test-default')
if ($base.Count -ne 8 -or @($base.exit_code | Where-Object { $_ -ne 0 }).Count -ne 0 -or (($base.id -join ',') -ne ($baseIds -join ','))) { throw 'Base runs.jsonl is not the expected 8 all-zero records' }

$commands = @(
    [ordered]@{ id='c4-02-test-proxy'; criterion='C-AUD-PROXY.c4[1]'; cmd='cargo test --workspace --features proxy'; args=@('test','--workspace','--features','proxy') },
    [ordered]@{ id='c4-03-build-default'; criterion='C-AUD-PROXY.c4[2]'; cmd='cargo build --workspace'; args=@('build','--workspace') },
    [ordered]@{ id='c4-04-fmt'; criterion='C-AUD-PROXY.c4[3]'; cmd='cargo fmt --all --check'; args=@('fmt','--all','--check') },
    [ordered]@{ id='c4-05-clippy-default'; criterion='C-AUD-PROXY.c4[4]'; cmd='cargo clippy --workspace --all-targets -- -D warnings'; args=@('clippy','--workspace','--all-targets','--','-D','warnings') },
    [ordered]@{ id='c4-06-cargo-deny'; criterion='C-AUD-PROXY.c4[5]'; cmd='cargo deny --locked check'; args=@('deny','--locked','check') },
    [ordered]@{ id='c5-01-architecture-guard'; criterion='C-AUD-PROXY.c5[0]'; cmd='cargo test --features proxy --test architecture_guard'; args=@('test','--features','proxy','--test','architecture_guard') },
    [ordered]@{ id='c5-02-package-guard'; criterion='C-AUD-PROXY.c5[1]'; cmd='cargo test --features proxy --test package_guard'; args=@('test','--features','proxy','--test','package_guard') },
    [ordered]@{ id='c5-03-responses-proxy-tests'; criterion='C-AUD-PROXY.c5[2]'; cmd='cargo test --features proxy --test responses_proxy_tests'; args=@('test','--features','proxy','--test','responses_proxy_tests') },
    [ordered]@{ id='c5-04-proxy-lib'; criterion='C-AUD-PROXY.c5[3]'; cmd='cargo test --features proxy --lib'; args=@('test','--features','proxy','--lib') },
    [ordered]@{ id='c5-05-contract-tests'; criterion='C-AUD-PROXY.c5[4]'; cmd='cargo test --features proxy --test contract_tests'; args=@('test','--features','proxy','--test','contract_tests') }
)

$index = 0
$failed = 0
foreach ($task in $commands) {
    $index++
    $stdoutPath = Join-Path $probe ($task.id + '.stdout.log')
    $stderrPath = Join-Path $probe ($task.id + '.stderr.log')
    if ($task.id -ne 'c4-02-test-proxy' -and ((Test-Path -LiteralPath $stdoutPath) -or (Test-Path -LiteralPath $stderrPath))) { throw "Refusing to overwrite existing output for $($task.id)" }

    $started = [DateTime]::UtcNow.ToString('o')
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $cargo
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    foreach ($arg in $task.args) { $psi.ArgumentList.Add($arg) }

    $process = [System.Diagnostics.Process]::Start($psi)
    $outStream = [System.IO.File]::Open($stdoutPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read)
    $errStream = [System.IO.File]::Open($stderrPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read)
    try {
        $outCopy = $process.StandardOutput.BaseStream.CopyToAsync($outStream)
        $errCopy = $process.StandardError.BaseStream.CopyToAsync($errStream)
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        [void]$outCopy.GetAwaiter().GetResult()
        [void]$errCopy.GetAwaiter().GetResult()
    }
    finally {
        $outStream.Dispose()
        $errStream.Dispose()
        $process.Dispose()
    }
    $finished = [DateTime]::UtcNow.ToString('o')
    if ($exitCode -ne 0) { $failed++ }

    $record = [ordered]@{
        id=$task.id
        criterion=$task.criterion
        cmd=$task.cmd
        exit_code=$exitCode
        started_at_utc=$started
        finished_at_utc=$finished
        stdout=[ordered]@{ path='.workforce/evidence/B-AUD-3/probe/' + (Split-Path -Leaf $stdoutPath); sha256=(Get-FileHash -LiteralPath $stdoutPath -Algorithm SHA256).Hash.ToLowerInvariant(); bytes=(Get-Item -LiteralPath $stdoutPath).Length }
        stderr=[ordered]@{ path='.workforce/evidence/B-AUD-3/probe/' + (Split-Path -Leaf $stderrPath); sha256=(Get-FileHash -LiteralPath $stderrPath -Algorithm SHA256).Hash.ToLowerInvariant(); bytes=(Get-Item -LiteralPath $stderrPath).Length }
    }
    $record | ConvertTo-Json -Compress -Depth 8 | Add-Content -LiteralPath $jsonlPath -Encoding utf8NoBOM
    Write-Output ("[{0}/{1}] exit={2} {3}" -f $index, $commands.Count, $exitCode, $task.cmd)
}

$all = @(Get-Content -LiteralPath $jsonlPath | ForEach-Object { $_ | ConvertFrom-Json })
if ($all.Count -ne 18) { throw "Expected 18 records after rerun, got $($all.Count)" }
Write-Output "completed_missing=$($commands.Count) total_records=$($all.Count) nonzero=$failed"
if ($failed -gt 0) { exit 1 }
