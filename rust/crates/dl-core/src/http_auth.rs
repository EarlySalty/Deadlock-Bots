pub fn bearer_token(authorization: &str) -> Option<&str> {
    authorization
        .split_once(' ')
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| token.trim())
}
