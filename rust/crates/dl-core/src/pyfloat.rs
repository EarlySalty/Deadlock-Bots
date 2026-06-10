//! Pythons `round(x, nd)` — geteilt von allen Port-Crates.
//!
//! Half-to-even auf der EXAKTEN Dezimaldarstellung des Doubles, nicht auf
//! `x * 10^nd`: der Multiplikationsfehler kippt Grenzfälle (50.365) in die
//! falsche Richtung, Python rundet dezimal korrekt. Wir formatieren den
//! exakten Wert mit 35 Nachkommastellen (jede Binärbruch-Expansion ist bis
//! dahin eindeutig entschieden) und runden auf der Ziffernfolge.

pub fn py_round(x: f64, nd: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let neg = x.is_sign_negative();
    let s = format!("{:.35}", x.abs());
    let Some((int_part, frac_part)) = s.split_once('.') else {
        return x;
    };
    if nd >= frac_part.len() {
        return x;
    }

    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes())
        .map(|b| b - b'0')
        .collect();
    let mut int_len = int_part.len();
    let cut = int_len + nd; // Index der ersten weggerundeten Ziffer

    let first_dropped = digits[cut];
    let round_up = match first_dropped.cmp(&5) {
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => {
            if digits[cut + 1..].iter().any(|d| *d != 0) {
                true
            } else {
                // exakter Tie → zur geraden Ziffer
                digits[cut - 1] % 2 == 1
            }
        }
    };
    digits.truncate(cut);
    if round_up {
        let mut i = cut;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                int_len += 1;
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }

    let mut out = String::with_capacity(digits.len() + 2);
    if neg {
        out.push('-');
    }
    for (i, d) in digits.iter().enumerate() {
        if i == int_len {
            out.push('.');
        }
        out.push((b'0' + d) as char);
    }
    out.parse::<f64>().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::py_round;

    #[test]
    fn referenzwerte_aus_cpython() {
        assert_eq!(py_round(55.355, 2), 55.35);
        assert_eq!(py_round(50.365, 2), 50.37);
        assert_eq!(py_round(0.125, 2), 0.12);
        assert_eq!(py_round(99.995, 2), 100.0);
        assert_eq!(py_round(-55.355, 2), -55.35);
        // round(x, 1) — für avg_session_minutes (CPython-Referenz)
        assert_eq!(py_round(60.25, 1), 60.2);
        assert_eq!(py_round(60.35, 1), 60.4);
        assert_eq!(py_round(0.05, 1), 0.1);
        assert_eq!(py_round(60.0, 1), 60.0);
    }
}
