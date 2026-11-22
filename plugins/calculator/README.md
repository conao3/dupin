# Calculator guest

This separate workspace builds an rlib and a bare wasm32-unknown-unknown cdylib,
with public `handle` and the existing v1 JSON Wasm exports. It has no launcher
dependency. See the [fixture commands](../../experiments/plugin-protocol/README.md).
Generated Wasm modules are not checked in.

Fend-core 1.5.9 parses and evaluates each query in a fresh context with default
temperature mode, no RNG and no exchange-rate handler. Host fuel, cancellation
and deadlines remain authoritative; the guest supplies no clock.

A completeness scan rejects unmatched delimiters because fend accepts missing
closing parentheses. It skips escaped strings, raw strings and fend comments,
and recognises numeric foot/inch quotes. Fend still validates expressions.
Input and output control characters are rejected.

Only four-word scalar temperature conversions such as `100 f in c` and
`-40 c to f` normalise the temperature symbols. Other occurrences retain fend's
unit meanings: `f` is farad and `c` is the speed of light. Context-wide custom
aliases would override these meanings, so they are deliberately avoided.

Normal evaluation retains long answers such as `2^200`, unlike the preview
wrapper's 50-byte cap. Output must contain a numeric span, fit the protocol's
16 KiB Pair result bound, be single-line and differ from the input. Plain scalar
literals and bare identifiers are hidden. Alphabetic unit phrases whose
numeric spans contain only implicit `1` are hidden too. Fend supplies formatted
spans rather than semantic unit metadata; this is a display policy rather than
a semantic classification API. Numeric expressions such as `sin pi` and complex
unit conversions retain their engine output without guessed unit captions.

Invalid, incomplete and unknown calculations return empty Results. Unsupported
commands and supplied state retain explicit protocol errors. A zero limit
returns before evaluation. Results use generic Pair source/result views and
empty actions: action descriptors currently have no execution contract.

Enter-copy, broker permissions, runtime adoption measurements, registration and
the landing demo remain parent integration work. This does not ship a launcher
calculator.
