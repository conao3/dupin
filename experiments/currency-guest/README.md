# Offline currency guest fixture

A second standalone, separately buildable guest for the runtime-size adoption
gate, exercising the offline engine path considered for #17. This is an experiment,
not shipped currency, network or storage functionality. No live rates are used.

The guest pins fend-core 1.5.9 and uses ExchangeRateFnV2 for expression evaluation
and conversion. Every evaluation creates a fresh Context with its default RNG
disabled and an immutable validated rate map. No ambient imports, WASI, clock,
arbitrary process or URL operations are needed. Host fuel and memory limits remain
owned by the shared runtime.

The shared dupin-plugin-protocol crate owns JSON protocol version 2 and typed
CopyText actions. The adapter mirrors calculator's private bare Wasm memory ABI
version 1: the trusted host owns disjoint live allocation buffers and discards
the instance after each request. This is not a public pointer ABI.

## Fixture state

Request.state.version must be 1. Its data object has exactly:

- base: USD, JPY or GBP, with a corresponding rate exactly 1.
- observation_date: a real Gregorian YYYY-MM-DD date, year 0001–9999.
- fixture: true, explicitly marking synthetic rates.
- rates: one to three objects with exactly code and rate. Codes are unique
  canonical USD, JPY or GBP. Rates are positive finite numbers at most 1e12,
  expressed as currency units per one base unit.

The three-code allowlist is the supported fixture universe, not an ISO currency
catalogue. Missing or unsupported state produces an explicit error. The shared
wire decoder bounds state data to 8 KiB and request bytes to 16 KiB. Duplicate
rate codes in the array are rejected. Duplicate JSON object keys have already
become last-wins in shared State.data's Value parsing; this guest does not detect
those duplicates. Unknown fields and invalid types fail.

fixtures/request.json uses synthetic USD=1, JPY=150 and GBP=0.75 relative to USD.
The observation date is fixture metadata, not a fetch timestamp. No age,
freshness, stale-status or wall-clock inference is made.

Only command currency is supported. Limit zero returns before state parsing or
engine evaluation, after shared request decoding. A narrow character and balanced
parenthesis gate accepts numeric conversion expressions ending in `in CODE` or
`to CODE`; fend owns arithmetic and units. Assignments, strings, comments and
richer fend syntax are outside this fixture. Unknown, incomplete and non-currency
expressions produce no card.

A generic Detail displays the full computed answer and synthetic fixture
observation date. The primary copy action contains exactly the answer, bounded
to the shared 16 KiB CopyText limit.

## Reproduce

From the repository root, with Rust >=1.99 and wasm32-unknown-unknown installed:

```sh
cargo test --manifest-path experiments/currency-guest/Cargo.toml --locked
cargo clippy --manifest-path experiments/currency-guest/Cargo.toml --locked --all-targets -- -D warnings
cargo fmt --manifest-path experiments/currency-guest/Cargo.toml --check
nix shell --inputs-from . nixpkgs#lld --command cargo build --manifest-path experiments/currency-guest/Cargo.toml --locked --release --lib --target wasm32-unknown-unknown
cargo run --manifest-path experiments/plugin-protocol/Cargo.toml --locked -p dupin-protocol-host -- experiments/currency-guest/target/wasm32-unknown-unknown/release/dupin_currency_guest.wasm experiments/currency-guest/fixtures/request.json
```

Expected answer: 274500 JPY for `30*(60+1) usd in jpy`. Other acceptance answers:
15000 JPY for `100 usd to jpy`, 25 GBP for `¥5000 in gbp`. These reflect synthetic
fixture arithmetic only.

Generated Wasm and target output are not committed. Parent work owns Nix
packaging, comparative footprint measurements and the adoption decision.
Real brokered currency integration still needs rate acquisition, permissions,
state delivery/persistence, status orchestration and product integration.
This fixture provides none of those capabilities.
