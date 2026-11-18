//! A small, blocking client for the [Argoshot](https://argoshot.com) screenshot API
//! (<https://argoshot.com/docs>).
//!
//! ```no_run
//! use argoshot::{Client, Options};
//!
//! let client = Client::new("ACCESS_KEY").with_secret_key("SECRET_KEY");
//! let png = client.take(&Options::new().url("https://example.com").format("png"))?;
//! std::fs::write("example.png", png)?;
//!
//! let og_image = client.url(&Options::new().url("https://example.com").viewport_width(1200).viewport_height(630));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Options are the API's own names (<https://argoshot.com/docs/options>): the common ones have typed setters,
//! any other is set by name with [`Options::set`].

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The API's address.
pub const DEFAULT_BASE_URL: &str = "https://api.argoshot.com";
/// The largest answer read (a full-page screenshot can be large).
const MAX_BODY: u64 = 256 * 1024 * 1024;

/// A request's options, by the API's names. Lists are JSON arrays; `null` values are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options(BTreeMap<String, Value>);

impl Options {
    pub fn new() -> Self {
        Self::default()
    }

    /// Any option by its API name: `.set("hide_selectors", vec![".banner", "#chat"])`, `.set("delay", 2)`.
    pub fn set(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.0.insert(name.to_string(), value.into());
        self
    }

    /// The page to render (http or https).
    pub fn url(self, url: &str) -> Self { self.set("url", url) }
    /// HTML to render (take() sends it as a JSON POST).
    pub fn html(self, html: &str) -> Self { self.set("html", html) }
    /// Markdown to render, as a page (take() sends it as a JSON POST).
    pub fn markdown(self, markdown: &str) -> Self { self.set("markdown", markdown) }
    /// png, jpeg (the default jpg), webp, gif, avif, pdf, html, markdown ...
    pub fn format(self, format: &str) -> Self { self.set("format", format) }
    pub fn viewport_width(self, px: u32) -> Self { self.set("viewport_width", px) }
    pub fn viewport_height(self, px: u32) -> Self { self.set("viewport_height", px) }
    /// A device preset (GET /devices lists them).
    pub fn viewport_device(self, device: &str) -> Self { self.set("viewport_device", device) }
    pub fn full_page(self, on: bool) -> Self { self.set("full_page", on) }
    pub fn block_cookie_banners(self, on: bool) -> Self { self.set("block_cookie_banners", on) }
    pub fn block_ads(self, on: bool) -> Self { self.set("block_ads", on) }
    pub fn dark_mode(self, on: bool) -> Self { self.set("dark_mode", on) }
    /// Capture one element (CSS selector).
    pub fn selector(self, selector: &str) -> Self { self.set("selector", selector) }
    /// Keep the result and answer identical requests from it.
    pub fn cache(self, on: bool) -> Self { self.set("cache", on) }
    /// Seconds the cached result is kept (needs cache).
    pub fn cache_ttl(self, seconds: u32) -> Self { self.set("cache_ttl", seconds) }

    /// The options a request sends: nulls and the client's own names (access_key, signature) left out.
    fn cleaned(&self) -> BTreeMap<&str, &Value> {
        self.0
            .iter()
            .filter(|(k, v)| !v.is_null() && k.as_str() != "access_key" && k.as_str() != "signature")
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }
}

/// An error the API answered (its `error_code` and `error_message`), or a response that was not the API's
/// (code `http_error`).
#[derive(Clone, Debug, PartialEq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub status: u16,
    pub documentation_url: Option<String>,
    pub upgrade_url: Option<String>,
    pub returned_status_code: Option<i64>,
    pub matched_string: Option<String>,
    /// Seconds, from Retry-After.
    pub retry_after: Option<u64>,
    pub body: String,
}

/// What a call can fail with.
#[derive(Debug)]
pub enum Error {
    /// The API answered an error.
    Api(Box<ApiError>),
    /// No answer: connection, TLS, timeout, or a body that could not be read.
    Transport(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api(e) => write!(f, "argoshot: {}: {}", e.code, e.message),
            Error::Transport(e) => write!(f, "argoshot: request failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// The API's error code, when the API answered one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Api(e) => Some(&e.code),
            Error::Transport(_) => None,
        }
    }
}

/// A request as the client sends it (method, URL, headers, body).
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
}

/// A client for the Argoshot API (blocking, on `ureq`).
#[derive(Clone)]
pub struct Client {
    access_key: String,
    secret_key: Option<String>,
    base_url: String,
    agent: ureq::Agent,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("base_url", &self.base_url).field("signed", &self.secret_key.is_some()).finish()
    }
}

impl Client {
    /// A client with your access key (timeout 120 s).
    pub fn new(access_key: &str) -> Self {
        Client {
            access_key: access_key.to_string(),
            secret_key: None,
            base_url: DEFAULT_BASE_URL.to_string(),
            agent: agent(Duration::from_secs(120)),
        }
    }

    /// Sign url() links and GET take() requests with this secret key.
    pub fn with_secret_key(mut self, secret_key: &str) -> Self {
        self.secret_key = if secret_key.is_empty() { None } else { Some(secret_key.to_string()) };
        self
    }

    /// The API's address (default <https://api.argoshot.com>).
    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    /// The whole request's timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.agent = agent(timeout);
        self
    }

    /// A GET /take URL for these options (for `<img src>` or og:image), signed when a secret key is set.
    /// The access key is in the URL: for public pages, use a key set to "signed requests only".
    pub fn url(&self, options: &Options) -> String {
        let mut query = build_query(options);
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str("access_key=");
        query.push_str(&encode(&self.access_key));
        if let Some(secret) = &self.secret_key {
            let signature = sign(secret, &query);
            query.push_str("&signature=");
            query.push_str(&signature);
        }
        format!("{}/take?{}", self.base_url, query)
    }

    /// The request take() sends for these options: GET (signed when a secret key is set) for url captures, a JSON
    /// POST when html or markdown is given.
    pub fn prepare_take(&self, options: &Options) -> Prepared {
        let opts = options.cleaned();
        let mut headers = vec![("X-Access-Key", self.access_key.clone()), ("User-Agent", format!("argoshot-rust/{VERSION}"))];
        if opts.contains_key("html") || opts.contains_key("markdown") {
            headers.push(("Content-Type", "application/json".to_string()));
            let body = serde_json::to_string(&opts).expect("options are JSON");
            return Prepared { method: "POST", url: format!("{}/take", self.base_url), headers, body: Some(body) };
        }
        let mut query = build_query(options);
        if let Some(secret) = &self.secret_key {
            let signature = sign(secret, &query);
            query.push_str("&signature=");
            query.push_str(&signature);
        }
        Prepared { method: "GET", url: format!("{}/take?{}", self.base_url, query), headers, body: None }
    }

    /// Render and return the file's bytes.
    pub fn take(&self, options: &Options) -> Result<Vec<u8>, Error> {
        self.send(self.prepare_take(options))
    }

    /// GET /usage: renders total, available and used, and the plan's limits, as JSON. Costs nothing.
    pub fn usage(&self) -> Result<Value, Error> {
        let prepared = Prepared {
            method: "GET",
            url: format!("{}/usage", self.base_url),
            headers: vec![("X-Access-Key", self.access_key.clone()), ("User-Agent", format!("argoshot-rust/{VERSION}"))],
            body: None,
        };
        let body = self.send(prepared)?;
        serde_json::from_slice(&body).map_err(|e| Error::Transport(format!("/usage is not JSON: {e}")))
    }

    fn send(&self, p: Prepared) -> Result<Vec<u8>, Error> {
        let transport = |e: ureq::Error| Error::Transport(e.to_string());
        let mut response = match p.body {
            Some(body) => {
                let mut request = self.agent.post(&p.url);
                for (name, value) in &p.headers {
                    request = request.header(*name, value);
                }
                request.send(body).map_err(transport)?
            }
            None => {
                let mut request = self.agent.get(&p.url);
                for (name, value) in &p.headers {
                    request = request.header(*name, value);
                }
                request.call().map_err(transport)?
            }
        };
        let status = response.status().as_u16();
        let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(transport)?;
        if !(200..300).contains(&status) {
            return Err(Error::Api(Box::new(parse_error(status, &String::from_utf8_lossy(&bytes), retry_after.as_deref()))));
        }
        Ok(bytes)
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(timeout)).build().into()
}

/// An [`ApiError`] from an error response's status, body and Retry-After header.
pub fn parse_error(status: u16, body: &str, retry_after: Option<&str>) -> ApiError {
    let retry_after = retry_after.and_then(|r| r.trim().parse::<u64>().ok());
    let text = |v: &Value, name: &str| v.get(name).and_then(Value::as_str).map(str::to_string);
    if let Ok(data) = serde_json::from_str::<Value>(body) {
        if let Some(code) = data.get("error_code").and_then(Value::as_str) {
            return ApiError {
                code: code.to_string(),
                message: text(&data, "error_message").unwrap_or_else(|| code.to_string()),
                status,
                documentation_url: text(&data, "documentation_url"),
                upgrade_url: text(&data, "upgrade_url"),
                returned_status_code: data.get("returned_status_code").and_then(Value::as_i64),
                matched_string: text(&data, "matched_string"),
                retry_after,
                body: body.to_string(),
            };
        }
    }
    let snippet: String = body.chars().take(200).collect();
    ApiError {
        code: "http_error".to_string(),
        message: if snippet.is_empty() { format!("HTTP {status}") } else { format!("HTTP {status}: {snippet}") },
        status,
        documentation_url: None,
        upgrade_url: None,
        returned_status_code: None,
        matched_string: None,
        retry_after,
        body: body.to_string(),
    }
}

/// The query string of these options: names sorted, lists repeated, nulls left out (no access key).
pub fn build_query(options: &Options) -> String {
    let mut parts = Vec::new();
    for (name, value) in options.cleaned() {
        let values: Vec<&Value> = match value {
            Value::Array(items) => items.iter().filter(|v| !v.is_null()).collect(),
            other => vec![other],
        };
        for v in values {
            parts.push(format!("{}={}", encode(name), encode(&text(v))));
        }
    }
    parts.join("&")
}

/// The signature the API checks: lowercase hex HMAC-SHA256 of the query string, under the secret key.
pub fn sign(secret_key: &str, query: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret_key.as_bytes()).expect("HMAC takes any key length");
    mac.update(query.as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// RFC 3986: everything but A-Z a-z 0-9 - . _ ~ is percent-encoded (a space is %20, a + is %2B).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The server's own test (jai-web-framework framework/Screenshots/tests/privacy/app.jai:194):
    // hmac_sha256("secret-of-the-customer-1234", "html=x&access_key=key-<tenant>"), lowercase hex.
    #[test]
    fn server_vector() {
        let client = Client::new("key-2").with_secret_key("secret-of-the-customer-1234");
        assert_eq!(
            client.url(&Options::new().html("x")),
            "https://api.argoshot.com/take?html=x&access_key=key-2&signature=9488e5f178e81282b921f410775de15b0767f894504cd18bd66f35616d60ffd1"
        );
    }

    #[test]
    fn signed_url() {
        let client = Client::new("AK_test").with_secret_key("SK_test");
        let options = Options::new()
            .url("https://example.com/a b?x=1&y=2")
            .format("png")
            .full_page(true)
            .viewport_width(1280)
            .set("device_scale_factor", 1.5)
            .set("hide_selectors", vec![".a", "#b .c"]);
        assert_eq!(
            client.url(&options),
            "https://api.argoshot.com/take?device_scale_factor=1.5&format=png&full_page=true&hide_selectors=.a&hide_selectors=%23b%20.c&url=https%3A%2F%2Fexample.com%2Fa%20b%3Fx%3D1%26y%3D2&viewport_width=1280&access_key=AK_test&signature=1a6f9d69eae82c57575f031ba76325705f5c0ddc6b48380a6fa7964c8a497b1c"
        );
    }

    #[test]
    fn non_ascii_html() {
        let client = Client::new("AK_test").with_secret_key("SK_test");
        assert_eq!(
            client.url(&Options::new().html("<h1>Hi — café + &</h1>").format("png")),
            "https://api.argoshot.com/take?format=png&html=%3Ch1%3EHi%20%E2%80%94%20caf%C3%A9%20%2B%20%26%3C%2Fh1%3E&access_key=AK_test&signature=a1fb4eea53e2c394dce7ab32e31a26a2c8c85b8ebc53ac330ddb0cc441ee9eb7"
        );
    }

    #[test]
    fn unsigned_custom_base_null_skipped() {
        let client = Client::new("AK_test").with_base_url("http://localhost:8080/");
        let options = Options::new()
            .url("https://example.com")
            .format("png")
            .set("selector", Value::Null)
            .set("access_key", "other")
            .set("signature", "x");
        assert_eq!(client.url(&options), "http://localhost:8080/take?format=png&url=https%3A%2F%2Fexample.com&access_key=AK_test");
    }

    #[test]
    fn reserved_characters() {
        assert_eq!(build_query(&Options::new().set("styles", "a{content:'!()*'}")), "styles=a%7Bcontent%3A%27%21%28%29%2A%27%7D");
    }

    #[test]
    fn parse_api_error() {
        let e = parse_error(400, r#"{"is_successful":false,"error_code":"selector_not_found","error_message":"Nothing matches the selector: #hero","documentation_url":"https://argoshot.com/docs/errors/selector-not-found/"}"#, None);
        assert_eq!(e.code, "selector_not_found");
        assert_eq!(e.message, "Nothing matches the selector: #hero");
        assert_eq!(e.status, 400);
        assert_eq!(e.documentation_url.as_deref(), Some("https://argoshot.com/docs/errors/selector-not-found/"));
        let e = parse_error(402, r#"{"is_successful":false,"error_code":"free_tier_exhausted","error_message":"Used.","upgrade_url":"https://argoshot.com/pricing"}"#, Some("5"));
        assert_eq!(e.upgrade_url.as_deref(), Some("https://argoshot.com/pricing"));
        assert_eq!(e.retry_after, Some(5));
        let bad = parse_error(502, "<html>Bad gateway</html>", None);
        assert_eq!(bad.code, "http_error");
        assert!(bad.message.contains("HTTP 502"));
    }

    #[test]
    fn prepare_get_signed() {
        let client = Client::new("AK_test").with_secret_key("SK_test");
        let p = client.prepare_take(&Options::new().url("https://example.com").format("png"));
        let query = "format=png&url=https%3A%2F%2Fexample.com";
        assert_eq!(p.method, "GET");
        assert_eq!(p.url, format!("https://api.argoshot.com/take?{query}&signature={}", sign("SK_test", query)));
        assert!(p.headers.contains(&("X-Access-Key", "AK_test".to_string())));
        assert_eq!(p.body, None);
    }

    #[test]
    fn prepare_post_for_html() {
        let client = Client::new("AK_test");
        let p = client.prepare_take(&Options::new().html("<h1>Hi</h1>").format("png").full_page(true).set("hide_selectors", vec![".x"]));
        assert_eq!(p.method, "POST");
        assert_eq!(p.url, "https://api.argoshot.com/take");
        assert!(p.headers.contains(&("Content-Type", "application/json".to_string())));
        let body: Value = serde_json::from_str(p.body.as_deref().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"html": "<h1>Hi</h1>", "format": "png", "full_page": true, "hide_selectors": [".x"]}));
    }
}
