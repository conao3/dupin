use std::{
    env, fs,
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use dupin_plugin_protocol::{Operation, Outcome, ProtocolError, Request, Response, State, VERSION, View};
use dupin_plugin_runtime::{GuestModule, HostError, MAX_MODULE_BYTES, QueryControl};
use serde_json::{Value, json};

const ITERATIONS: usize = 20;
const DEADLINE: Duration = Duration::from_secs(2);
const QUERY_FUEL: u64 = 20_000_000;
const CURRENCY_REQUEST: &[u8] = include_bytes!("../../currency-guest/fixtures/request.json");

fn request(command: &str, query: &str, state: Option<State>) -> Request {
    Request {
        version: VERSION,
        request_id: "runtime-probe-1".into(),
        generation: "1".into(),
        command: command.into(),
        query: query.into(),
        limit: 1,
        state,
    }
}

fn query(module: &GuestModule, request: &Request) -> Result<Response, HostError> {
    module.query_controlled(
        request,
        &QueryControl {
            deadline: Instant::now() + DEADLINE,
            is_cancelled: &|| false,
        },
    )
}

fn results(response: &Response) -> &[dupin_plugin_protocol::PluginItem] {
    match &response.outcome {
        Outcome::Results { items, state } => {
            assert!(state.is_none(), "these guests do not persist query state");
            items
        }
        Outcome::Error { code, message } => panic!("guest returned {code:?}: {message}"),
    }
}

fn assert_calculator(response: &Response, query_text: &str, expected: &str) {
    let items = results(response);
    assert_eq!(items.len(), 1, "{query_text}");
    assert_eq!(
        items[0].view,
        View::Pair {
            source: query_text.into(),
            result: expected.into(),
        }
    );
    assert_eq!(items[0].primary.as_deref(), Some("copy-result"));
    assert_eq!(
        items[0].actions[0].operation,
        Operation::CopyText {
            text: expected.into()
        }
    );
}

fn measure(
    backend: &str,
    request: &Request,
    first_query_for_backend: bool,
    mut invoke: impl FnMut() -> Response,
    validate: &impl Fn(&Response),
) -> (Value, Response) {
    let started = Instant::now();
    let response = invoke();
    let first_query_ns = started.elapsed().as_nanos();
    validate(&response);
    let first_response = response.clone();
    black_box(response);

    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let started = Instant::now();
        let response = invoke();
        samples.push(started.elapsed().as_nanos());
        validate(&response);
        black_box(response);
    }
    samples.sort_unstable();
    (
        json!({
            "backend": backend,
            "command": request.command,
            "query": request.query,
            "iterations": ITERATIONS,
            "first_query_ns": first_query_ns,
            "first_query_for_backend": first_query_for_backend,
            "warm_query_median_ns": samples[samples.len() / 2],
            "warm_query_p95_ns": samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
        }),
        first_response,
    )
}

fn native_query(handle: fn(&[u8]) -> Result<Vec<u8>, ProtocolError>, request: &Request) -> Response {
    let bytes = request.encode().expect("native request must encode");
    let response = handle(&bytes).expect("native plugin should handle request");
    Response::decode(&response, request).expect("native response must validate and correlate")
}

fn compare(
    module: &GuestModule,
    request: &Request,
    first_query_for_backend: bool,
    native_handle: fn(&[u8]) -> Result<Vec<u8>, ProtocolError>,
    validate: impl Fn(&Response),
) -> Value {
    let (wasm_metrics, wasm_response) = measure(
        "shared_wasm_runtime",
        request,
        first_query_for_backend,
        || query(module, request).expect("Wasm query should succeed"),
        &validate,
    );
    let (native_metrics, native_response) = measure(
        "native_plugin",
        request,
        first_query_for_backend,
        || native_query(native_handle, request),
        &validate,
    );
    assert_eq!(wasm_response, native_response, "backend responses differ");
    json!({
        "wasm": wasm_metrics,
        "native": native_metrics,
        "responses_match": true,
        "measurement_boundary": "Request.encode -> plugin handle -> Response.decode",
    })
}

fn currency_state() -> State {
    let request: Request = serde_json::from_slice(CURRENCY_REQUEST).expect("fixture request JSON");
    assert_eq!(request.command, "currency");
    assert_eq!(request.query, "30*(60+1) usd in jpy");
    request
        .state
        .expect("fixture request includes bounded rate state")
}

fn assert_currency(response: &Response) {
    let items = results(response);
    assert_eq!(items.len(), 1);
    let View::Detail { body, .. } = &items[0].view else {
        panic!("currency fixture must return a detail view");
    };
    assert!(
        body.starts_with("274500 JPY\n"),
        "unexpected fixture body: {body}"
    );
    assert!(body.contains("observation date: 2026-01-15"));
    assert_eq!(items[0].primary.as_deref(), Some("copy-result"));
    assert_eq!(
        items[0].actions[0].operation,
        Operation::CopyText {
            text: "274500 JPY".into()
        }
    );
}

fn measure_currency(module: &GuestModule) -> Value {
    let request = request("currency", "30*(60+1) usd in jpy", Some(currency_state()));
    compare(
        module,
        &request,
        true,
        dupin_currency_guest::handle,
        assert_currency,
    )
}

fn verify_minimal(bytes: &[u8]) -> Result<Value, HostError> {
    assert!(bytes.len() <= MAX_MODULE_BYTES);
    let started = Instant::now();
    let module = GuestModule::load(bytes)?;
    let module_load_ns = started.elapsed().as_nanos();
    let request = Request {
        version: VERSION,
        request_id: "minimal-probe-1".into(),
        generation: "1".into(),
        command: "minimal".into(),
        query: "probe".into(),
        limit: 1,
        state: None,
    };
    let started = Instant::now();
    let response = query(&module, &request)?;
    let first_query_ns = started.elapsed().as_nanos();
    let Outcome::Results { items, state } = response.outcome else {
        panic!("minimal guest must return an empty result set");
    };
    assert!(items.is_empty(), "minimal guest must return no results");
    assert!(state.is_none(), "minimal guest must not return saved state");
    Ok(json!({
        "wasm_bytes": bytes.len(),
        "module_load_ns": module_load_ns,
        "first_query_ns": first_query_ns,
        "first_query_for_module": true,
        "valid_empty_protocol_response": true,
    }))
}

fn verify_limits(module: &GuestModule) -> Value {
    let pathological = request("calculator", "2^1000000", None);
    let started = Instant::now();
    assert_eq!(query(module, &pathological), Err(HostError::Fuel));
    let fuel_exhaustion_ns = started.elapsed().as_nanos();

    let cancellable = request("calculator", "2^1000000", None);
    let checks = AtomicUsize::new(0);
    let started = Instant::now();
    let result = module.query_controlled(
        &cancellable,
        &QueryControl {
            deadline: Instant::now() + DEADLINE,
            is_cancelled: &|| checks.fetch_add(1, Ordering::Relaxed) >= 10,
        },
    );
    let cancellation_latency_ns = started.elapsed().as_nanos();
    assert_eq!(result, Err(HostError::Cancelled));
    let cancellation_checks = checks.load(Ordering::Relaxed);
    json!({
        "pathological_query_fuel_limit": QUERY_FUEL,
        "fuel_exhaustion_ns": fuel_exhaustion_ns,
        "fuel_exhaustion_verified": true,
        "cancellation_checks": cancellation_checks,
        "cancellation_latency_ns": cancellation_latency_ns,
        "cancellation_during_query_verified": true,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let calculator_path = args
        .next()
        .ok_or("pass calculator.wasm and currency-fixture.wasm paths")?;
    let currency_path = args
        .next()
        .ok_or("pass calculator.wasm and currency-fixture.wasm paths")?;
    let minimal_path = args.next();
    if args.next().is_some() {
        return Err("expected two module paths and an optional minimal guest path".into());
    }
    let calculator_bytes = fs::read(calculator_path)?;
    let currency_bytes = fs::read(currency_path)?;
    assert!(calculator_bytes.len() <= MAX_MODULE_BYTES);
    assert!(currency_bytes.len() <= MAX_MODULE_BYTES);

    let started = Instant::now();
    let calculator = GuestModule::load(&calculator_bytes)?;
    let calculator_module_load_ns = started.elapsed().as_nanos();
    let started = Instant::now();
    let currency = GuestModule::load(&currency_bytes)?;
    let currency_module_load_ns = started.elapsed().as_nanos();

    let representative = [
        ("30*(60+1)", "1830"),
        ("560km to mi", "approx. 347.9678676529 mi"),
        ("100 f in c", "approx. 37.7777777778 °C"),
        ("30*(60+1)km/h in m/s", "approx. 508.3333333333 m / s"),
    ];
    let calculator_metrics = representative
        .iter()
        .enumerate()
        .map(|(index, (query_text, expected))| {
            compare(
                &calculator,
                &request("calculator", query_text, None),
                index == 0,
                dupin_calculator::handle,
                |response| assert_calculator(response, query_text, expected),
            )
        })
        .collect::<Vec<_>>();
    let currency_metrics = measure_currency(&currency);
    let minimal_metrics = minimal_path
        .map(fs::read)
        .transpose()?
        .as_deref()
        .map(verify_minimal)
        .transpose()?;
    let limits = verify_limits(&calculator);

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "runtime": {
                "crate": "dupin-plugin-runtime",
                "native_comparison": "same standalone process, current native plugin libraries and shared Wasm runtime; same V2 request and response decoding",
                "compilation_mode": "Eager",
                "query_deadline_ms": DEADLINE.as_millis(),
                "query_fuel_limit": QUERY_FUEL,
                "max_module_bytes": MAX_MODULE_BYTES,
                "memory_limit_bytes": 16 * 1024 * 1024,
                "store_per_query": true,
                "iterations_per_query": ITERATIONS,
                "first_query_measurement": "first sample for each workload; first_query_for_backend identifies first workload query to that backend",
            },
            "modules": {
                "calculator": {
                    "wasm_bytes": calculator_bytes.len(),
                    "module_load_ns": calculator_module_load_ns,
                    "queries": calculator_metrics,
                },
                "currency_fixture": {
                    "wasm_bytes": currency_bytes.len(),
                    "module_load_ns": currency_module_load_ns,
                    "query": currency_metrics,
                    "note": "offline synthetic rates; fixture guest is not registered in Dupin",
                }
            },
            "minimal_abi_guest": minimal_metrics,
            "limits": limits,
            "scope": "standalone runtime probe; excludes app startup, UI, OS services and process memory",
        }))?
    );
    Ok(())
}
