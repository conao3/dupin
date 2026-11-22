# Integrated runtime probe

This probe compares the current native plugin libraries with the shared Wasmi runtime using the same protocol-v2 request and response decoder. Both paths measure request encoding, the plugin call, and response decoding; validation work runs outside each timed interval. Wasm uses the production runtime's Eager module loading, fresh store per query, 20 million fuel budget and two-second deadline. The native path uses the same calculator and currency crates as the guests, linked as native libraries in this standalone probe.

For each workload it reports the first sample and the median and p95 of 20 repeats. `first_query_for_backend` marks the first workload query for that backend; later calculator workloads are measured after earlier workloads have run. Module-load timings are reported separately. The optional minimal guest is measured separately because it has no matching native feature.

Build both guests with the Nix packages, then run the probe against their installed Wasm files:

```sh
nix build .#calculator-plugin --out-link result-calculator
nix build .#currency-guest --out-link result-currency
nix build .#minimal-plugin --out-link result-minimal
cargo run --manifest-path experiments/integrated-runtime-probe/Cargo.toml --release -- \
  result-calculator/share/dupin/plugins/calculator.wasm \
  result-currency/share/dupin/plugins/currency-fixture.wasm \
  result-minimal/share/dupin/plugins/minimal.wasm
```

The exact fixture request is read from `experiments/currency-guest/fixtures/request.json`; its rates are synthetic and dated. The probe checks native and Wasm response equality, displayed values, copy payloads, currency observation date, query cancellation, and fuel exhaustion at the same budget used for ordinary queries. It never runs the pathological expression natively because the native path has no fuel limit.

The optional third path is a minimal ABI-compatible guest that returns a valid empty protocol result. Its size helps estimate the runtime's minimum usable guest overhead; omitting the path skips this check.

These measurements cover the runtime and guest query boundary. They do not measure Dupin process startup, the rendered interface, macOS services, actual clipboard writes, or process memory. The currency guest is a test fixture and is not registered as a Dupin feature.
