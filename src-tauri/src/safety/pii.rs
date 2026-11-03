//! Recognising values that are likely personal data or secrets: emails, US social security
//! numbers, card numbers (Luhn-checked), phone numbers and long opaque ids. Shared by the
//! masker (`safety::mask`) and the check on note text (`store::note_values`); it depends on
//! nothing else in SayQL.

pub fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let ok = |c: char| c.is_alphanumeric() || "._%+-'".contains(c);
    !local.is_empty()
        && local.chars().all(ok)
        && domain.contains('.')
        && domain
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '-'))
}

/// `123-45-6789`.
pub fn is_ssn(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .zip([3, 2, 4])
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_digit()))
}

/// 13 to 19 digits, optionally grouped by spaces or dashes, passing the Luhn check.
pub fn is_card(s: &str) -> bool {
    if !s
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
    {
        return false;
    }
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                if d * 2 > 9 { d * 2 - 9 } else { d * 2 }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// 10 to 15 digits, or 7 or more after a leading `+`, with phone separators. Not dates,
/// and not decimal numbers.
pub fn is_phone(s: &str) -> bool {
    if !s
        .chars()
        .all(|c| c.is_ascii_digit() || " +-().".contains(c))
        || s.starts_with('-')
        || s.ends_with('-')
    {
        return false;
    }
    if s.contains('.') && s.parse::<f64>().is_ok() {
        return false;
    }
    if has_date(s) {
        return false;
    }
    let digits = s.chars().filter(char::is_ascii_digit).count();
    (10..=15).contains(&digits) || (s.starts_with('+') && (7..=15).contains(&digits))
}

/// Contains `2026-10-23` or `23-10-2026`.
fn has_date(s: &str) -> bool {
    let b = s.as_bytes();
    let shape = |at: usize, pattern: &[u8]| {
        b.len() >= at + pattern.len()
            && pattern.iter().enumerate().all(|(k, p)| {
                if *p == b'd' {
                    b[at + k].is_ascii_digit()
                } else {
                    b[at + k] == *p
                }
            })
    };
    (0..b.len()).any(|i| shape(i, b"dddd-dd-dd") || shape(i, b"dd-dd-dddd"))
}

/// Opaque identifiers and secrets: UUIDs, hashes, API keys, JWTs. 16 or more characters with
/// no spaces, letters and at least four digits. Not timestamps.
pub fn is_long_id(s: &str) -> bool {
    s.chars().count() >= 16
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+/=:".contains(c))
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars().filter(char::is_ascii_digit).count() >= 4
        && !starts_with_date(s)
}

/// Starts with `YYYY-MM-DD`: a date or timestamp.
pub fn starts_with_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..10].iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}
