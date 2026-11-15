pub const BILLING: &str = "billing";
pub const MAX_CODE_LEN: usize = 64;

pub fn safe_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CODE_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub fn status_reason(status: u16) -> &'static str {
    match status {
        429 => "http_429",
        529 => "http_529",
        401 => "http_401",
        403 => "http_403",
        400 | 404 | 413 | 422 => "http_4xx",
        500..=599 => "http_5xx",
        _ => "http_status",
    }
}

pub fn refusal_status(status: u16) -> bool {
    matches!(status, 401 | 402 | 403 | 412 | 429 | 500..=599)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_reason_is_a_safe_code_and_request_errors_are_no_refusals() {
        for status in [
            0, 200, 400, 401, 402, 403, 404, 412, 413, 422, 429, 500, 529, 599, 600,
        ] {
            assert!(safe_code(status_reason(status)), "{status}");
        }
        assert!(safe_code(BILLING));
        for status in [400, 404, 413, 422, 200] {
            assert!(!refusal_status(status), "{status}");
        }
        for status in [401, 402, 403, 412, 429, 500, 529] {
            assert!(refusal_status(status), "{status}");
        }
        assert!(safe_code(&"a".repeat(MAX_CODE_LEN)));
        assert!(!safe_code(&"a".repeat(MAX_CODE_LEN + 1)));
        assert!(!safe_code(""));
        assert!(!safe_code("Billing"));
    }
}
