# App-level Wasm size experiment

This measures a pinned Dupin revision, not the current working tree. It applies a
measurement-only patch to disposable `git archive` snapshots and leaves the app's
normal startup, permissions and local configuration untouched.

```sh
nu experiments/wasm-app-size/measure.nu /path/to/calculator.wasm --prepare-only
nu experiments/wasm-app-size/measure.nu /path/to/calculator.wasm
```

Use a bare guest built by the calculator experiment, with its private allocation
and evaluation exports. The initial reference guest is `fend-core` 1.5.9. The
script records the actual guest bytes and SHA-256, so differently built guests
are distinguishable. The fixture is trusted; this is not third-party plugin
installation or a public ABI.

The default baseline is commit `ac3991952f90435ea207e7c0bd9f876dec371028`.
The baseline's flake pins the package toolchain, fonts and packaging tools.
Run on an Apple silicon Mac with Nix, Nushell, Git and `patch`. Full builds can
take several minutes. `--prepare-only` validates source extraction and patching
without compiling the application.

The variants are:

- unchanged Dupin;
- Wasmi 2.0.2 with `std`, `validate` and `auto-dispatch`, fuel enabled and a 16 MiB linear-memory
  limit, loading an empty module;
- the runtime plus a minimal external guest that returns 42;
- the runtime plus the calculator and minimal guests as separate resources.

Each variant is stripped through the package's existing Nix rules. The script
copies each app, uses the release packager's shared helper to replace Nix libiconv
links with the system library, ad-hoc signs it with the hardened runtime, verifies its signature
and creates the same ZIP format used by the updater. It records raw and signed
executable bytes, the sum of regular-file bytes in the signed app, and ZIP bytes.
It then invokes explicit probe arguments before GPU prewarming or normal app
startup, verifying that runtime/guest code was retained by LTO and executes.
Results and app copies remain in the reported temporary directory; `results.json`
contains structured measurements and build provenance.
Both Nix outputs have GC roots in that directory so a later build cannot collect
the earlier package. Removing the run directory releases those roots.

The patch saves the runtime executable before building the guest variants and
restores it before Nix's install hook. Otherwise the last linked variant would be
installed under the runtime-only label.

## Interpretation

The recorded Apple silicon run is in
[`results/2026-11-22-aarch64-darwin.json`](results/2026-11-22-aarch64-darwin.json).
All three signed-app probes passed, without additional entitlements.
This historical run used the patch before `auto-dispatch` was enabled. Its
recorded patch hash and measurements remain unchanged; the current patch needs a
new full run before claiming its bundle sizes.

| Variant                              | Signed app bytes |  ZIP bytes | App increase | ZIP increase |
| ------------------------------------ | ---------------: | ---------: | -----------: | -----------: |
| Baseline                             |       18,423,525 |  9,557,772 |            — |            — |
| Runtime                              |       19,559,797 |  9,954,186 |    1,136,272 |      396,414 |
| Runtime + minimal guest              |       19,576,474 |  9,957,490 |    1,152,949 |      399,718 |
| Runtime + calculator + minimal guest |       20,326,614 | 10,211,718 |    1,903,089 |      653,946 |

The calculator variant adds about 1.81 MiB to the app and 0.62 MiB to the ZIP.
Its executable is 144 bytes smaller than the runtime-only variant; whole-program
linking can change layout, so this is not a negative guest-code cost. Guest
resources are counted separately in the app total. Recorded artifact paths are
relative to the run directory.

These are deltas against the pinned app. They include the host probe code and
external resources, rather than treating a standalone interpreter as the app's
incremental cost. The minimal guest bundled alongside the calculator has no
duplicated feature libraries; two realistic feature guests remain a separate
measurement requirement.

Ad-hoc signing verifies hardened-runtime compatibility without asking for signing
credentials. It does not reproduce Developer ID certificates, notary submission or
stapling. ZIP metadata and signing can vary between runs. No DMG or memory/latency
measurement is included here; the calculator experiment covers execution timing
and limits separately. These results alone do not adopt a runtime.

## Current integrated footprint gate

`measure-integrated.nu` measures the app and guest layout used by the integrated
plugin host. Run it from an Apple silicon Mac after committing the source being
measured:

```sh
nu experiments/wasm-app-size/measure-integrated.nu --prepare-only
nu experiments/wasm-app-size/measure-integrated.nu
```

The first command records the source state and toolchain metadata without
building the app variants. A full run builds five Nix outputs from that exact
Git revision, with GC roots inside its temporary directory, then signs and
archives disposable app copies. It records the
stripped executable size, signed executable size, total regular-file bytes in
the signed app, and updater-format ZIP size for each case:

- native app built with the Wasm runtime feature disabled;
- the integrated runtime executable with its plugin resources removed;
- that same executable with a minimal valid Wasm plugin resource;
- the same executable with the calculator guest;
- the same executable with both the calculator and offline currency fixture.

Only the first two cases change the executable. The final three use the same
integrated binary and vary its bundled resources. The minimal plugin exports the
runtime ABI and returns a static protocol-v2 empty-results reply. Its only valid
correlation is for a request with `request_id = "minimal-probe-1"` and
`generation = "1"`; the fixture ignores the other request fields. The app-size
script only bundles this resource and never launches or executes it. The
currency fixture is included only to count a second realistic guest and is not
registered as a product feature.

The JSON records the source commit and dirty state, the script and packaging
helper hashes, the root nixpkgs lock entry, Rust and macOS versions, Nix output
paths, runtime settings, and each guest's size and SHA-256. The temporary
directory is printed with the result; it contains `results.json`, signed apps,
ZIPs, and the Nix GC roots. Removing it releases those roots.

This gate measures package footprint only. It does not launch the app, execute
the guest, benchmark queries, measure cancellation or memory, or prove runtime
adoption. Ad-hoc signing checks hardened-runtime compatibility without
additional entitlements, Developer ID signing, notarisation, or stapling.

### Integrated run, 22 November 2026

[Current recorded results](results/2026-11-22-integrated-cached-aarch64-darwin.json) use
`3ca4203f08c1885afb4c207be4adbc91eb9a4f0d`, a clean source snapshot after
the Cargo-cache integration. Sizes are bytes; app totals include regular files
after signing. The [earlier run](results/2026-11-22-integrated-aarch64-darwin.json)
remains as a historical record of its own pinned source.

| Variant                                  | Stripped executable | Signed app | Updater ZIP |
| ---------------------------------------- | ------------------: | ---------: | ----------: |
| Native only                              |          18,603,456 | 18,637,477 |   9,683,804 |
| Runtime, no guests                       |          19,878,816 | 19,905,413 |  10,152,819 |
| Runtime, minimal ABI guest               |          19,878,816 | 19,905,983 |  10,154,428 |
| Runtime and calculator                   |          19,878,816 | 20,789,368 |  10,459,601 |
| Runtime, calculator and currency fixture |          19,878,816 | 21,657,488 |  10,762,846 |

The calculator build adds 2,151,891 app bytes and 775,797 ZIP bytes
against the same-source native-only build. Adding the second realistic guest
adds another 868,120 app bytes and 303,245 ZIP bytes. Both guests contain their
own evaluator and encoding code; the second-guest case counts that duplication
rather than assuming a shared guest library.

All signed variants passed strict verification with hardened runtime and no
additional entitlements. The record retains the temporary app paths; the app
and ZIP files themselves are not committed.

The [current execution probe](../integrated-runtime-probe/README.md) checks these
exact packaged guests against their native implementations. Package size and
query evidence do not complete adoption: paired app startup and idle-memory
measurements remain outstanding.

The [remapped-guest rerun](results/2026-11-22-integrated-remapped-aarch64-darwin.json)
records the same five package variants after removing temporary vendor paths
from both standalone guests. Both guest packages passed a repeated Nix build
with identical bytes. The earlier records retain their original source and
artifact hashes.
