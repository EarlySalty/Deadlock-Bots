//! Zugriffs-Guards und Redirect-Validierung für die Auth-Provider-Routen.
//!
//! Hier liegt die **reine** Sicherheitslogik (Token-Prüfung, erlaubte
//! Weiterleitungsziele) getrennt von der axum-Schicht — so ist sie ohne
//! HTTP-Server testbar. Status-Codes und Reihenfolge der Prüfungen sind
//! deckungsgleich zu `service/dashboard.py`.

use url::Url;

/// Ablehnungsgründe der internen Token-Guards mit ihren HTTP-Codes (wie im
/// Original: 403 loopback, 401 Token, 503 nicht konfiguriert).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalReject {
    NotLoopback,
    MissingOrInvalidToken,
    NotConfigured,
}

impl InternalReject {
    pub fn status(self) -> u16 {
        match self {
            InternalReject::NotLoopback => 403,
            InternalReject::MissingOrInvalidToken => 401,
            InternalReject::NotConfigured => 503,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            InternalReject::NotLoopback => "loopback access required",
            InternalReject::MissingOrInvalidToken => "missing or invalid X-Internal-Token",
            InternalReject::NotConfigured => "internal api token not configured",
        }
    }
}

/// `_require_any_internal_api_access`: loopback, dann Token vorhanden, dann
/// mind. ein Token konfiguriert, dann Treffer (turnier ODER twitch).
pub fn check_internal_any(
    is_loopback: bool,
    presented: Option<&str>,
    turnier_tokens: &[String],
    twitch_tokens: &[String],
) -> Result<(), InternalReject> {
    if !is_loopback {
        return Err(InternalReject::NotLoopback);
    }
    let presented = presented.map(str::trim).filter(|t| !t.is_empty());
    if presented.is_none() {
        return Err(InternalReject::MissingOrInvalidToken);
    }
    let configured = !turnier_tokens.is_empty() || !twitch_tokens.is_empty();
    if !configured {
        return Err(InternalReject::NotConfigured);
    }
    let token = presented.unwrap_or_default();
    let matches = turnier_tokens
        .iter()
        .chain(twitch_tokens)
        .any(|t| t == token);
    if matches {
        Ok(())
    } else {
        Err(InternalReject::MissingOrInvalidToken)
    }
}

/// `_require_turnier/twitch_internal_api_access`: loopback, dann konfiguriert
/// (503), dann Token-Treffer (401).
pub fn check_internal_specific(
    is_loopback: bool,
    presented: Option<&str>,
    tokens: &[String],
) -> Result<(), InternalReject> {
    if !is_loopback {
        return Err(InternalReject::NotLoopback);
    }
    if tokens.is_empty() {
        return Err(InternalReject::NotConfigured);
    }
    let token = presented.map(str::trim).filter(|t| !t.is_empty());
    match token {
        Some(token) if tokens.iter().any(|t| t == token) => Ok(()),
        _ => Err(InternalReject::MissingOrInvalidToken),
    }
}

/// `_is_allowed_redirect_after`: Loopback erlaubt http/https, sonst nur https
/// und ausschließlich die beiden Community-Hosts. Keine Userinfo im URL.
pub fn is_allowed_redirect_after(value: &str) -> bool {
    let candidate = value.trim();
    if candidate.is_empty() {
        return false;
    }
    let Ok(parsed) = Url::parse(candidate) else {
        return false;
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_lowercase();
    let scheme = parsed.scheme().to_lowercase();
    if matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return scheme == "http" || scheme == "https";
    }
    if scheme != "https" {
        return false;
    }
    matches!(
        host.as_str(),
        "deutsche-deadlock-community.de" | "admin.deutsche-deadlock-community.de"
    )
}

/// `_safe_template_href`: relative Pfade (eine führende `/`) und erlaubte
/// absolute URLs werden durchgereicht, sonst der Fallback. Schützt vor
/// Open-Redirects und `javascript:`-Schemata.
pub fn safe_href(value: &str, fallback: &str) -> String {
    let candidate = value.trim();
    if candidate.is_empty() {
        return fallback.to_string();
    }
    if candidate.starts_with('/') && !candidate.starts_with("//") {
        return candidate.to_string();
    }
    if is_allowed_redirect_after(candidate) {
        return candidate.to_string();
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn internal_any_pruefreihenfolge() {
        let turnier = tokens(&["T"]);
        let twitch = tokens(&["W"]);
        // Nicht loopback → 403, vor allen Token-Prüfungen.
        assert_eq!(
            check_internal_any(false, Some("T"), &turnier, &twitch),
            Err(InternalReject::NotLoopback)
        );
        // Kein Token → 401.
        assert_eq!(
            check_internal_any(true, None, &turnier, &twitch),
            Err(InternalReject::MissingOrInvalidToken)
        );
        // Keine Tokens konfiguriert → 503.
        assert_eq!(
            check_internal_any(true, Some("x"), &[], &[]),
            Err(InternalReject::NotConfigured)
        );
        // Treffer in einer der Ketten → ok.
        assert!(check_internal_any(true, Some("W"), &turnier, &twitch).is_ok());
        // Falscher Token → 401.
        assert_eq!(
            check_internal_any(true, Some("nope"), &turnier, &twitch),
            Err(InternalReject::MissingOrInvalidToken)
        );
    }

    #[test]
    fn internal_specific_503_vor_401() {
        // Nicht konfiguriert (503) wird vor dem Token-Treffer geprüft.
        assert_eq!(
            check_internal_specific(true, Some("x"), &[]),
            Err(InternalReject::NotConfigured)
        );
        let toks = tokens(&["secret"]);
        assert!(check_internal_specific(true, Some("secret"), &toks).is_ok());
        assert_eq!(
            check_internal_specific(true, Some(" "), &toks),
            Err(InternalReject::MissingOrInvalidToken)
        );
    }

    #[test]
    fn redirect_after_regeln() {
        assert!(is_allowed_redirect_after(
            "https://deutsche-deadlock-community.de/x"
        ));
        assert!(is_allowed_redirect_after(
            "https://admin.deutsche-deadlock-community.de/admin"
        ));
        assert!(is_allowed_redirect_after("http://127.0.0.1:8766/cb"));
        assert!(is_allowed_redirect_after("http://localhost/cb"));
        // Fremder Host → nein.
        assert!(!is_allowed_redirect_after("https://evil.example/cb"));
        // Nicht-loopback http → nein.
        assert!(!is_allowed_redirect_after(
            "http://deutsche-deadlock-community.de/x"
        ));
        // Userinfo → nein.
        assert!(!is_allowed_redirect_after(
            "https://u:p@deutsche-deadlock-community.de/x"
        ));
        // Relativ/leer/Müll → nein.
        assert!(!is_allowed_redirect_after("/admin"));
        assert!(!is_allowed_redirect_after(""));
        assert!(!is_allowed_redirect_after("javascript:alert(1)"));
    }

    #[test]
    fn safe_href_fallback() {
        assert_eq!(safe_href("/admin", "/x"), "/admin");
        assert_eq!(safe_href("", "/x"), "/x");
        // Protocol-relative (Open-Redirect) → Fallback.
        assert_eq!(safe_href("//evil.example", "/x"), "/x");
        assert_eq!(safe_href("javascript:alert(1)", "/x"), "/x");
        assert_eq!(
            safe_href("https://deutsche-deadlock-community.de/y", "/x"),
            "https://deutsche-deadlock-community.de/y"
        );
        assert_eq!(safe_href("https://evil.example", "/x"), "/x");
    }
}
