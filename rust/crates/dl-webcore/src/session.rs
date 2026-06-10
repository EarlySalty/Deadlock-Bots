//! HMAC-signierte Session-Cookies, byte-kompatibel zu `service/public_stats.py`.
//!
//! Format: `<base64url(payload-json, ohne Padding)>.<hmac-sha256-hex>`
//! - Payload-JSON: kompakt (`,`/`:`), Schlüssel SORTIERT, UTF-8 unescaped —
//!   exakt wie Pythons `json.dumps(..., ensure_ascii=False,
//!   separators=(",",":"), sort_keys=True)`.
//! - HMAC über den encodierten (unge­paddeten) Base64-String, Secret als UTF-8.
//!
//! Beide Welten müssen während der Migration gegenseitig Cookies verifizieren
//! können — der Interop-Test unten enthält eine von Python erzeugte Fixture.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

/// Signiert und verifiziert Session-Payloads. Ohne Secret ist jede
/// Verifikation negativ und Signieren unmöglich (wie im Original).
#[derive(Clone)]
pub struct SessionCodec {
    secret: Option<String>,
}

impl SessionCodec {
    pub fn new(secret: Option<String>) -> Self {
        let secret = secret
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self { secret }
    }

    pub fn has_secret(&self) -> bool {
        self.secret.is_some()
    }

    /// Signiert ein FLACHES Payload (Strings/Zahlen/null — Session-Payloads
    /// sind flach; verschachtelte Objekte würden die Schlüssel-Sortierung
    /// nicht garantieren).
    pub fn sign(&self, payload: &BTreeMap<String, serde_json::Value>) -> Option<String> {
        let secret = self.secret.as_ref()?;
        // BTreeMap serialisiert deterministisch sortiert — unabhängig von
        // serde_json-Feature-Flags (preserve_order) anderer Crates.
        let body = serde_json::to_string(payload).ok()?;
        let encoded = URL_SAFE_NO_PAD.encode(body.as_bytes());
        let sig = hmac_hex(secret, &encoded);
        Some(format!("{encoded}.{sig}"))
    }

    /// Verifiziert Signatur + Ablauf (`exp` <= now → ungültig) und liefert
    /// das Payload zurück.
    pub fn verify(
        &self,
        cookie: &str,
        now_unix: i64,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let secret = self.secret.as_ref()?;
        let (encoded, signature) = cookie.rsplit_once('.')?;
        if encoded.is_empty() || signature.is_empty() {
            return None;
        }

        // Konstante-Zeit-Vergleich über die rohen MAC-Bytes.
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
        mac.update(encoded.as_bytes());
        let presented = hex::decode(signature).ok()?;
        mac.verify_slice(&presented).ok()?;

        // Python strippt Padding beim Encodieren; defensiv auch hier strippen.
        let raw = URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('=')).ok()?;
        let payload: serde_json::Value = serde_json::from_slice(&raw).ok()?;
        let obj = payload.as_object()?.clone();

        let exp = match obj.get("exp") {
            Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?,
            None => 0,
        };
        if exp <= now_unix {
            return None;
        }
        Some(obj)
    }
}

fn hmac_hex(secret: &str, value: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC akzeptiert beliebige Schlüssellängen");
    mac.update(value.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const SECRET: &str = "test-secret-123";
    /// Von service/public_stats.py (_sign) mit SECRET erzeugt — Interop-Beweis.
    const PYTHON_COOKIE: &str = "eyJhdmF0YXIiOm51bGwsImV4cCI6NDEwMjQ0NDgwMCwiaWF0IjoxNzE4MDAwMDAwLCJuYW1lIjoiVMO8c3RlciIsInVzZXJfaWQiOiIxMjM0NTY3ODkifQ.f7ef4d5be5b49fd4e31bf7ec23e2e161218a1f822e43351f2f3d32b3dead51c6";

    fn payload() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("user_id".to_string(), json!("123456789")),
            ("name".to_string(), json!("Tüster")),
            ("avatar".to_string(), Value::Null),
            ("iat".to_string(), json!(1718000000)),
            ("exp".to_string(), json!(4102444800_i64)),
        ])
    }

    #[test]
    fn rust_signiert_byteidentisch_zu_python() {
        let codec = SessionCodec::new(Some(SECRET.to_string()));
        let cookie = codec.sign(&payload()).expect("signieren");
        assert_eq!(cookie, PYTHON_COOKIE);
    }

    #[test]
    fn rust_verifiziert_python_cookie() {
        let codec = SessionCodec::new(Some(SECRET.to_string()));
        let obj = codec.verify(PYTHON_COOKIE, 1718000001).expect("gültig");
        assert_eq!(
            obj.get("user_id").and_then(Value::as_str),
            Some("123456789")
        );
        assert_eq!(obj.get("name").and_then(Value::as_str), Some("Tüster"));
    }

    #[test]
    fn abgelaufene_session_ist_ungueltig() {
        let codec = SessionCodec::new(Some(SECRET.to_string()));
        assert!(codec.verify(PYTHON_COOKIE, 4102444800).is_none());
    }

    #[test]
    fn manipulierte_signatur_ist_ungueltig() {
        let codec = SessionCodec::new(Some(SECRET.to_string()));
        let mut kaputt = PYTHON_COOKIE.to_string();
        kaputt.pop();
        kaputt.push('0');
        assert!(codec.verify(&kaputt, 1718000001).is_none());
    }

    #[test]
    fn ohne_secret_ist_alles_ungueltig() {
        let codec = SessionCodec::new(None);
        assert!(codec.verify(PYTHON_COOKIE, 0).is_none());
        assert!(codec.sign(&payload()).is_none());
        let leer = SessionCodec::new(Some("   ".to_string()));
        assert!(!leer.has_secret());
    }
}
