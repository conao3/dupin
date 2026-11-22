#!/usr/bin/env nu

use ../../scripts/unlink-store.nu

const READY_PREFIX = 'dupin-trace: ready '
const RUNNER = 'printf "%s\\n" "$$" > "$PID_FILE"; exec "$APP_BINARY" >>"$APP_LOG" 2>&1'

def --wrapped checked [...args: string]: nothing -> string {
    let result = run-external ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($args.0) failed:\n($result.stderr)"}
    }
    $result.stdout | str trim
}

def sha256 [file: path]: nothing -> string {
    open --raw $file | hash sha256
}

export def config-text []: nothing -> string {
    {
        hotkey: ''
        auto_update: false
        plugins: {
            calculator: {
                enabled: true
                grants: []
            }
        }
    } | to toml
}

export def variant-order [pair: int]: nothing -> list<string> {
    if ($pair mod 2) == 0 { [native wasm] } else { [wasm native] }
}

export def parse-ready-line [line: string]: nothing -> record {
    let fields = $line | split row ' '
    if ($fields | length) != 4 or ($fields | first) != 'dupin-trace:' or ($fields | get 1) != 'ready' {
        error make {msg: 'invalid Dupin readiness marker'}
    }
    let pid_field = $fields | get 2 | split row '='
    let startup_field = $fields | get 3 | split row '='
    if ($pid_field | length) != 2 or ($startup_field | length) != 2 or ($pid_field | first) != 'pid' or ($startup_field | first) != 'startup_ms' {
        error make {msg: 'invalid Dupin readiness marker fields'}
    }
    {
        pid: ($pid_field | get 1 | into int)
        startup_ms: ($startup_field | get 1 | into float)
    }
}

def ready-marker [log: path]: nothing -> any {
    if not ($log | path exists) {
        return null
    }
    let lines = open --raw $log | lines | where { str starts-with $READY_PREFIX }
    if ($lines | is-empty) { null } else { parse-ready-line ($lines | last) }
}

def wait-ready [log: path, timeout: duration]: nothing -> any {
    let deadline = (date now) + $timeout
    loop {
        let ready = ready-marker $log
        if $ready != null {
            return $ready
        }
        if (date now) > $deadline {
            error make {msg: $"Dupin did not report readiness within ($timeout)"}
        }
        sleep 25ms
    }
}

def prepare-app [source: path, destination: path]: nothing -> record {
    let executable = $source | path join Contents MacOS dupin
    if ($source | path type) != 'dir' or ($executable | path type) != 'file' {
        error make {msg: $"not a Dupin app bundle: ($source)"}
    }
    checked /usr/bin/ditto $source $destination | ignore
    checked chmod -R u+w $destination | ignore
    let binary = $destination | path join Contents MacOS dupin
    unlink-store $binary
    (checked
        /usr/bin/codesign
        --force
        --options
        runtime
        --sign
        -
        $destination
    ) | ignore
    checked /usr/bin/codesign --verify --strict $destination | ignore
    {
        source_app: ($source | path expand --strict)
        source_executable_sha256: (sha256 $executable)
        app: $destination
        executable: ($binary | path expand --strict)
        executable_sha256: (sha256 $binary)
    }
}

def stop-owned [helper: path, pid_file: path, executable: path]: nothing -> nothing {
    if not ($pid_file | path exists) {
        return
    }
    let pid = open --raw $pid_file | str trim
    let check_args = [
        $helper
        '--pid'
        $pid
        ($executable | into string)
    ]
    let checked_pid = run-external ...$check_args | complete
    if $checked_pid.exit_code == 0 {
        let stop_args = [
            $helper
            '--terminate'
            $pid
            ($executable | into string)
            'term'
        ]
        let stopped = run-external ...$stop_args | complete
        if $stopped.exit_code != 0 {
            error make {msg: $stopped.stderr}
        }
        sleep 250ms
        let alive = run-external ...$check_args | complete
        if $alive.exit_code == 0 {
            let kill_args = [
                $helper
                '--terminate'
                $pid
                ($executable | into string)
                'kill'
            ]
            let killed = run-external ...$kill_args | complete
            if $killed.exit_code != 0 {
                error make {msg: $killed.stderr}
            }
        }
    }
}

def one-run [
    helper: path
    executable: path
    label: string
    pair: int
    position: int
    work: path
    sample_seconds: list<int>
    ready_timeout: duration
]: nothing -> record {
    let run_dir = $work | path join $"pair-($pair + 1)-($label)"
    let task_home = $run_dir | path join home
    let config_root = $run_dir | path join config
    let data_root = $run_dir | path join data
    let config_dir = $config_root | path join dupin
    let log = $run_dir | path join dupin.log
    let pid_file = $run_dir | path join app.pid
    mkdir $task_home $config_dir $data_root
    config-text | save ($config_dir | path join config.toml)

    let tag = random int 1000..2147483000
    let environment = {
        HOME: ($task_home | into string)
        XDG_CONFIG_HOME: ($config_root | into string)
        XDG_DATA_HOME: ($data_root | into string)
        DUPIN_TRACE: '1'
        DUPIN_DISABLE_UPDATES: '1'
        APP_BINARY: ($executable | into string)
        APP_LOG: ($log | into string)
        PID_FILE: ($pid_file | into string)
    }
    let spawned_at = date now
    let job_id = job spawn --description $"measure ($label) pair ($pair + 1)" {
        let process = with-env $environment {
            let args = ['/bin/sh', '-c', $RUNNER]
            run-external ...$args | complete
        }
        {exit_code: $process.exit_code, stderr: $process.stderr} | job send 0 --tag $tag
    }

    let outcome = try {
        let ready = wait-ready $log $ready_timeout
        let ready_detected_at = date now
        if not ($pid_file | path exists) {
            error make {msg: 'Dupin reported readiness without a launcher PID file'}
        }
        let spawned_pid = open --raw $pid_file | str trim | into int
        if $spawned_pid != $ready.pid {
            error make {msg: 'launcher PID and readiness PID did not match'}
        }
        let samples = $sample_seconds | each {|seconds|
            let target = $ready_detected_at + ($seconds * 1sec)
            let remaining = $target - (date now)
            if $remaining > 0sec { sleep $remaining }
            let observed = (date now) - $ready_detected_at
            let usage = checked $helper --pid ($ready.pid | into string) ($executable | into string) | from json
            $usage
            | insert scheduled_after_ready_seconds $seconds
            | insert observed_after_ready_ms ($observed / 1ms)
        }
        {
            ok: true
            value: {
                pair: ($pair + 1)
                order: $position
                variant: $label
                pid: $ready.pid
                startup_ms: $ready.startup_ms
                runner_spawn_to_ready_ms: (($ready_detected_at - $spawned_at) / 1ms)
                samples: $samples
                log: ($log | path relative-to $work)
            }
        }
    } catch {|error|
        {ok: false, error: $error.msg}
    }

    let cleanup = try {
        stop-owned $helper $pid_file $executable
        {ok: true}
    } catch {|error|
        {ok: false, error: $error.msg}
    }
    let finished = try {
        job recv --tag $tag --timeout 10sec
    } catch {|_| null }
    if $finished == null {
        job kill $job_id
        error make {msg: $"launcher job did not finish after cleanup: ($run_dir)"}
    }
    if not $cleanup.ok {
        error make {msg: $"could not clean up measured process: ($cleanup.error)"}
    }
    if not $outcome.ok {
        error make {
            msg: $"($label) pair ($pair + 1) failed: ($outcome.error); child exit ($finished.exit_code): ($finished.stderr)"
        }
    }
    $outcome.value
}

def main [
    native_app: path
    wasm_app: path
    --output: path
    --pairs: int = 3
    --ready-timeout: duration = 20sec
    --sample-seconds: list<int> = [1, 5, 15]
]: nothing -> nothing {
    if (uname | get operating-system) != 'Darwin' or (uname | get machine) != 'arm64' {
        error make {msg: 'This experiment requires an Apple silicon Mac.'}
    }
    if $pairs < 1 or $pairs > 10 {
        error make {msg: '--pairs must be between 1 and 10'}
    }
    if ($sample_seconds | is-empty) or ($sample_seconds | any {|seconds| $seconds < 0 or $seconds > 60 }) {
        error make {msg: '--sample-seconds must contain values from 0 through 60'}
    }
    if ($sample_seconds | sort) != $sample_seconds {
        error make {msg: '--sample-seconds must be in ascending order'}
    }
    let output = $output | path expand
    if ($output | path exists) {
        error make {msg: $"output path already exists: ($output)"}
    }
    mkdir $output
    let work = mktemp --directory | path expand --strict
    let script = $env.FILE_PWD | path join measure.nu | path expand --strict
    let source_root = $env.FILE_PWD | path dirname | path dirname | path expand --strict
    let usage_source = $env.FILE_PWD | path join usage.c | path expand --strict
    let helper = $work | path join usage
    let system = uname
    let metadata = {
        source: {
            commit: (checked git -C $source_root rev-parse HEAD)
            status: (checked git -C $source_root status --porcelain)
            github_sha: ($env.GITHUB_SHA? | default null)
            pull_request_head: ($env.PR_HEAD_SHA? | default null)
        }
        script_sha256: (sha256 $script)
        helper_source_sha256: (sha256 $usage_source)
        system: $system
        hardware: (checked /usr/sbin/sysctl -n machdep.cpu.brand_string)
        macos: (checked /usr/bin/sw_vers)
        clang: (checked /usr/bin/clang --version)
        nushell: (checked nu --version)
        runtime_measurement: 'Pre-query process startup and idle samples; no launcher interaction or plugin query is sent.'
        startup_definition: 'Monotonic duration from entry to main through the panel on_ready callback.'
        memory_api: 'proc_pid_rusage RUSAGE_INFO_V4: current physical footprint, resident bytes and lifetime maximum footprint through each sample.'
        sample_seconds_after_ready: $sample_seconds
        runner_config: (config-text)
        signing: 'Disposable app copies; Nix store links replaced with macOS system libraries; ad-hoc hardened-runtime signature; no extra entitlements.'
    }

    $metadata | to json | save ($output | path join metadata.json)

    (checked
        /usr/bin/clang
        -std=c11
        -Wall
        -Wextra
        -Werror
        -O2
        $usage_source
        -o
        $helper
    ) | ignore
    let self_sample = checked $helper --self | from json
    if $self_sample.physical_footprint_bytes <= 0 or $self_sample.resident_bytes <= 0 {
        error make {msg: 'resource-usage helper self-check returned an empty measurement'}
    }
    let native = prepare-app $native_app ($work | path join native.app)
    let wasm = prepare-app $wasm_app ($work | path join wasm.app)
    if $native.executable_sha256 == $wasm.executable_sha256 {
        error make {
            msg: 'native-only and Wasm app executables have identical SHA-256 values'
        }
    }
    let variants = {native: $native, wasm: $wasm}
    let guest = $wasm.app | path join Contents Resources plugins calculator.wasm
    if not ($guest | path exists) or ($native.app | path join Contents Resources plugins calculator.wasm | path exists) {
        error make {msg: 'expected a calculator module only in the Wasm bundle'}
    }
    let provenance = $metadata | insert bundles $variants | insert calculator_sha256 (sha256 $guest)
    $provenance | to json | save --force ($output | path join metadata.json)
    let journal = $output | path join measurements.jsonl
    '' | save $journal
    let outcome = try {
        0..<$pairs | each {|pair|
            variant-order $pair | enumerate | each {|entry|
                let label = $entry.item
                let measurement = one-run $helper ($variants | get $label | get executable) $label $pair $entry.index $output $sample_seconds $ready_timeout
                $measurement | to json --raw | $"($in)\n" | save --append $journal
            }
        } | ignore
        {ok: true, problem: null}
    } catch {|error|
        {ok: false, problem: $error.msg}
    }
    let measurements = open --raw $journal | lines | each { from json }
    let result = {
        metadata: $provenance
        outcome: $outcome
        measurements: $measurements
        notes: [
            'The app stays hidden and receives no UI input; hotkey registration and update checks are disabled in an isolated per-run home.'
            'Measurements stop before the first plugin query, so they cover host startup and pre-query idle runtime cost only.'
            'The calculator module is never loaded or instantiated during these runs.'
            'Pair 1 is the first launch of each freshly signed copy; later pairs reuse those copies. This is not a cold-machine comparison.'
            'runner_spawn_to_ready_ms uses the wall clock and 25 ms readiness polling, including shell, exec and pre-main time.'
            'Lifetime maximum footprint includes startup and idle wait through the sample; it does not include a query or render peak.'
            'Each pair alternates which variant starts first. Raw per-process observations are retained; this experiment does not set an adoption budget.'
        ]
    }
    let result_path = $output | path join results.json
    $result | to json | save --force $result_path
    rm --recursive --force $work
    if not $outcome.ok {
        error make {
            msg: $"measurement failed; partial results saved in ($result_path): ($outcome.problem)"
        }
    }
    print (
        {
            output: $output
            results: $result_path
            measurements: ($measurements | length)
        }
        | to json
    )
}
