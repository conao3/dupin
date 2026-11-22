use dupin_plugin_protocol::{
    ErrorCode, MAX_COPY_BYTES, Operation, Outcome, PluginAction, PluginItem, ProtocolError, Request,
    Response, State, VERSION, View,
};
#[cfg(target_arch = "wasm32")]
use dupin_plugin_protocol::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const KNOWN: &[&str] = &["USD", "JPY", "GBP"];
const MAX_RATES: usize = 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    base: String,
    observation_date: String,
    fixture: bool,
    rates: Vec<Rate>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rate {
    code: String,
    rate: f64,
}

struct Rates {
    values: BTreeMap<String, f64>,
    used: Arc<AtomicBool>,
}
impl fend_core::ExchangeRateFnV2 for Rates {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        let value = self.values.get(currency).copied().ok_or("Currency unavailable")?;
        self.used.store(true, Ordering::Relaxed);
        Ok(value)
    }
}
fn snapshot(state: Option<State>) -> Result<Snapshot, &'static str> {
    let state = state.ok_or("Fixture rates unavailable: state is missing.")?;
    if state.version != 1 {
        return Err("Unsupported fixture state version.");
    }
    let snapshot: Snapshot =
        serde_json::from_value(state.data).map_err(|_| "Invalid fixture state schema.")?;
    if !snapshot.fixture
        || !KNOWN.contains(&snapshot.base.as_str())
        || snapshot.rates.is_empty()
        || snapshot.rates.len() > MAX_RATES
        || !valid_date(&snapshot.observation_date)
    {
        return Err("Invalid fixture metadata or rate count.");
    }
    let mut seen = BTreeMap::new();
    for entry in &snapshot.rates {
        if !KNOWN.contains(&entry.code.as_str())
            || !entry.rate.is_finite()
            || entry.rate <= 0.0
            || entry.rate > 1e12
            || seen.insert(&entry.code, entry.rate).is_some()
        {
            return Err("Invalid, duplicate or unsupported fixture rate.");
        }
    }
    if seen.get(&snapshot.base) != Some(&1.0) {
        return Err("Fixture base rate must equal one.");
    }
    Ok(snapshot)
}
fn valid_date(date: &str) -> bool {
    let b = date.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = date[..4].parse().unwrap();
    let month: usize = date[5..7].parse().unwrap();
    let day: u32 = date[8..].parse().unwrap();
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    year > 0 && (1..=12).contains(&month) && day > 0 && day <= days[month - 1]
}
fn calculate(query: &str, snapshot: Snapshot) -> Option<String> {
    // The gate recognises conversion requests; fend owns expression semantics.
    let (expression, target) = query.rsplit_once(" in ").or_else(|| query.rsplit_once(" to "))?;
    let target = target.trim().to_ascii_uppercase();
    if !snapshot.rates.iter().any(|r| r.code == target) {
        return None;
    }
    let mut depth = 0usize;
    for c in expression.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            _ if c.is_ascii_alphanumeric()
                || matches!(c, ' ' | '.' | '+' | '-' | '*' | '/' | '^' | '%' | '¥' | '$' | '£') => {}
            _ => return None,
        }
    }
    if depth != 0 || !expression.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let used = Arc::new(AtomicBool::new(false));
    let mut context = fend_core::Context::new();
    context.set_exchange_rate_handler_v2(Rates {
        values: snapshot.rates.into_iter().map(|r| (r.code, r.rate)).collect(),
        used: used.clone(),
    });
    let result = fend_core::evaluate(query, &mut context).ok()?;
    let answer = result.get_main_result();
    if !used.load(Ordering::Relaxed)
        || result.output_is_empty()
        || answer.len() > MAX_COPY_BYTES
        || answer.chars().any(char::is_control)
        || !answer.ends_with(&format!(" {target}"))
        || !result
            .get_main_result_spans()
            .any(|s| s.kind() == fend_core::SpanKind::Number)
    {
        return None;
    }
    Some(answer.to_owned())
}
pub fn handle(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    handle_with(bytes, calculate)
}
fn handle_with(
    bytes: &[u8],
    evaluate: impl FnOnce(&str, Snapshot) -> Option<String>,
) -> Result<Vec<u8>, ProtocolError> {
    let request = Request::decode(bytes)?;
    let empty = || Outcome::Results {
        items: vec![],
        state: None,
    };
    let outcome = if request.command != "currency" {
        Outcome::Error {
            code: ErrorCode::UnsupportedCommand,
            message: "This fixture only supports currency.".into(),
        }
    } else if request.limit == 0 {
        empty()
    } else {
        match snapshot(request.state.clone()) {
            Err(message) => Outcome::Error {
                code: ErrorCode::InvalidQuery,
                message: message.into(),
            },
            Ok(snapshot) => {
                let date = snapshot.observation_date.clone();
                match evaluate(&request.query, snapshot) {
                    None => empty(),
                    Some(answer) => Outcome::Results {
                        items: vec![PluginItem {
                            id: "answer".into(),
                            view: View::Detail {
                                title: "Currency fixture".into(),
                                body: format!("{answer}\nSynthetic fixture rates; observation date: {date}"),
                            },
                            primary: Some("copy-result".into()),
                            actions: vec![PluginAction {
                                id: "copy-result".into(),
                                title: "Copy result".into(),
                                operation: Operation::CopyText { text: answer },
                            }],
                        }],
                        state: None,
                    },
                }
            }
        }
    };
    Response {
        version: VERSION,
        request_id: request.request_id.clone(),
        generation: request.generation.clone(),
        outcome,
    }
    .encode(&request)
}
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn dupin_abi_version() -> i32 {
    1
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn dupin_alloc(len: i32) -> i32 {
    if len <= 0 || len as usize > MAX_RESPONSE_BYTES {
        return 0;
    }
    Box::into_raw(vec![0_u8; len as usize].into_boxed_slice()) as *mut u8 as i32
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
/// # Safety
/// The host supplies disjoint live `dupin_alloc` buffers of the declared lengths.
/// Allocations live until this request's Wasm instance is discarded.
pub unsafe extern "C" fn dupin_query(input: i32, len: i32, output: i32, cap: i32) -> i32 {
    if input <= 0
        || output <= 0
        || len <= 0
        || len as usize > MAX_REQUEST_BYTES
        || cap <= 0
        || cap as usize > MAX_RESPONSE_BYTES
    {
        return -1;
    }
    // SAFETY: the host owns these allocations and keeps them alive for this call.
    let bytes = unsafe { std::slice::from_raw_parts(input as *const u8, len as usize) };
    let Ok(result) = handle(bytes) else {
        return -2;
    };
    if result.len() > cap as usize {
        return -3;
    }
    // SAFETY: `result` owns its allocation and the host's output buffer has enough room.
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), output as *mut u8, result.len());
    }
    result.len() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(query: &str) -> Request {
        let mut r = Request::decode(include_bytes!("../fixtures/request.json")).unwrap();
        r.query = query.into();
        r
    }
    fn outcome(r: &Request) -> Outcome {
        Response::decode(&handle(&r.encode().unwrap()).unwrap(), r)
            .unwrap()
            .outcome
    }
    #[test]
    fn conversions_and_presentation() {
        for (q, answer) in [
            ("30*(60+1) usd in jpy", "274500 JPY"),
            ("100 usd to jpy", "15000 JPY"),
            ("¥5000 in gbp", "25 GBP"),
        ] {
            let r = request(q);
            let Outcome::Results { items, .. } = outcome(&r) else {
                panic!()
            };
            assert_eq!(items.len(), 1, "{q}");
            assert_eq!(items[0].primary.as_deref(), Some("copy-result"));
            assert_eq!(
                items[0].actions[0].operation,
                Operation::CopyText { text: answer.into() }
            );
            let View::Detail { body, .. } = &items[0].view else {
                panic!()
            };
            assert!(
                body.contains(answer) && body.contains("2026-01-15") && body.contains("Synthetic fixture")
            );
        }
    }
    #[test]
    fn no_misleading_cards_and_fresh_context() {
        for q in [
            "100 xyz in jpy",
            "100 eur in jpy",
            "30*(60+1 usd in jpy",
            "2+2",
            "100 m in jpy",
            "usd in jpy",
            "x = 100 usd; x in jpy",
            "x in jpy",
            "roll d6 in jpy",
            "100 usd in",
            "100 usd in jpy + 1",
        ] {
            assert!(
                matches!(outcome(&request(q)),Outcome::Results{items,..} if items.is_empty()),
                "{q}"
            );
        }
        assert!(calculate("100 usd in jpy", snapshot(request("").state).unwrap()).is_some());
    }
    #[test]
    fn immutable_rates_are_request_scoped() {
        let first = request("100 usd in jpy");
        let mut second = first.clone();
        second.state.as_mut().unwrap().data["rates"][1]["rate"] = 200.into();
        assert_eq!(
            calculate(&first.query, snapshot(first.state.clone()).unwrap()).as_deref(),
            Some("15000 JPY")
        );
        assert_eq!(
            calculate(&second.query, snapshot(second.state).unwrap()).as_deref(),
            Some("20000 JPY")
        );
        assert_eq!(
            calculate(&first.query, snapshot(first.state).unwrap()).as_deref(),
            Some("15000 JPY")
        );
    }

    #[test]
    fn zero_limit_skips_state_and_engine() {
        let mut r = request("2^1000000000");
        r.limit = 0;
        r.state = None;
        let bytes = handle_with(&r.encode().unwrap(), |_, _| panic!("must not evaluate")).unwrap();
        assert!(
            matches!(Response::decode(&bytes,&r).unwrap().outcome,Outcome::Results{items,..} if items.is_empty())
        );
    }
    #[test]
    fn state_validation() {
        let r = request("");
        assert!(snapshot(None).is_err());
        let mut state = r.state.clone().unwrap();
        state.version = 2;
        assert!(snapshot(Some(state)).is_err());
        for data in [
            serde_json::json!({}),
            serde_json::json!({"base":"USD","observation_date":"2026-02-30","fixture":true,"rates":[{"code":"USD","rate":1}]}),
        ] {
            assert!(snapshot(Some(State { version: 1, data })).is_err());
        }
        for rate in [0.0, -1.0, 1e13, f64::INFINITY, f64::NAN] {
            let mut s = r.state.clone().unwrap();
            s.data["rates"][1]["rate"] = serde_json::json!(rate);
            assert!(snapshot(Some(s)).is_err());
        }
        for code in ["usd", "XXX", "USDD", ""] {
            let mut s = r.state.clone().unwrap();
            s.data["rates"][1]["code"] = code.into();
            assert!(snapshot(Some(s)).is_err());
        }
        for extra in [
            serde_json::json!({"code":"USD","rate":1}),
            serde_json::json!({"code":"JPY","rate":150}),
        ] {
            let mut s = r.state.clone().unwrap();
            s.data["rates"].as_array_mut().unwrap().push(extra);
            assert!(snapshot(Some(s)).is_err());
        }
        let mut s = r.state.clone().unwrap();
        s.data["rates"][1]["code"] = "USD".into();
        assert!(snapshot(Some(s)).is_err());
        let mut s = r.state.clone().unwrap();
        s.data["extra"] = true.into();
        assert!(snapshot(Some(s)).is_err());
        let mut s = r.state.clone().unwrap();
        s.data["rates"][0]["rate"] = 2.into();
        assert!(snapshot(Some(s)).is_err());
        let mut r = r;
        r.state = None;
        assert!(matches!(
            outcome(&r),
            Outcome::Error {
                code: ErrorCode::InvalidQuery,
                ..
            }
        ));
        r.command = "calculator".into();
        assert!(matches!(
            outcome(&r),
            Outcome::Error {
                code: ErrorCode::UnsupportedCommand,
                ..
            }
        ));
    }
}
