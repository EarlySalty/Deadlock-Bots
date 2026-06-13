//! Kryptografisch sichere URL-sichere Tokens — Pendant zu Pythons
//! `secrets.token_urlsafe(n)`.
//!
//! Session-IDs und OAuth-`state`-Werte des Dashboards sind im Original
//! `secrets.token_urlsafe(32)`: 32 Zufalls-Bytes, base64url-kodiert ohne
//! Padding (43 Zeichen). Wir bilden exakt das nach — die Länge ist kein
//! Vertrag nach außen (die Werte sind opak), aber dieselbe Entropie und
//! dasselbe Alphabet zu nutzen hält das Verhalten deckungsgleich.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;

/// Erzeugt ein URL-sicheres Token aus `n_bytes` Zufalls-Bytes.
pub fn token_urlsafe(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

/// Standard-Token wie das Dashboard sie vergibt (32 Bytes Entropie).
pub fn session_token() -> String {
    token_urlsafe(32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hat_erwartete_laenge_und_alphabet() {
        let t = session_token();
        // 32 Bytes → ceil(32/3)*4 = 44, minus Padding (=) → 43 Zeichen.
        assert_eq!(t.len(), 43);
        assert!(t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn tokens_sind_eindeutig() {
        let a = session_token();
        let b = session_token();
        assert_ne!(a, b);
    }
}
