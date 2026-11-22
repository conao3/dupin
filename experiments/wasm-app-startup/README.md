# Paired app startup and idle memory

This experiment compares the packaged native-only app with the integrated Wasm
app on the same disposable Apple silicon macOS runner. Build both packages from
the same checkout, then pass their `.app` bundle paths:

```sh
nix build .#dupin-no-wasm --out-link target/native-only
nix build .#dupin --out-link target/wasm
nu experiments/wasm-app-startup/measure.nu \
  --output "$RUNNER_TEMP/wasm-startup" \
  target/native-only/Applications/Dupin.app \
  target/wasm/Applications/Dupin.app
```

The runner makes disposable copies, replaces Nix library links with the system
`libiconv`, and ad-hoc signs the copies with the hardened runtime and no extra
entitlements. Each measured launch gets its own `HOME`, `XDG_CONFIG_HOME` and
`XDG_DATA_HOME`. Its config disables the global hotkey and automatic updates,
and leaves the calculator's clipboard grant empty. The app is started directly
and stays hidden; the runner sends no keys, activates no window and issues no
plugin query.

With `DUPIN_TRACE=1`, the panel's `on_ready` callback reports the PID and the
monotonic duration from entry to `main` through native panel readiness. The
runner waits at most 20 seconds for this marker, then samples the same process
at 1, 5 and 15 seconds after it observes readiness. A small helper reads
`proc_pid_rusage` `RUSAGE_INFO_V4`: current physical footprint, resident bytes,
and lifetime maximum footprint through each sample. Before signaling a process
at cleanup, the helper checks that its executable path matches the bundle it
started.

Three paired runs alternate which app starts first. The JSON output records all
individual samples, executable SHA-256 values, source status, macOS and hardware
versions, helper and script hashes, and the signing and measurement definitions.
It does not set an adoption budget or claim results before a run is recorded.

These numbers cover host startup and pre-query idle use only. The lifetime peak
includes startup and the quiet wait through a sample; it does not include
calculator execution or a rendered launcher frame. They do not measure cold
machine startup, hotkey activation latency or GPU render peak. `scripts/footprint.nu`
continues to cover a separate headless render-peak check.

The Nushell fixtures validate the isolated config, alternating order and ready
marker parser without launching Dupin:

```sh
nu experiments/wasm-app-startup/tests.nu
```
