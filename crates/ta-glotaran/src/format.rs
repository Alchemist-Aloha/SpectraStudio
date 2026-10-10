//! Writing a float the way Python writes it.
//!
//! The legacy Glotaran file is written with `np.savetxt(..., fmt="%s")`, which
//! hands every value to `str()`. Python's float representation is the shortest
//! decimal string that round-trips, in plain notation while the decimal
//! exponent stays between -4 and 16 and in scientific notation outside that
//! window, with a two-digit signed exponent.
//!
//! Rust's own `Display` never uses scientific notation, and its `{:e}` writes
//! `8e-5` where Python writes `8e-05`, so the formatting is spelled out here
//! rather than borrowed. It is verified against Python's own output for the
//! values whose form is easiest to get wrong: the ends of the plain range, the
//! exponent's sign and width, and negative zero.

/// Python's `repr(float)`, which is also `str(float)` on the versions this
/// project targets.
pub fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    // Rust's `{:e}` gives the shortest mantissa that round-trips, which is the
    // same set of digits Python would print.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("the scientific form always has an exponent");
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa
        .trim_start_matches('-')
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if negative { "-" } else { "" };

    // -4 <= exponent < 16 is Python's plain-notation window.
    if (-4..16).contains(&exponent) {
        let mut text = String::new();
        text.push_str(sign);
        if exponent < 0 {
            text.push_str("0.");
            for _ in 0..(-exponent - 1) {
                text.push('0');
            }
            text.push_str(digits);
        } else {
            let point = exponent as usize + 1;
            if digits.len() <= point {
                text.push_str(digits);
                for _ in digits.len()..point {
                    text.push('0');
                }
                text.push_str(".0");
            } else {
                text.push_str(&digits[..point]);
                text.push('.');
                text.push_str(&digits[point..]);
            }
        }
        return text;
    }

    // Scientific: one digit, a point, the rest, then a two-digit exponent.
    // Python writes `8e-05`, not `8.0e-05`: the point only appears when there
    // is something after it.
    let mut text = String::new();
    text.push_str(sign);
    text.push_str(&digits[..1]);
    if digits.len() > 1 {
        text.push('.');
        text.push_str(&digits[1..]);
    }
    text.push('e');
    if exponent < 0 {
        text.push('-');
    } else {
        text.push('+');
    }
    let magnitude = exponent.unsigned_abs();
    if magnitude < 10 {
        text.push('0');
    }
    text.push_str(&magnitude.to_string());
    text
}

/// A whole number of a coordinate axis, written as Python would write it.
pub fn python_int(value: usize) -> String {
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plain_window_ends_where_python_ends_it() {
        // The exponent runs from -4 to 15 in plain notation.
        assert_eq!(python_float(1e-4), "0.0001");
        assert_eq!(python_float(1e-5), "1e-05");
        assert_eq!(python_float(1e15), "1000000000000000.0");
        assert_eq!(python_float(1e16), "1e+16");
        assert_eq!(python_float(1.23e16), "1.23e+16");
    }

    #[test]
    fn an_exponent_keeps_its_sign_and_its_two_digits() {
        assert_eq!(python_float(-8e-5), "-8e-05");
        assert_eq!(python_float(1.5e-5), "1.5e-05");
        assert_eq!(python_float(1e-300), "1e-300");
        assert_eq!(python_float(5e-324), "5e-324");
        assert_eq!(python_float(1.7976931348623157e308), "1.7976931348623157e+308");
    }

    #[test]
    fn a_whole_number_still_gets_a_point() {
        assert_eq!(python_float(0.0), "0.0");
        assert_eq!(python_float(1.0), "1.0");
        assert_eq!(python_float(-1.0), "-1.0");
        assert_eq!(python_float(100.0), "100.0");
        assert_eq!(python_float(400.38315), "400.38315");
    }

    #[test]
    fn the_shortest_round_tripping_form_is_kept() {
        for value in [
            0.1,
            -0.00011,
            0.00028,
            0.000123456,
            799.61685,
            3.141592653589793,
            6.02214076e23,
        ] {
            let text = python_float(value);
            let parsed: f64 = text.parse().expect("it parses back");
            assert_eq!(parsed, value, "{text} does not round-trip");
        }
    }

    #[test]
    fn negative_zero_keeps_its_sign() {
        // Python prints -0.0, and a reader of the file can tell the difference.
        assert_eq!(python_float(-0.0), "-0.0");
    }
}
