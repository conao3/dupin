#!/usr/bin/env nu

use ../../scripts/unlink-store.nu

def --wrapped checked [...args: string]: nothing -> string {
    let result = run-external ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($args.0) failed:\n($result.stderr)"}
    }
    $result.stdout | str trim
}

def bytes [file: path]: nothing -> int {
    ls --all $file | get 0.size | into int
}

def app-bytes [app: path]: nothing -> int {
    ls --all --long $"($app)/**/*"
    | where type == file
    | get size
    | each {|size| $size | into int }
    | math sum
}

def package [
    label: string
    source: path
    artifacts: path
    plugins: record
]: nothing -> record {
    let app = $artifacts | path join $"($label).app"
    let archive = $artifacts | path join $"($label).zip"
    let plugin_dir = $app | path join Contents Resources plugins
    checked /usr/bin/ditto $source $app | ignore
    checked chmod -R u+w $app | ignore
    if $plugins.kind == 'none' {
        rm --recursive --force $plugin_dir
    } else if $plugins.kind == 'minimal' {
        rm --recursive --force $plugin_dir
        mkdir $plugin_dir
        cp ($plugins.module | into string) ($plugin_dir | path join minimal.wasm)
    } else if $plugins.kind == 'calculator+currency' {
        let currency = $plugins.currency | into string
        cp $currency ($plugin_dir | path join currency-fixture.wasm)
    }
    let executable = $app | path join Contents MacOS dupin
    unlink-store $executable
    checked /usr/bin/codesign --force --options runtime --sign - $app | ignore
    checked /usr/bin/codesign --verify --strict $app | ignore
    checked /usr/bin/ditto -c -k --keepParent $app $archive | ignore
    {
        label: $label
        raw_executable_bytes: (bytes ($source | path join Contents MacOS dupin))
        signed_executable_bytes: (bytes $executable)
        signed_app_file_bytes: (app-bytes $app)
        zip_bytes: (bytes $archive)
        app: $app
    }
}

def nix-output [flake: string, attribute: string, link: path]: nothing -> string {
    print --stderr $"Building ($attribute)…"
    (checked
        nix
        build
        $"($flake)#($attribute)"
        --out-link
        $link
        --print-out-paths
    )
}

def main [--prepare-only]: nothing -> record {
    let root = $env.FILE_PWD | path dirname | path dirname | path expand --strict
    let script = $env.FILE_PWD | path join measure-integrated.nu
    let system = checked nix eval --impure --raw --expr builtins.currentSystem
    if $system != 'aarch64-darwin' {
        error make {msg: 'This experiment requires an Apple silicon Mac.'}
    }
    let git_head = checked git -C $root rev-parse HEAD
    let flake = $"git+file://($root)?rev=($git_head)"
    let lock = checked git -C $root show $"($git_head):flake.lock" | from json
    let root_node = $lock.nodes | get $lock.root
    let nixpkgs = $lock.nodes | get $root_node.inputs.nixpkgs | get locked
    let status = checked git -C $root status --porcelain
    let tracked_status = checked git -C $root status --porcelain --untracked-files=no
    if not $prepare_only and not ($tracked_status | is-empty) {
        error make {
            msg: 'Commit tracked source changes before running the full footprint measurement.'
        }
    }
    let work = mktemp --directory | path expand
    let roots = $work | path join nix-roots
    let artifacts = $work | path join artifacts
    mkdir $roots $artifacts
    let metadata = {
        source: {
            head: $git_head
            dirty: (not ($status | is-empty))
            tracked_files_dirty: (not ($tracked_status | is-empty))
            status: $status
        }
        pinned_flake: $flake
        system: $system
        nixpkgs: $nixpkgs
        toolchain: (
            (checked
                nix
                shell
                --inputs-from
                $flake
                nixpkgs#rustc
                --command
                rustc
                --version
                --verbose
            )
        )
        macos: (checked /usr/bin/sw_vers)
        release_profile: 'Workspace release profile with fat LTO and one codegen unit; stripped through the pinned Nix package rules.'
        runtime: 'Wasmi 2.0.2; CompilationMode::Eager; std, validate and auto-dispatch; 16 MiB linear-memory limit; fuel enabled.'
        signing: 'Ad-hoc; hardened runtime; no additional entitlements; no notary submission.'
        archive: 'ditto ZIP with the app as the archive root, matching the updater packaging format.'
        script_sha256: (open --raw $script | hash sha256)
        unlink_store_sha256: (open --raw ($root | path join scripts unlink-store.nu) | hash sha256)
        limits: 'Footprint only. No app launch, query latency, cancellation latency, peak memory, idle cost, or runtime adoption is measured.'
        minimum_guest: 'Minimal ABI-v1 guest returning protocol-v2 empty results. The fixed reply correlates only with request_id=minimal-probe-1 and generation=1. The footprint script does not execute it.'
        currency_guest: 'Offline currency fixture included only to measure a second realistic guest; it is not registered or shipped as a product feature.'
    }
    if $prepare_only {
        return {work: $work, metadata: $metadata, prepared: true}
    }

    let native_out = nix-output $flake 'dupin-no-wasm' ($roots | path join native-only)
    let integrated_out = nix-output $flake 'dupin' ($roots | path join integrated)
    let calculator_out = nix-output $flake 'calculator-plugin' ($roots | path join calculator)
    let currency_out = nix-output $flake 'currency-guest' ($roots | path join currency)
    let minimal_out = nix-output $flake 'minimal-plugin' ($roots | path join minimal-plugin)
    let native_app = $native_out | path join Applications Dupin.app
    let integrated_app = $integrated_out | path join Applications Dupin.app
    let calculator = $calculator_out | path join share dupin plugins calculator.wasm
    let currency = $currency_out | path join share dupin plugins currency-fixture.wasm
    let minimal_guest = $minimal_out | path join share dupin plugins minimal.wasm
    let calculator_hash = open --raw $calculator | hash sha256
    let currency_hash = open --raw $currency | hash sha256
    let minimal_hash = open --raw $minimal_guest | hash sha256
    let measured = [
        {
            label: native-only
            source: $native_app
            plugins: {kind: none}
        }
        {
            label: runtime-no-guests
            source: $integrated_app
            plugins: {kind: none}
        }
        {
            label: runtime-minimal-guest
            source: $integrated_app
            plugins: {kind: minimal, module: $minimal_guest}
        }
        {
            label: runtime-calculator
            source: $integrated_app
            plugins: {kind: calculator}
        }
        {
            label: runtime-calculator-currency
            source: $integrated_app
            plugins: {kind: 'calculator+currency', currency: $currency}
        }
    ] | each {|variant| package $variant.label $variant.source $artifacts $variant.plugins }
    let result = {
        metadata: ($metadata
            | insert nix_outputs {
                native_only: $native_out
                integrated: $integrated_out
                calculator_plugin: $calculator_out
                currency_fixture: $currency_out
                minimal_plugin: $minimal_out
            }
            | insert guests {
                minimal: {bytes: (bytes $minimal_guest), sha256: $minimal_hash}
                calculator: {bytes: (bytes $calculator), sha256: $calculator_hash}
                currency_fixture: {bytes: (bytes $currency), sha256: $currency_hash}
            }
        )
        measurements: $measured
        notes: [
            'runtime-no-guests is the actual integrated host with plugin resources removed after the Nix build.'
            'The four runtime variants use the same integrated executable; only bundled guest resources differ.'
            'The minimal guest is an ABI and protocol fixture; the footprint script bundles it but does not execute it.'
            'runtime-calculator-currency bundles two realistic guests for footprint accounting, but does not register the currency command.'
            'App bytes sum regular files; ZIP bytes include the signed app directory and ZIP metadata.'
        ]
    }
    $result | to json | save --force ($work | path join results.json)
    {work: $work, result: $result}
}
