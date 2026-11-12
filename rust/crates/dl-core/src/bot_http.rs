use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Client {
    inner: Option<reqwest::Client>,
    total: Option<Duration>,
    failure: Option<std::sync::Arc<str>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Deadlines {
    connect_ms: Option<u64>,
    total_ms: Option<u64>,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self::from_millis(
            Some(crate::settings_catalog::http_defaults::CONNECT_MS),
            Some(crate::settings_catalog::http_defaults::REQUEST_MS),
        )
    }
}

impl Deadlines {
    pub fn from_millis(connect_ms: Option<u64>, total_ms: Option<u64>) -> Self {
        Self {
            connect_ms,
            total_ms,
        }
    }

    pub fn for_operation(timeout: Duration) -> Self {
        let total_ms = u64::try_from(timeout.as_millis())
            .ok()
            .filter(|_| valid_operation(timeout));
        Self::from_millis(
            total_ms.map(|value| value.min(crate::settings_catalog::http_defaults::CONNECT_MS)),
            total_ms,
        )
    }

    pub fn validate(self) -> Result<(), Error> {
        let range = 1..=crate::settings_catalog::http_defaults::MAX_OPERATION_MS;
        match (self.connect_ms, self.total_ms) {
            (Some(connect), Some(total))
                if range.contains(&connect) && range.contains(&total) && connect <= total =>
            {
                Ok(())
            }
            _ => Err(Error::Configuration),
        }
    }

    pub fn steam(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let read = |key, fallback| match lookup(key) {
            None => Some(fallback),
            Some(value) => value
                .trim()
                .parse()
                .ok()
                .filter(|value| crate::runtime_config::STEAM_HTTP_TIMEOUT_MS.contains(value)),
        };
        Self::from_millis(
            read(
                "STEAM_BOT_CONNECT_TIMEOUT_MS",
                crate::settings_catalog::http_defaults::CONNECT_MS,
            ),
            read(
                "STEAM_BOT_REQUEST_TIMEOUT_MS",
                crate::settings_catalog::http_defaults::REQUEST_MS,
            ),
        )
    }
}

pub struct Builder {
    valid: bool,
    inner: reqwest::ClientBuilder,
    connect: Option<Duration>,
    total: Option<Duration>,
}

pub struct RequestBuilder {
    inner: Option<reqwest::RequestBuilder>,
    total: Option<Duration>,
    failure: Option<std::sync::Arc<str>>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP-Zeitlimits sind ungültig. Bitte die Konfiguration prüfen.")]
    Configuration,
    #[error("HTTP-Client konnte nicht initialisiert werden: {0}")]
    Initialization(std::sync::Arc<str>),
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}

impl Error {
    pub fn is_configuration(&self) -> bool {
        matches!(self, Self::Configuration | Self::Initialization(_))
    }
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
            failure: Some(error.to_string().into()),
        }
    }

    pub fn new(deadlines: Deadlines) -> Self {
        Self::builder(deadlines)
            .build()
            .unwrap_or_else(Self::disabled)
    }

    pub fn builder(deadlines: Deadlines) -> Builder {
        Builder {
            valid: deadlines.validate().is_ok(),
            inner: reqwest::Client::builder(),
            connect: deadlines.connect_ms.map(Duration::from_millis),
            total: deadlines.total_ms.map(Duration::from_millis),
        }
    }

    pub fn request(&self, method: reqwest::Method, url: impl reqwest::IntoUrl) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.as_ref().map(|c| c.request(method, url)),
            total: self.total,
            failure: self.failure.clone(),
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
}

impl Builder {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        if valid_operation(timeout) {
            self.total = self.total.map(|_| timeout);
            self.connect = self.connect.map(|connect| connect.min(timeout));
        } else {
            self.total = None;
        }
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
        if !self.valid {
            return Err(Error::Configuration);
        }
        let inner = self.inner.connect_timeout(connect).timeout(total).build()?;
        Ok(Client {
            inner: Some(inner),
            total: Some(total),
            failure: None,
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
    pub fn multipart(self, value: reqwest::multipart::Form) -> Self {
        self.map(|r| r.multipart(value))
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
            inner: match &self.inner {
                Some(inner) => Some(inner.try_clone()?),
                None => None,
            },
            total: self.total,
            failure: self.failure.clone(),
        })
    }
    fn finish(self) -> Result<(reqwest::Client, reqwest::Request), Error> {
        if let Some(failure) = self.failure {
            return Err(Error::Initialization(failure));
        }
        let (Some(inner), Some(total)) = (self.inner, self.total) else {
            return Err(Error::Configuration);
        };
        let (client, request) = inner.build_split();
        let mut request = request?;
        let timeout = request.timeout().copied().unwrap_or(total);
        if !valid_operation(timeout) {
            return Err(Error::Configuration);
        }
        *request.timeout_mut() = Some(timeout);
        Ok((client, request))
    }
    pub fn build(self) -> Result<reqwest::Request, Error> {
        self.finish().map(|(_, request)| request)
    }
    pub async fn send(self) -> Result<reqwest::Response, Error> {
        let (client, request) = self.finish()?;
        Ok(client.execute(request).await?)
    }
}

fn valid_operation(timeout: Duration) -> bool {
    !timeout.is_zero()
        && timeout
            <= Duration::from_millis(crate::settings_catalog::http_defaults::MAX_OPERATION_MS)
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
            assert!(Client::builder(Deadlines::from_millis(connect, total))
                .build()
                .is_err());
        }
        assert!(Client::builder(Deadlines::from_millis(Some(10), Some(50)))
            .build()
            .is_ok());
    }
    #[test]
    fn operation_budgets_and_snapshots_do_not_drift() {
        let first = Client::builder(Deadlines::from_millis(Some(10), Some(50)))
            .build()
            .expect("valid");
        let second = Client::builder(Deadlines::from_millis(Some(10), Some(100)))
            .build()
            .expect("valid");
        assert_eq!(first.total, Some(Duration::from_millis(50)));
        assert_eq!(second.total, Some(Duration::from_millis(100)));
        let longer = first
            .get("http://localhost")
            .timeout(Duration::from_secs(1))
            .build()
            .expect("request");
        assert_eq!(longer.timeout(), Some(&Duration::from_secs(1)));
        let shorter = first
            .get("http://localhost")
            .timeout(Duration::from_millis(5))
            .build()
            .expect("request");
        assert_eq!(shorter.timeout(), Some(&Duration::from_millis(5)));
        assert!(Client::builder(Deadlines::from_millis(Some(51), Some(50)))
            .timeout(Duration::from_millis(5))
            .build()
            .is_err());

        let short = Client::builder(Deadlines::from_millis(Some(10), Some(50)))
            .timeout(Duration::from_millis(5))
            .build()
            .expect("shorter total also bounds connect");
        assert_eq!(short.total, Some(Duration::from_millis(5)));
    }
    #[test]
    fn catalog_bounds_allow_defaults_and_reject_invalid_values() {
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
            assert!(field.nullable);
            for value in [serde_json::Value::Null, serde_json::json!("120000")] {
                assert!(crate::settings_catalog::normalize(
                    &fields,
                    &std::collections::BTreeMap::from([(path.clone(), value)])
                )
                .is_ok());
            }
            for value in [
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
    fn defaults_and_named_operations_are_independent_of_steam() {
        let client = Client::new(Deadlines::default());
        for seconds in [
            crate::settings_catalog::http_defaults::STEAM_INVITE_SECS,
            crate::settings_catalog::http_defaults::STEAM_SYNC_SECS,
        ] {
            let request = client
                .post("http://localhost")
                .timeout(Duration::from_secs(seconds))
                .build()
                .expect("named operation");
            assert_eq!(request.timeout(), Some(&Duration::from_secs(seconds)));
        }
        assert!(Client::builder(Deadlines::steam(|_| None)).build().is_ok());
        for timeout in [Duration::ZERO, Duration::MAX] {
            assert!(Client::builder(Deadlines::for_operation(timeout))
                .build()
                .is_err());
            assert!(client
                .get("http://localhost")
                .timeout(timeout)
                .build()
                .is_err());
        }
        let disabled = Client::disabled(Error::Configuration);
        assert!(disabled
            .get("http://localhost")
            .try_clone()
            .expect("clone preserves error")
            .build()
            .expect_err("disabled")
            .is_configuration());
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
                            && !compact.contains("reqwest::blocking::Client")
                            && !compact.contains("reqwest::get(")
                            && !compact.contains("reqwest::blocking::get(")
                            && !compact.contains("usereqwestas")
                            && !compact.contains("usereqwest::*"),
                        "raw client bypass in {}",
                        path.display()
                    );
                    for import in compact.split("use").filter_map(|s| s.split(';').next()) {
                        assert!(
                            !(import.trim_start_matches(':').starts_with("reqwest::{")
                                && import.contains("Client")),
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
            "crates/dl-brain",
            "bin/dl-knowledge",
        ] {
            scan(&rust.join(name).join("src"));
        }
    }
}
