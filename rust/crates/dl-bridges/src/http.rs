use std::time::Duration;

use dl_core::runtime_config::{
    BridgeOptions, HTTP_DEFAULT_CONNECT_TIMEOUT_MS, HTTP_DEFAULT_REQUEST_TIMEOUT_MS,
};

pub const RETRY_AFTER_FALLBACK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpLimits {
    pub connect: Duration,
    pub request: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpSetupError {
    #[error("Betriebskonfiguration für HTTP-Zeitlimits nicht lesbar: {0}")]
    Config(String),
    #[error("HTTP-Client konnte nicht gebaut werden: {0}")]
    Build(#[source] reqwest::Error),
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self::from_bridges(&BridgeOptions::default())
    }
}

impl HttpLimits {
    pub fn from_bridges(bridges: &BridgeOptions) -> Self {
        Self {
            connect: Duration::from_millis(
                bridges
                    .http_connect_timeout_ms
                    .unwrap_or(HTTP_DEFAULT_CONNECT_TIMEOUT_MS),
            ),
            request: Duration::from_millis(
                bridges
                    .http_request_timeout_ms
                    .unwrap_or(HTTP_DEFAULT_REQUEST_TIMEOUT_MS),
            ),
        }
    }

    pub fn current() -> Result<Self, HttpSetupError> {
        let process = dl_core::config::process_bot_config()
            .map_err(|err| HttpSetupError::Config(err.to_string()))?;
        Ok(Self::from_bridges(&process.snapshot().runtime.bridges))
    }

    pub fn with_request(self, request: Duration) -> Self {
        Self {
            connect: self.connect.min(request),
            request,
        }
    }
}

pub fn rate_limit_wait(
    retry_after_secs: Option<f64>,
    waited: Duration,
    budget: Duration,
) -> Option<Duration> {
    let wait = retry_after_secs.map_or(RETRY_AFTER_FALLBACK, |seconds| {
        dl_core::duration::retry_after(seconds, Duration::MAX)
    });
    (waited.saturating_add(wait) <= budget).then_some(wait)
}

pub fn client(limits: HttpLimits) -> Result<reqwest::Client, HttpSetupError> {
    reqwest::Client::builder()
        .connect_timeout(limits.connect)
        .timeout(limits.request)
        .build()
        .map_err(HttpSetupError::Build)
}

pub fn internal_client(limits: HttpLimits) -> Result<reqwest::Client, HttpSetupError> {
    reqwest::Client::builder()
        .connect_timeout(limits.connect)
        .timeout(limits.request)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(HttpSetupError::Build)
}

pub fn current_client() -> Result<reqwest::Client, HttpSetupError> {
    client(HttpLimits::current()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grenzen_kommen_aus_der_konfiguration_oder_den_benannten_standards() {
        let defaults = HttpLimits::from_bridges(&BridgeOptions::default());
        assert_eq!(
            defaults.connect,
            Duration::from_millis(HTTP_DEFAULT_CONNECT_TIMEOUT_MS)
        );
        assert_eq!(
            defaults.request,
            Duration::from_millis(HTTP_DEFAULT_REQUEST_TIMEOUT_MS)
        );
        let configured = HttpLimits::from_bridges(&BridgeOptions {
            http_connect_timeout_ms: Some(7),
            http_request_timeout_ms: Some(11),
            ..BridgeOptions::default()
        });
        assert_eq!(configured.connect, Duration::from_millis(7));
        assert_eq!(configured.request, Duration::from_millis(11));
        let shorter = configured.with_request(Duration::from_millis(5));
        assert_eq!(shorter.connect, Duration::from_millis(5));
    }

    #[test]
    fn rate_limit_wait_refuses_what_the_budget_cannot_hold() {
        let budget = Duration::from_secs(10);
        for seconds in [f64::INFINITY, 1e300, 1e20, 11.0] {
            assert_eq!(
                rate_limit_wait(Some(seconds), Duration::ZERO, budget),
                None,
                "{seconds}"
            );
        }
        assert_eq!(
            rate_limit_wait(Some(f64::NAN), Duration::ZERO, budget),
            Some(Duration::ZERO)
        );
        assert_eq!(
            rate_limit_wait(Some(-2.0), Duration::ZERO, budget),
            Some(Duration::ZERO)
        );
        assert_eq!(
            rate_limit_wait(None, Duration::ZERO, budget),
            Some(RETRY_AFTER_FALLBACK)
        );
        assert_eq!(
            rate_limit_wait(Some(4.0), Duration::from_secs(6), budget),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            rate_limit_wait(Some(4.0), Duration::from_secs(7), budget),
            None
        );
    }

    #[tokio::test]
    async fn anfrage_bricht_am_zeitlimit_ab() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Testport");
        let address = listener.local_addr().expect("Testadresse");
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("Verbindung");
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let limits = HttpLimits {
            connect: Duration::from_millis(200),
            request: Duration::from_millis(200),
        };
        for http in [
            client(limits).expect("Client"),
            internal_client(limits).expect("Client"),
        ] {
            let started = std::time::Instant::now();
            let error = http
                .get(format!("http://{address}/"))
                .send()
                .await
                .expect_err("Zeitlimit");
            assert!(error.is_timeout(), "{error}");
            assert!(started.elapsed() < Duration::from_secs(5));
        }
        server.abort();
    }
}
