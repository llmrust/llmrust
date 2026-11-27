$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = 'D:\llmrust-0.1.5'
$probe = Join-Path $repo '.workforce\evidence\B-AUD-3\probe'
$manifestPath = Join-Path $probe 'run-manifest.json'
$jsonlPath = Join-Path $probe 'runs.jsonl'
$cargo = @(Get-Command cargo -CommandType Application | Sort-Object Source)[0].Source
$specSha256 = 'f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7'
$handoffId = '79b1a1c78436c5d212c854a3'

if (Test-Path -LiteralPath $manifestPath) { throw "Refusing to overwrite existing $manifestPath" }
if (Test-Path -LiteralPath $jsonlPath) { throw "Refusing to overwrite existing $jsonlPath" }

$commands = @(
    [ordered]@{ id='required-01-build'; criterion='config.gates.required_commands[0]'; cmd='cargo build --locked --workspace --all-targets --all-features'; args=@('build','--locked','--workspace','--all-targets','--all-features') },
    [ordered]@{ id='required-02-test-default'; criterion='config.gates.required_commands[1]'; cmd='cargo test --locked --workspace'; args=@('test','--locked','--workspace') },
    [ordered]@{ id='required-03-test-all-features'; criterion='config.gates.required_commands[2]'; cmd='cargo test --locked --workspace --all-features'; args=@('test','--locked','--workspace','--all-features') },
    [ordered]@{ id='required-04-fmt'; criterion='config.gates.required_commands[3]'; cmd='cargo fmt --all --check'; args=@('fmt','--all','--check') },
    [ordered]@{ id='required-05-clippy'; criterion='config.gates.required_commands[4]'; cmd='cargo clippy --locked --all-targets --all-features -- -D warnings'; args=@('clippy','--locked','--all-targets','--all-features','--','-D','warnings') },
    [ordered]@{ id='required-06-rustdoc'; criterion='config.gates.required_commands[5]'; cmd='cargo rustdoc --locked --all-features -- -D warnings'; args=@('rustdoc','--locked','--all-features','--','-D','warnings') },
    [ordered]@{ id='required-07-cargo-deny'; criterion='config.gates.required_commands[6]'; cmd='cargo deny --locked check'; args=@('deny','--locked','check') },
    [ordered]@{ id='c4-01-test-default'; criterion='C-AUD-PROXY.c4[0]'; cmd='cargo test --workspace'; args=@('test','--workspace') },
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

$records = [System.Collections.Generic.List[object]]::new()
$index = 0
foreach ($task in $commands) {
    $index++
    $stdoutPath = Join-Path $probe ($task.id + '.stdout.log')
    $stderrPath = Join-Path $probe ($task.id + '.stderr.log')
    if ((Test-Path -LiteralPath $stdoutPath) -or (Test-Path -LiteralPath $stderrPath)) { throw "Refusing to overwrite raw output for $($task.id)" }

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
    $records.Add($record)
    $record | ConvertTo-Json -Compress -Depth 8 | Add-Content -LiteralPath $jsonlPath -Encoding utf8NoBOM
    Write-Output ("[{0}/{1}] exit={2} {3}" -f $index, $commands.Count, $exitCode, $task.cmd)
}

$manifest = [ordered]@{
    schema='gate-command-runs/v1'
    role='gate'
    agent='gate-llmrust'
    written_by='gate-llmrust'
    batch_id='B-AUD-3'
    card_id='C-AUD-PROXY'
    spec_sha256=$specSha256
    handoff_id=$handoffId
    cargo=$cargo
    runs=@($records)
}
$manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath -Encoding utf8NoBOM
$failed = @($records | Where-Object { $_.exit_code -ne 0 })
Write-Output "completed=$($records.Count) nonzero=$($failed.Count)"
Write-Output "manifest=$manifestPath"
if ($failed.Count -gt 0) { exit 1 }
