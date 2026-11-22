use dupin_plugin_protocol::{
    ErrorCode, Operation, Outcome, PluginAction, PluginItem, ProtocolError, Request, Response, VERSION, View,
};
#[cfg(target_arch = "wasm32")]
use dupin_plugin_protocol::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};
use fend_core::{Context, Interrupt, SpanKind};

struct HostFuel;

impl Interrupt for HostFuel {
    fn should_interrupt(&self) -> bool {
        // Instruction fuel is enforced by the host without giving the guest a clock import.
        false
    }
}

pub fn handle(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    handle_with(bytes, calculate)
}

fn calculate(query: &str) -> String {
    if !complete_input(query) || query.chars().any(char::is_control) {
        return String::new();
    }
    let input = temperature_shorthand(query);
    let mut context = Context::new();
    let Ok(result) = fend_core::evaluate_with_interrupt(&input, &mut context, &HostFuel) else {
        return String::new();
    };
    let output = result.get_main_result();
    let spans: Vec<_> = result.get_main_result_spans().collect();
    if result.output_is_empty()
        || output.len() > 16 * 1024
        || output.trim() == query.trim()
        || output.chars().any(char::is_control)
        || !result
            .get_main_result_spans()
            .any(|span| span.kind() == SpanKind::Number)
        || query.trim().parse::<f64>().is_ok()
        || query.trim().chars().all(|c| c.is_alphabetic() || c == '_')
        || (query.chars().all(|c| c.is_alphabetic() || c.is_whitespace())
            && spans.iter().any(|span| span.kind() == SpanKind::Ident)
            && spans
                .iter()
                .filter(|span| span.kind() == SpanKind::Number)
                .all(|span| span.string() == "1"))
    {
        return String::new();
    }
    output.to_owned()
}

// Context aliases would also replace f/c in unrelated unit expressions.
fn temperature_shorthand(query: &str) -> String {
    let words: Vec<_> = query.split_whitespace().collect();
    if let [value, from, connector, to] = words.as_slice()
        && value.parse::<f64>().is_ok()
        && matches!(*connector, "in" | "to")
        && matches!(*from, "f" | "c" | "F" | "C")
        && matches!(*to, "f" | "c" | "F" | "C")
    {
        return format!(
            "{value} {} {connector} {}",
            from.to_ascii_uppercase(),
            to.to_ascii_uppercase()
        );
    }
    query.to_owned()
}

// Fend accepts missing closing delimiters, which are still unfinished live input.
fn complete_input(input: &str) -> bool {
    let mut chars = input.chars().peekable();
    let mut stack = Vec::new();
    let mut after_number = false;
    let mut word = String::new();
    while let Some(c) = chars.next() {
        if c == '#' {
            if matches!(chars.peek(), Some(' ' | '!')) {
                break;
            }
            if chars.peek() == Some(&'"') {
                chars.next();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '"' && chars.peek() == Some(&'#') {
                        chars.next();
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return false;
                }
                after_number = false;
                continue;
            }
        }
        if matches!(c, '\'' | '"') && !after_number && !matches!(word.as_str(), "to" | "in") {
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '\\' {
                    chars.next();
                } else if next == c {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return false;
            }
            continue;
        }
        match c {
            '(' => stack.push(')'),
            '[' => stack.push(']'),
            '{' => stack.push('}'),
            ')' | ']' | '}' if stack.pop() != Some(c) => return false,
            _ => {}
        }
        if !c.is_whitespace() {
            after_number = c.is_ascii_digit() || matches!(c, ')' | ']' | '}' | '\'' | '"');
            if c.is_alphabetic() {
                word.push(c);
            } else {
                word.clear();
            }
        } else if chars.peek().is_some_and(|next| next.is_alphabetic()) {
            word.clear();
        }
    }
    stack.is_empty()
}

fn handle_with(bytes: &[u8], evaluate: impl FnOnce(&str) -> String) -> Result<Vec<u8>, ProtocolError> {
    let request = Request::decode(bytes)?;
    let outcome = if request.command != "calculator" {
        Outcome::Error {
            code: ErrorCode::UnsupportedCommand,
            message: "This guest only evaluates calculator queries.".into(),
        }
    } else if request.state.is_some() {
        Outcome::Error {
            code: ErrorCode::InvalidQuery,
            message: "Saved state is not supported by this guest.".into(),
        }
    } else if request.limit == 0 || request.query.trim().is_empty() {
        Outcome::Results {
            items: Vec::new(),
            state: None,
        }
    } else {
        let result = evaluate(&request.query);
        if result.is_empty() {
            Outcome::Results {
                items: Vec::new(),
                state: None,
            }
        } else {
            Outcome::Results {
                items: vec![PluginItem {
                    id: "answer".into(),
                    view: View::Pair {
                        source: request.query.clone(),
                        result: result.clone(),
                    },
                    primary: Some("copy-result".into()),
                    actions: vec![PluginAction {
                        id: "copy-result".into(),
                        title: "Copy result".into(),
                        operation: Operation::CopyText { text: result },
                    }],
                }],
                state: None,
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

    #[test]
    fn numeric_acceptance() {
        for (query, expected) in [
            ("12*3", "36"),
            ("2^10", "1024"),
            ("(1+2)/3", "1"),
            ("20% of 150", "30"),
            ("560km to mi", "approx. 347.9678676529 mi"),
            ("100 f in c", "approx. 37.7777777778 °C"),
            ("100 F in C", "approx. 37.7777777778 °C"),
            ("1GiB in MB", "1073.741824 MB"),
            ("2m^2 in cm^2", "20000 cm^2"),
            ("30*(60+1)km/h in m/s", "approx. 508.3333333333 m / s"),
            ("sin pi", "0"),
            (
                "2^200",
                "1606938044258990275541962092341162602522202993782792835301376",
            ),
        ] {
            assert_eq!(calculate(query), expected, "{query}");
        }
    }

    #[test]
    fn scoped_temperature_and_context_isolation() {
        assert_eq!(calculate("1 f"), "1 F");
        assert_eq!(calculate("1 c"), "299792458 m / s");
        assert_eq!(temperature_shorthand("100 f * c"), "100 f * c");
        assert_eq!(temperature_shorthand("100 f in cm"), "100 f in cm");
        assert_eq!(calculate("x = 5; x+1"), "6");
        assert!(calculate("x+1").is_empty());
        assert_eq!(calculate("12*3"), "36");
    }

    #[test]
    fn incomplete_and_non_numeric_results_are_empty() {
        for query in [
            "30*(60+1",
            "(2+3))",
            "2+",
            "1/0",
            "square foot",
            "\"123\"",
            "\"unfinished",
            "2+2\n",
            "roll d6",
        ] {
            assert!(calculate(query).is_empty(), "{query}");
        }
        for query in ["\"(\"", "'('", "#\"(\"#", "1 # (", "5' in m"] {
            assert!(complete_input(query), "{query}");
        }
        assert!(complete_input("1m to '"));
        assert!(calculate("2^60000").is_empty());
    }

    fn request(query: &str) -> Request {
        Request {
            version: VERSION,
            request_id: "request-1".into(),
            generation: "18446744073709551615".into(),
            command: "calculator".into(),
            query: query.into(),
            limit: 1,
            state: None,
        }
    }

    #[test]
    fn calculator_returns_a_correlated_pair() {
        let request = request("30*(60+1)");
        let bytes = handle(&request.encode().unwrap()).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        let Outcome::Results { items, .. } = response.outcome else {
            panic!("expected result");
        };
        assert_eq!(
            items[0].view,
            View::Pair {
                source: request.query,
                result: "1830".into()
            }
        );
        assert_eq!(items[0].primary.as_deref(), Some("copy-result"));
        assert_eq!(
            items[0].actions[0].operation,
            Operation::CopyText { text: "1830".into() }
        );
    }

    #[test]
    fn zero_limit_does_not_evaluate_a_pathological_expression() {
        let mut request = request("2^1000000000");
        request.limit = 0;
        let bytes = handle_with(&request.encode().unwrap(), |_| panic!("must not evaluate")).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        assert!(matches!(response.outcome, Outcome::Results { items, .. } if items.is_empty()));
    }

    #[test]
    fn unknown_commands_have_an_explicit_error() {
        let mut request = request("2+2");
        request.command = "calendar".into();
        let bytes = handle(&request.encode().unwrap()).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        assert!(matches!(
            response.outcome,
            Outcome::Error {
                code: ErrorCode::UnsupportedCommand,
                ..
            }
        ));
    }

    #[test]
    fn unsupported_saved_state_is_not_silently_ignored() {
        let mut request = request("2+2");
        request.state = Some(dupin_plugin_protocol::State {
            version: 1,
            data: Default::default(),
        });
        let bytes = handle_with(&request.encode().unwrap(), |_| panic!("must not evaluate")).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        assert!(matches!(
            response.outcome,
            Outcome::Error {
                code: ErrorCode::InvalidQuery,
                ..
            }
        ));
    }

    #[test]
    fn uninteresting_previews_do_not_become_errors() {
        for query in ["42", "not-maths", "30*(60+1", "sin", "km", "roll d6", "hello"] {
            let request = request(query);
            let bytes = handle(&request.encode().unwrap()).unwrap();
            let response = Response::decode(&bytes, &request).unwrap();
            assert!(matches!(response.outcome, Outcome::Results { items, .. } if items.is_empty()));
        }
    }
}
