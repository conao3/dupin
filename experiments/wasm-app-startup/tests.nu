#!/usr/bin/env nu

use ./measure.nu [config-text parse-ready-line ready-marker-text variant-order]

def main []: nothing -> nothing {
    let config = config-text | from toml
    if $config.hotkey != '' or $config.auto_update != false or $config.plugins.calculator.grants != [] {
        error make {
            msg: 'measurement config must disable hotkeys, updates and calculator grants'
        }
    }
    if (variant-order 0) != [native wasm] or (variant-order 1) != [wasm native] or (variant-order 2) != [native wasm] {
        error make {msg: 'paired runs must alternate the starting variant'}
    }
    let marker = parse-ready-line 'dupin-trace: ready pid=42 startup_ms=123.456'
    if $marker.pid != 42 or $marker.startup_ms != 123.456 {
        error make {msg: 'readiness marker parsing changed'}
    }
    for partial in ['dupin-trace: ready pid=', 'dupin-trace: ready pid=42 startup_ms=123.456'] {
        if (ready-marker-text $partial) != null {
            error make {msg: 'readiness must wait for a complete line'}
        }
    }
    if (ready-marker-text "another trace\ndupin-trace: ready pid=42 startup_ms=123.456\n") != $marker {
        error make {msg: 'completed readiness marker was not found'}
    }
    let rejects_invalid = try {
        parse-ready-line 'dupin-trace: ready pid=not-a-pid startup_ms=0.000'
        false
    } catch {
        true
    }
    if not $rejects_invalid {
        error make {msg: 'readiness parser accepted an invalid PID'}
    }
    print 'startup measurement fixtures passed'
}
