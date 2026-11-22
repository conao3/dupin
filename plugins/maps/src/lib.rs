use dupin_plugin_protocol::{
    ErrorCode, Operation, Outcome, PluginAction, PluginItem, ProtocolError, Request, Response, VERSION, View,
};
#[cfg(target_arch = "wasm32")]
use dupin_plugin_protocol::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};

pub fn handle(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let request = Request::decode(bytes)?;
    let term = request.query.trim();
    let outcome = if request.command != "search" || request.state.is_some() {
        Outcome::Error {
            code: ErrorCode::InvalidQuery,
            message: "Expected a stateless Maps search.".into(),
        }
    } else {
        let mut items = Vec::new();
        if request.limit > 0 && !term.is_empty() {
            let mut url = String::from("maps://?q=");
            for byte in term.bytes() {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                    url.push(char::from(byte));
                } else {
                    use std::fmt::Write;
                    write!(url, "%{byte:02X}").expect("writing to a string");
                }
            }
            if url.len() <= dupin_plugin_protocol::url_scope::MAX_URL_BYTES {
                items.push(PluginItem {
                    id: "search".into(),
                    view: View::Text {
                        title: term
                            .chars()
                            .scan(0, |bytes, c| {
                                *bytes += c.len_utf8();
                                (*bytes <= 512).then_some(c)
                            })
                            .collect(),
                        subtitle: "Search in Maps".into(),
                    },
                    primary: Some("open-maps".into()),
                    actions: vec![PluginAction {
                        id: "open-maps".into(),
                        title: "Search in Maps".into(),
                        operation: Operation::OpenUrl { url },
                    }],
                });
            }
        }
        Outcome::Results { items, state: None }
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
        Request {
            version: VERSION,
            request_id: "r".into(),
            generation: "42".into(),
            command: "search".into(),
            query: query.into(),
            limit: 1,
            state: None,
        }
    }
    fn items(request: &Request) -> Vec<PluginItem> {
        let bytes = handle(&request.encode().unwrap()).unwrap();
        let response = Response::decode(&bytes, request).unwrap();
        let Outcome::Results { items, state } = response.outcome else {
            panic!("expected results")
        };
        assert!(state.is_none());
        items
    }
    #[test]
    fn query_encoding_and_manifest_contract() {
        for (query, encoded) in [
            ("London", "London"),
            ("  Baker Street  ", "Baker%20Street"),
            ("東京", "%E6%9D%B1%E4%BA%AC"),
            ("&?#%+", "%26%3F%23%25%2B"),
        ] {
            let rows = items(&request(query));
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0].view,
                View::Text {
                    title: query.trim().into(),
                    subtitle: "Search in Maps".into()
                }
            );
            assert_eq!(rows[0].primary.as_deref(), Some("open-maps"));
            assert_eq!(rows[0].actions[0].title, "Search in Maps");
            assert_eq!(
                rows[0].actions[0].operation,
                Operation::OpenUrl {
                    url: format!("maps://?q={encoded}")
                }
            );
        }
    }
    #[test]
    fn bounds_and_empty_queries() {
        for query in ["", "   "] {
            assert!(items(&request(query)).is_empty());
        }
        let mut zero = request("London");
        zero.limit = 0;
        assert!(items(&zero).is_empty());
        assert_eq!(items(&request(&"a".repeat(4096))).len(), 1);
        assert!(items(&request(&"%".repeat(4096))).is_empty());
        assert!(request(&"a".repeat(4097)).encode().is_err());
    }
}
