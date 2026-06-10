//! Client-IP-Ermittlung wie im Python-Original (tierlist Vote-Rate-Limit):
//! erstes Element aus `X-Forwarded-For`, sonst die Peer-Adresse.

use axum::http::HeaderMap;
use std::net::SocketAddr;

pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn xff_erstes_element_gewinnt() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.7, 10.0.0.1"),
        );
        assert_eq!(client_ip(&headers, None), "203.0.113.7");
    }

    #[test]
    fn fallback_auf_peer() {
        let headers = HeaderMap::new();
        let peer: SocketAddr = "127.0.0.1:9999".parse().expect("addr");
        assert_eq!(client_ip(&headers, Some(peer)), "127.0.0.1");
    }
}
