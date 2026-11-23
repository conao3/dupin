use serde::{Deserialize, Serialize};
use url::Url;
pub const MAX_URL_BYTES: usize = 8192;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "Scope", into = "Scope")]
pub struct UrlScope {
    scheme: String,
    host: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    scheme: String,
    host: Option<String>,
}
impl From<UrlScope> for Scope {
    fn from(s: UrlScope) -> Self {
        Self {
            scheme: s.scheme,
            host: s.host,
        }
    }
}
impl TryFrom<Scope> for UrlScope {
    type Error = &'static str;
    fn try_from(s: Scope) -> Result<Self, Self::Error> {
        Self::new(&s.scheme, s.host.as_deref())
    }
}
impl UrlScope {
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn new(scheme: &str, host: Option<&str>) -> Result<Self, &'static str> {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "dict" | "maps" | "http" | "https") {
            return Err("unsupported URL scheme");
        }
        if matches!(scheme.as_str(), "http" | "https") && host.is_none() {
            return Err("network scope requires hostname");
        }
        let host = host
            .map(|host| {
                if host.is_empty()
                    || !host
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                    || host.ends_with('.')
                {
                    return Err("invalid hostname");
                }
                let parsed = Url::parse(&format!("https://{host}/")).map_err(|_| "invalid hostname")?;
                if parsed.host_str() != Some(host.to_ascii_lowercase().as_str()) {
                    return Err("invalid hostname");
                }
                Ok(host.to_ascii_lowercase())
            })
            .transpose()?;
        Ok(Self { scheme, host })
    }
    pub fn id(&self) -> String {
        format!("open-url:{}:{}", self.scheme, self.host.as_deref().unwrap_or(""))
    }
    pub fn allows(&self, value: &str) -> bool {
        validated_url(value).is_ok_and(|url| {
            url.scheme() == self.scheme
                && self.host.as_ref().is_none_or(|host| {
                    url.host_str()
                        .is_some_and(|actual| actual.eq_ignore_ascii_case(host))
                })
        })
    }
}
pub fn validated_url(value: &str) -> Result<Url, &'static str> {
    if value.is_empty()
        || value.len() > MAX_URL_BYTES
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains('\\')
    {
        return Err("invalid URL");
    }
    let bytes = value.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return Err("invalid URL escape");
        }
    }
    let url = Url::parse(value).map_err(|_| "invalid URL")?;
    let network = matches!(url.scheme(), "http" | "https");
    if !matches!(url.scheme(), "dict" | "maps" | "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("unsafe URL");
    }
    // Network grants require an unambiguous host; Dictionary URLs put escaped terms here.
    let rest = value.split_once(':').ok_or("invalid URL")?.1;
    if let Some(authority) = rest.strip_prefix("//") {
        let authority = authority.split(['/', '?', '#']).next().unwrap_or("");
        if !authority.is_ascii()
            || authority.contains(['@', ':'])
            || (network && (authority.contains('%') || authority.ends_with('.')))
            || url
                .host_str()
                .is_some_and(|host| !host.eq_ignore_ascii_case(authority))
        {
            return Err("ambiguous URL authority");
        }
    } else if network {
        return Err("network URL requires authority");
    }
    if network && url.host_str().is_none() {
        return Err("network URL requires hostname");
    }
    Ok(url)
}
