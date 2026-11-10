param([Parameter(Mandatory=$true)][string]$RunDirectory)
$ErrorActionPreference = 'Stop'
$runPath = (Resolve-Path -LiteralPath $RunDirectory).Path
$networkPath = Join-Path $runPath 'network'
$cases = Get-Content -LiteralPath (Join-Path $networkPath 'summary.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$environment = Get-Content -LiteralPath (Join-Path $runPath 'environment.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$manifest = Get-Content -LiteralPath (Join-Path $networkPath 'manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$rows = @()
foreach ($case in $cases) {
    $report = Get-Content -LiteralPath (Join-Path $networkPath ($case.case + '.json')) -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($report.status -eq 'HARNESS_ERROR') { $rows += [pscustomobject][ordered]@{case=$case.case;status=$report.status;error=$report.error}; continue }
    $cpuWindows = @()
    for ($i=1; $i -lt $report.resources.Count; $i++) {
        $duration = $report.resources[$i].elapsed_ns - $report.resources[$i-1].elapsed_ns
        if ($duration -gt 0) { $cpuWindows += (($report.resources[$i].sample.cpu_ns - $report.resources[$i-1].sample.cpu_ns) / $duration * 100) }
    }
    $sortedCpu = @($cpuWindows | Sort-Object)
    $interrupted = if ($null -ne $report.environment_interrupted) { [bool]$report.environment_interrupted } else { $report.elapsed_ns -gt (($report.options.events / $report.options.rate_hz + 6) * 1e9) }
    $p95Cpu = if ($sortedCpu.Count) { $sortedCpu[[math]::Ceiling($sortedCpu.Count * 0.95)-1] } else { $null }
    $rows += [pscustomobject][ordered]@{
        case = $case.case
        status = $report.status
        environment_interrupted = $interrupted
        interruption_basis = if ($null -ne $report.environment_interrupted) { 'Actor gap >1 second; possible suspension, debugger stop or severe scheduling interruption' } else { 'Inferred only from elapsed time exceeding the entire experiment budget; not a confirmed power-event trace' }
        maximum_actor_gap_ms = if ($null -ne $report.maximum_actor_gap_ns) { $report.maximum_actor_gap_ns / 1e6 } else { $null }
        maintenance_wakes = $report.maintenance_wakes
        timer_resolution_override = $report.timer_resolution_override
        scheduled = $report.options.events
        generated = $report.generated
        applied = $report.applied
        acknowledged = $report.acknowledged
        missing_generated = $report.missing
        not_generated = $report.not_generated
        deadline_deliveries = $report.delivered_before_recovery_deadline
        effective_history_deadline_ms = $report.effective_recovery_deadline_ms
        effective_retry_ms = $report.effective_retry_ms
        setup_ms = if ($null -ne $report.setup_elapsed_ns) { $report.setup_elapsed_ns / 1e6 } else { $null }
        client_udp_datagrams = $report.client_udp_datagrams
        host_udp_datagrams = $report.host_udp_datagrams
        fidelity_errors = $report.fidelity_errors
        cleanup_failures = $report.cleanup_failures
        owned_after_cleanup = $report.owned_after_cleanup
        recorded_latency_count = $report.same_clock_generated_to_recording_submit.count
        complete_correct_recorded_run = (-not $interrupted -and $report.status -eq 'PASS' -and $report.applied -eq $report.options.events -and $report.acknowledged -eq $report.options.events -and $report.same_clock_generated_to_recording_submit.count -eq $report.applied -and $report.fidelity_errors -eq 0 -and $report.cleanup_failures -eq 0 -and $report.owned_after_cleanup -eq 0)
        elapsed_seconds = [math]::Round($report.elapsed_ns / 1e9, 4)
        nominal_hz = $report.options.rate_hz
        achieved_hz = [math]::Round($report.achieved_rate_hz, 2)
        p50_ms = [math]::Round($report.same_clock_generated_to_recording_submit.p50_ns / 1e6, 4)
        p95_ms = [math]::Round($report.same_clock_generated_to_recording_submit.p95_ns / 1e6, 4)
        p99_ms = [math]::Round($report.same_clock_generated_to_recording_submit.p99_ns / 1e6, 4)
        pacing_lateness_p99_ms = [math]::Round($report.generation_lateness.p99_ns / 1e6, 4)
        combined_process_cpu_percent_one_core = $report.process_cpu_percent_one_logical_core
        combined_process_cpu_percent_total_machine = if ($null -ne $report.process_cpu_percent_one_logical_core) { $report.process_cpu_percent_one_logical_core / $environment.logical_processors } else { $null }
        cpu_p95_sample_window_percent_one_core = $p95Cpu
        cpu_window_count = $sortedCpu.Count
        private_peak_mib = if ($null -ne $report.private_memory_peak_bytes) { [math]::Round($report.private_memory_peak_bytes / 1MB, 3) } else { $null }
        ipv4_estimate_mbit_per_second = [math]::Round($report.ipv4_wire_estimate_bits_per_second_both_endpoints / 1e6, 4)
        send_history_peak = $report.send_history_peak
        reorder_peak = $report.reorder_peak
        retries = $report.retries
        stop_reason = $report.stop_reason
    }
}
$loss = @()
foreach ($level in @(50,100,200)) {
    $blocks = @($rows | Where-Object { $_.case -like ('loss-' + $level + '-seed-*') })
    $scheduled = ($blocks | Measure-Object scheduled -Sum).Sum
    $delivered = ($blocks | Measure-Object deadline_deliveries -Sum).Sum
    $fractions = @($blocks | ForEach-Object { $_.deadline_deliveries / $_.scheduled })
    $loss += [ordered]@{
        loss_percent_each_direction = $level / 100
        blocks = $blocks.Count
        scheduled = $scheduled
        delivered_before_selected_deadline = $delivered
        empirical_fraction = if ($scheduled) { $delivered / $scheduled } else { $null }
        block_fractions = $fractions
        interpretation = 'Descriptive synthetic results. Events within a block are correlated; no independent-event confidence guarantee is inferred.'
    }
}
$lifecyclePath = Join-Path $networkPath 'lifecycle.json'
$lifecycle = if (Test-Path -LiteralPath $lifecyclePath) { @(Get-Content -LiteralPath $lifecyclePath -Raw -Encoding UTF8 | ConvertFrom-Json) } else { @() }
$idlePath = Join-Path $runPath 'agent-idle-resources.json'
$idle = if (Test-Path -LiteralPath $idlePath) { Get-Content -LiteralPath $idlePath -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
$result = [ordered]@{
    measurement_class = 'single_pc_recording_analysis'
    run_directory = $runPath
    cases = $rows
    loss_blocks = $loss
    lifecycle_passed = @($lifecycle | Where-Object passed).Count
    lifecycle_total = @($lifecycle).Count
    expected_network_cases = if ($manifest.quick) { 19 } else { 35 }
    network_suite_fully_executed = ($rows.Count -eq $(if ($manifest.quick) { 19 } else { 35 }))
    network_suite_complete = ($rows.Count -eq $(if ($manifest.quick) { 19 } else { 35 }) -and @($rows | Where-Object { -not $_.complete_correct_recorded_run }).Count -eq 0)
    idle_agent = $idle
    target_interpretation = 'Healthy completeness and recorded cleanup are checked. C0/I0, physical gaming, mouse fidelity, per-endpoint active resources and competitor gates remain unverified. Timings from stopped cases exclude missing/ungenerated events and cannot certify latency acceptance.'
}
$result | ConvertTo-Json -Depth 9 | Set-Content -LiteralPath (Join-Path $runPath 'analysis.json') -Encoding UTF8
$rows | ConvertTo-Json -Depth 4
