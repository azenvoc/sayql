//! Which columns and values are likely personal data: by a column's name (its words), and by
//! a value's shape (`crate::safety::pii`).

use super::*;

/// The words of a column name: `PatientEmail`, `patient_email` and `patient email` all give
/// `patient`, `email`. Letters and digits are separate words (`line1` → `line`, `1`).
pub(super) fn name_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            words.push(std::mem::take(&mut cur));
            prev = None;
            continue;
        }
        if let Some(p) = prev
            && ((p.is_lowercase() && c.is_uppercase()) || (p.is_alphabetic() != c.is_alphabetic()))
        {
            words.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
        prev = Some(c);
    }
    words.push(cur);
    words.retain(|w| !w.is_empty());
    words
}

/// Words before `name` that make it a thing's name rather than a person's: `product_name`.
pub(super) const THING_NAMES: &[&str] = &[
    "table",
    "column",
    "schema",
    "product",
    "category",
    "file",
    "host",
    "db",
    "database",
    "event",
    "status",
    "role",
    "plan",
    "type",
    "country",
    "city",
    "state",
    "company",
    "org",
    "organization",
    "organisation",
    "tenant",
    "team",
    "brand",
    "app",
    "service",
    "job",
    "feature",
    "department",
    "dept",
    "device",
    "browser",
    "os",
    "model",
    "item",
    "sku",
    "field",
    "key",
    "index",
    "constraint",
    "view",
    "project",
    "group",
    "region",
    "workspace",
    "account",
    "clinic",
    "facility",
    "site",
    "store",
    "shop",
    "domain",
    "metric",
    "tag",
    "label",
    "permission",
    "queue",
    "topic",
    "channel",
    "bucket",
    "test",
    "step",
    "stage",
    "task",
    "drug",
    "medication",
    "procedure",
    "diagnosis",
];

/// The kind of personal data a column holds, judged by its name.
pub fn column_kind(name: &str) -> Option<&'static str> {
    let words = name_words(name);
    let joined = words.concat();
    let is = |w: &str, list: &[&str]| {
        list.contains(&w) || w.strip_suffix('s').is_some_and(|s| list.contains(&s))
    };
    let has = |list: &[&str]| words.iter().any(|w| is(w, list));
    if joined.contains("email") || has(&["mail"]) {
        Some("email")
    } else if joined.contains("phone")
        || has(&[
            "mobile",
            "cell",
            "tel",
            "telephone",
            "fax",
            "msisdn",
            "whatsapp",
        ])
    {
        Some("phone")
    } else if joined.contains("ipaddress") || has(&["ip", "ipv4", "ipv6"]) {
        Some("ip")
    } else if [
        "password",
        "passwd",
        "passphrase",
        "apikey",
        "secret",
        "credential",
    ]
    .iter()
    .any(|w| joined.contains(w))
        || has(&[
            "pwd", "token", "hash", "salt", "otp", "totp", "session", "cvv", "cvc", "pin",
        ])
        || (has(&[
            "api",
            "private",
            "access",
            "refresh",
            "signing",
            "encryption",
        ]) && has(&["key"]))
    {
        Some("secret")
    } else if joined.contains("socialsecurity")
        || has(&[
            "ssn", "sin", "nin", "passport", "license", "licence", "nhs", "aadhaar", "pan",
        ])
        || (has(&["national", "tax", "government", "gov"]) && has(&["id", "number", "no", "num"]))
    {
        Some("ssn")
    } else if joined.contains("creditcard")
        || joined.contains("cardnumber")
        || has(&["iban", "routing", "swift", "bic"])
        || (has(&["card", "account", "bank"]) && has(&["number", "no", "num"]))
    {
        Some("card")
    } else if has(&["birth", "dob", "birthday", "birthdate", "born"]) {
        Some("birth")
    } else if has(&["mrn", "medical", "insurance", "chart"]) {
        Some("mrn")
    } else if has(&["patient"]) {
        Some("patient")
    } else if joined.contains("address")
        || has(&[
            "addr",
            "street",
            "zip",
            "zipcode",
            "postcode",
            "postal",
            "geo",
            "latitude",
            "longitude",
            "lat",
            "lng",
            "lon",
        ])
    {
        Some("address")
    } else if has(&[
        "firstname",
        "lastname",
        "fullname",
        "surname",
        "forename",
        "nickname",
        "username",
        "displayname",
        "maidenname",
        "handle",
        "signature",
        "initials",
    ]) || (has(&["name"]) && !words.iter().any(|w| is(w, THING_NAMES)))
        || (has(&["login"]) && (words.len() == 1 || has(&["name", "id"])))
    {
        Some("name")
    } else if has(&[
        "note",
        "comment",
        "remark",
        "message",
        "memo",
        "narrative",
        "transcript",
    ]) {
        Some("text")
    } else {
        None
    }
}

/// The kind of personal data a value looks like, judged by its shape.
pub fn value_kind(v: &Value) -> Option<&'static str> {
    match v {
        Value::Null | Value::Bool(_) => None,
        Value::Number(n) => {
            let digits = n.to_string();
            if !digits.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                None
            } else if is_card(&digits) {
                Some("card")
            } else if (10..=15).contains(&digits.trim_start_matches('-').len()) {
                Some("phone")
            } else {
                None
            }
        }
        Value::String(s) => text_kind(s),
        Value::Array(_) | Value::Object(_) => Some("json"),
    }
}

pub(super) fn text_kind(s: &str) -> Option<&'static str> {
    let t = s.trim();
    if let Some(kind) = word_kind(t) {
        return Some(kind);
    }
    // Free text: any word, or any run of digits and separators, that looks personal.
    let words = t
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !(c.is_alphanumeric() || c == '+')));
    let runs = t
        .split(|c: char| !(c.is_ascii_digit() || " +-().".contains(c)))
        .map(|r| r.trim().trim_end_matches('.'));
    if words
        .chain(runs)
        .filter(|w| !w.is_empty())
        .any(|w| word_kind(w).is_some())
    {
        Some("text")
    } else {
        None
    }
}

/// The kind of a single value with no spaces around it.
pub(super) fn word_kind(s: &str) -> Option<&'static str> {
    if is_email(s) {
        Some("email")
    } else if is_ssn(s) {
        Some("ssn")
    } else if is_card(s) {
        Some("card")
    } else if is_phone(s) {
        Some("phone")
    } else if s.parse::<std::net::IpAddr>().is_ok() {
        Some("ip")
    } else if is_long_id(s) {
        Some("id")
    } else {
        None
    }
}
