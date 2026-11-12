use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Client {
    inner: Option<reqwest::Client>,
    total: Option<Duration>,
}

impl Default for Client {
    fn default() -> Self {
        tracing::error!("HTTP client disabled: configuration or initialization failed; requests will be rejected");
        Self {
            inner: None,
            total: None,
        }
    }
}

pub struct Builder {
    inner: reqwest::ClientBuilder,
    valid: bool,
    connect: Option<Duration>,
    total: Option<Duration>,
}

pub struct RequestBuilder {
    inner: Option<reqwest::RequestBuilder>,
    total: Option<Duration>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP-Zeitlimits fehlen oder sind ungültig. Bitte die Konfiguration prüfen und den Dienst neu starten.")]
    Configuration,
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}

impl Error {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_timeout())
    }
    pub fn is_connect(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_connect())
    }
    pub fn is_request(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_request())
    }
    pub fn is_body(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_body())
    }
    pub fn is_decode(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_decode())
    }
}

impl Client {
    pub fn disabled(error: Error) -> Self {
        tracing::error!(%error, "HTTP client disabled; requests will be rejected");
        Self {
            inner: None,
            total: None,
        }
    }

    pub fn new() -> Self {
        Self::builder().build().unwrap_or_else(Self::disabled)
    }

    pub fn builder() -> Builder {
        Self::builder_with_timeouts(
            crate::runtime_config::lookup("STEAM_BOT_CONNECT_TIMEOUT_MS")
                .and_then(|v| v.parse().ok()),
            crate::runtime_config::lookup("STEAM_BOT_REQUEST_TIMEOUT_MS")
                .and_then(|v| v.parse().ok()),
        )
    }

    pub fn builder_with_timeouts(connect: Option<u64>, total: Option<u64>) -> Builder {
        let bounded = |v: Option<u64>| {
            v.filter(|v| crate::runtime_config::STEAM_HTTP_TIMEOUT_MS.contains(v))
                .map(Duration::from_millis)
        };
        Builder {
            valid: connect
                .zip(total)
                .is_some_and(|(connect, total)| connect <= total),
            inner: reqwest::Client::builder(),
            connect: bounded(connect),
            total: bounded(total),
        }
    }

    pub fn request(&self, method: reqwest::Method, url: impl reqwest::IntoUrl) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.as_ref().map(|c| c.request(method, url)),
            total: self.total,
        }
    }
    pub fn get(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::GET, url)
    }
    pub fn post(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::POST, url)
    }
    pub fn patch(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::PATCH, url)
    }
    pub fn put(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::PUT, url)
    }
    pub fn delete(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::DELETE, url)
    }
    pub fn head(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::HEAD, url)
    }
}

impl Builder {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.total = self.total.map(|total| total.min(timeout));
        self.connect = self
            .connect
            .zip(self.total)
            .map(|(connect, total)| connect.min(total));
        self
    }
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect = self.connect.map(|connect| connect.min(timeout));
        self
    }
    pub fn no_proxy(mut self) -> Self {
        self.inner = self.inner.no_proxy();
        self
    }
    pub fn redirect(mut self, policy: reqwest::redirect::Policy) -> Self {
        self.inner = self.inner.redirect(policy);
        self
    }
    pub fn build(self) -> Result<Client, Error> {
        let (Some(connect), Some(total)) = (self.connect, self.total) else {
            return Err(Error::Configuration);
        };
        if !self.valid || connect > total || total.is_zero() {
            return Err(Error::Configuration);
        }
        let inner = self.inner.connect_timeout(connect).timeout(total).build()?;
        Ok(Client {
            inner: Some(inner),
            total: Some(total),
        })
    }
}

impl RequestBuilder {
    fn map(mut self, f: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder) -> Self {
        self.inner = self.inner.map(f);
        self
    }
    pub fn timeout(self, timeout: Duration) -> Self {
        self.map(|r| r.timeout(timeout))
    }
    pub fn json<T: serde::Serialize + ?Sized>(self, value: &T) -> Self {
        self.map(|r| r.json(value))
    }
    pub fn query<T: serde::Serialize + ?Sized>(self, value: &T) -> Self {
        self.map(|r| r.query(value))
    }
    pub fn form<T: serde::Serialize + ?Sized>(self, value: &T) -> Self {
        self.map(|r| r.form(value))
    }
    pub fn multipart(self, value: reqwest::multipart::Form) -> Self {
        self.map(|r| r.multipart(value))
    }
    pub fn body(self, value: impl Into<reqwest::Body>) -> Self {
        self.map(|r| r.body(value))
    }
    pub fn headers(self, value: reqwest::header::HeaderMap) -> Self {
        self.map(|r| r.headers(value))
    }
    pub fn bearer_auth<T: std::fmt::Display>(self, value: T) -> Self {
        self.map(|r| r.bearer_auth(value))
    }
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        reqwest::header::HeaderName: TryFrom<K>,
        <reqwest::header::HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        reqwest::header::HeaderValue: TryFrom<V>,
        <reqwest::header::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.map(|r| r.header(key, value))
    }
    pub fn try_clone(&self) -> Option<Self> {
        Some(Self {
            inner: Some(self.inner.as_ref()?.try_clone()?),
            total: self.total,
        })
    }
    pub fn build(self) -> Result<reqwest::Request, Error> {
        let (Some(inner), Some(total)) = (self.inner, self.total) else {
            return Err(Error::Configuration);
        };
        let mut request = inner.build()?;
        let timeout = request
            .timeout()
            .copied()
            .map_or(total, |value| value.min(total));
        *request.timeout_mut() = Some(timeout);
        Ok(request)
    }
    pub async fn send(self) -> Result<reqwest::Response, Error> {
        let (Some(inner), Some(total)) = (self.inner, self.total) else {
            tracing::error!(
                "HTTP request rejected: configure valid bot HTTP deadlines and restart the service"
            );
            return Err(Error::Configuration);
        };
        let (client, request) = inner.build_split();
        let mut request = request?;
        let timeout = request
            .timeout()
            .copied()
            .map_or(total, |value| value.min(total));
        *request.timeout_mut() = Some(timeout);
        Ok(client.execute(request).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_and_missing_limits_fail_closed() {
        for (connect, total) in [
            (None, Some(50)),
            (Some(10), None),
            (Some(0), Some(50)),
            (Some(10), Some(u64::MAX)),
            (Some(51), Some(50)),
        ] {
            assert!(Client::builder_with_timeouts(connect, total)
                .build()
                .is_err());
        }
        assert!(Client::builder_with_timeouts(Some(10), Some(50))
            .build()
            .is_ok());
    }
    #[test]
    fn overrides_only_shorten_and_snapshots_do_not_drift() {
        let first = Client::builder_with_timeouts(Some(10), Some(50))
            .build()
            .expect("valid");
        let second = Client::builder_with_timeouts(Some(10), Some(100))
            .build()
            .expect("valid");
        assert_eq!(first.total, Some(Duration::from_millis(50)));
        assert_eq!(second.total, Some(Duration::from_millis(100)));
        let longer = first
            .get("http://localhost")
            .timeout(Duration::from_secs(1))
            .build()
            .expect("request");
        assert_eq!(longer.timeout(), Some(&Duration::from_millis(50)));
        let shorter = first
            .get("http://localhost")
            .timeout(Duration::from_millis(5))
            .build()
            .expect("request");
        assert_eq!(shorter.timeout(), Some(&Duration::from_millis(5)));
        assert!(Client::builder_with_timeouts(Some(51), Some(50))
            .timeout(Duration::from_millis(5))
            .build()
            .is_err());

        let short = Client::builder_with_timeouts(Some(10), Some(50))
            .timeout(Duration::from_millis(5))
            .build()
            .expect("shorter total also bounds connect");
        assert_eq!(short.total, Some(Duration::from_millis(5)));
    }
    #[test]
    fn catalog_and_parser_share_bounds_and_reject_null_reset() {
        let fields = crate::settings_catalog::fields(include_str!("settings_catalog.tsv"), &[]);
        for name in ["steam_connect_timeout_ms", "steam_request_timeout_ms"] {
            let path = format!("runtime.bridges.{name}");
            let field = fields.iter().find(|f| f.path == path).expect("registered");
            assert_eq!(
                field.min,
                Some(
                    crate::runtime_config::STEAM_HTTP_TIMEOUT_MS
                        .start()
                        .to_string()
                )
            );
            assert_eq!(
                field.max,
                Some(
                    crate::runtime_config::STEAM_HTTP_TIMEOUT_MS
                        .end()
                        .to_string()
                )
            );
            assert!(!field.nullable);
            for value in [
                serde_json::Value::Null,
                serde_json::json!(""),
                serde_json::json!("0"),
                serde_json::json!("18446744073709551616"),
            ] {
                assert!(crate::settings_catalog::normalize(
                    &fields,
                    &std::collections::BTreeMap::from([(path.clone(), value)])
                )
                .is_err());
            }
        }
    }

    #[test]
    fn bot_crates_cannot_construct_raw_http_clients() {
        fn scan(path: &std::path::Path) {
            for entry in std::fs::read_dir(path).expect("source directory") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    scan(&path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).expect("source text");
                    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                    assert!(
                        !compact.contains("reqwest::Client")
                            && !compact.contains("reqwest::blocking::Client"),
                        "raw client bypass in {}",
                        path.display()
                    );
                    for import in compact.split("use").filter_map(|s| s.split(';').next()) {
                        assert!(
                            !(import.starts_with("reqwest::{") && import.contains("Client")),
                            "aliased raw client in {}",
                            path.display()
                        );
                    }
                }
            }
        }
        let rust = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for name in [
            "bin/dl-bot",
            "crates/dl-bridges",
            "crates/dl-voice",
            "crates/dl-ai",
            "crates/dl-community",
        ] {
            scan(&rust.join(name).join("src"));
        }
    }
}
