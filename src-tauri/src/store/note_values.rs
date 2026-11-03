//! The one check for "does this note hold a value from the data?", used for every note and
//! suggestion: notes the person saves, notes agent mode suggests, and notes and evidence AI
//! apps suggest over MCP (`suggest_note`).
//!
//! Notes are schema-level facts, and the audit asks for codes and conventions, so these pass:
//! small numbers and enum codes (`status IN (2,3)`, `1=Pending`), capitalised enum and status
//! words, identifiers and SQL, file paths with dates or digits in them
//! (`migrations/2026_03_01_merge.sql`), short commit hashes, dates and units ("divide by 100").
//! These are refused: emails, phone numbers, card, ID and account (IBAN) numbers, IP
//! addresses, UUIDs and other long ids or keys, standalone numbers of six or more digits or
//! with thousands separators, money amounts with a currency symbol, people's names and masked
//! tokens.

use crate::safety::pii::{is_card, is_email, is_long_id, is_phone, is_ssn};

/// What the refusal lists as fine and as not, to keep the message true to the checks.
pub const ALLOWED: &str =
    "Codes, enum names, SQL, file paths, dates and short commit hashes are fine";
pub const REFUSED: &str = "leave out values from rows: emails, phone, card or account numbers, \
     UUIDs and long ids, numbers of six or more digits, money amounts and people's names";

/// Characters that split a word into parts: SQL and code punctuation. `1=Pending` is `1` and
/// `Pending`; `IN (1,2)` is `IN`, `1` and `2`.
const SEPARATORS: &[char] = &[
    '=', ',', ';', '(', ')', '[', ']', '{', '}', '<', '>', '"', '\'', '`', '|', '*',
];

/// What in `text` looks like a value from the data, if anything (for the refusal message).
pub fn note_value(text: &str) -> Option<&'static str> {
    if text.contains('‹') {
        return Some("a masked value");
    }
    for word in text.split_whitespace() {
        if let Some(what) = money(word).or_else(|| thousands(word)) {
            return Some(what);
        }
        let whole = trim(word);
        let parts = std::iter::once(whole).chain(whole.split(SEPARATORS).map(trim));
        for part in parts.filter(|p| !p.is_empty()) {
            if let Some(what) = part_value(part) {
                return Some(what);
            }
        }
    }
    spaced_iban(text)
        .or_else(|| digit_runs(text))
        .or_else(|| person_name(text))
}

fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| !(c.is_alphanumeric() || c == '+'))
}

/// One word, or one part of a word, with no spaces in it.
fn part_value(p: &str) -> Option<&'static str> {
    if is_email(p) {
        return Some("an email address");
    }
    if p.contains(['.', ':']) && p.parse::<std::net::IpAddr>().is_ok() {
        return Some("an IP address");
    }
    if is_uuid(p) {
        return Some("a UUID");
    }
    if is_ssn(p) {
        return Some("an ID number");
    }
    if is_card(p) {
        return Some("a card number");
    }
    if is_iban(p) {
        return Some("an account number");
    }
    // Paths and identifiers are judged by their segments, so a migration named by its date
    // or timestamp passes and a key or long hash in one doesn't.
    let long = if p.contains(['/', '_']) {
        p.split(['/', '_', '.', '-']).any(is_long_id)
    } else {
        is_long_id(p)
    };
    if long {
        return Some("a long id");
    }
    if let Some(digits) = number_digits(p) {
        if digits >= 6 {
            return Some("a long number");
        }
    } else if is_phone(p) {
        return Some("a phone number");
    }
    None
}

/// The number of digits before the decimal point, if `p` is just a number.
fn number_digits(p: &str) -> Option<usize> {
    let p = p.trim_start_matches(['+', '-']);
    let (int, frac) = p.split_once('.').unwrap_or((p, ""));
    (!int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b.is_ascii_digit()))
    .then_some(int.len())
}

/// `550e8400-e29b-41d4-a716-446655440000`.
fn is_uuid(p: &str) -> bool {
    let parts: Vec<&str> = p.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(s, n)| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `GB82WEST12345698765432`: two letters, two digits, then 11 to 30 letters and digits.
fn is_iban(p: &str) -> bool {
    let b = p.as_bytes();
    (15..=34).contains(&b.len())
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[4..]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b.iter().filter(|c| c.is_ascii_digit()).count() >= 8
}

/// `GB82 WEST 1234 5698 7654 32`: an IBAN in groups of four.
fn spaced_iban(text: &str) -> Option<&'static str> {
    let words: Vec<&str> = text.split_whitespace().map(trim).collect();
    let group = |w: &str| {
        (1..=4).contains(&w.len())
            && w.bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    (0..words.len())
        .any(|i| {
            let start = words[i].as_bytes();
            if start.len() != 4
                || !start[..2].iter().all(u8::is_ascii_uppercase)
                || !start[2..].iter().all(u8::is_ascii_digit)
            {
                return false;
            }
            let joined: String = words[i..]
                .iter()
                .take_while(|w| group(w))
                .take(9)
                .copied()
                .collect();
            is_iban(&joined)
        })
        .then_some("an account number")
}

/// `$1,200.50`, `€30`, `₹500`. A `$` before one or two digits alone is a SQL parameter (`$1`).
fn money(word: &str) -> Option<&'static str> {
    let chars: Vec<char> = word.chars().collect();
    chars.iter().enumerate().find_map(|(i, c)| {
        let rest: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit() || **c == ',' || **c == '.')
            .collect();
        let rest = rest.trim_end_matches(['.', ',']);
        let digits = rest.chars().filter(char::is_ascii_digit).count();
        let amount = match c {
            '€' | '£' | '₹' | '¥' => digits > 0,
            '$' => digits >= 3 || (digits > 0 && rest.contains(['.', ','])),
            _ => false,
        };
        amount.then_some("a money amount")
    })
}

/// `130,000`: digits grouped in threes by commas. Not in a parenthesised list, where
/// `IN (200,404)` is codes.
fn thousands(word: &str) -> Option<&'static str> {
    if word.contains(['(', ')']) {
        return None;
    }
    let b = word.as_bytes();
    (0..b.len())
        .filter(|&i| b[i] == b',')
        .any(|i| {
            let before = b[..i]
                .iter()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let after = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            (1..=3).contains(&before) && after == 3
        })
        .then_some("a number with thousands separators")
}

/// Phone, card and ID numbers written with spaces: `+1 555 014 2233`, `4111 1111 1111 1111`.
/// Words with letters or path characters are left out: a migration named by its timestamp is
/// no phone number.
fn digit_runs(text: &str) -> Option<&'static str> {
    let numbers = text
        .split_whitespace()
        .map(|w| {
            if w.contains(|c: char| c.is_alphabetic() || "_/@".contains(c)) {
                "|"
            } else {
                w
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    numbers
        .split(|c: char| !(c.is_ascii_digit() || " +-().".contains(c)))
        .map(|r| r.trim().trim_matches(['(', ')', '.', ' ']).trim())
        .filter(|r| r.chars().filter(char::is_ascii_digit).count() >= 9)
        .find_map(|r| {
            if is_card(r) {
                Some("a card number")
            } else if is_ssn(r) {
                Some("an ID number")
            } else if is_phone(r) {
                Some("a phone number")
            } else {
                None
            }
        })
}

/// Two capitalised words in a row in prose, like `Priya Raman`, unless they are enum, status or
/// schema words, product names, months or sentence openers. Words in backticks are code, and a
/// word stuck to `=` or `:` (`1=Pending`) or split off by punctuation (`Paid, Shipped`) isn't a
/// name. A known first name followed by any capitalised word is.
fn person_name(text: &str) -> Option<&'static str> {
    let prose = without_code(text);
    let words: Vec<&str> = prose.split_whitespace().collect();
    words
        .windows(2)
        .any(|w| {
            let (a, b) = (w[0], w[1]);
            // Punctuation may come before the first and after the second, not between them.
            let a = a.trim_start_matches(['(', '[', '"', '\'', '“', '‘']);
            let b = b.trim_end_matches(|c: char| !c.is_alphabetic());
            let b = b.strip_suffix("'s").unwrap_or(b);
            let (Some(a), Some(b)) = (title_word(a), title_word(b)) else {
                return false;
            };
            let (la, lb) = (a.to_lowercase(), b.to_lowercase());
            FIRST_NAMES.contains(&la.as_str())
                || !(NOT_NAMES.contains(&la.as_str()) || NOT_NAMES.contains(&lb.as_str()))
        })
        .then_some("what looks like a person's name")
}

/// `w` when it's a capitalised word: a capital, then at least one more letter, all lowercase.
fn title_word(w: &str) -> Option<&str> {
    let mut cs = w.chars();
    (cs.next().is_some_and(char::is_uppercase)
        && w.chars().count() >= 2
        && cs.all(char::is_lowercase))
    .then_some(w)
}

/// `text` with everything in backticks blanked out.
fn without_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut code = false;
    for c in text.chars() {
        if c == '`' {
            code = !code;
            out.push(' ');
        } else if code {
            out.push(if c.is_whitespace() { c } else { 'x' });
        } else {
            out.push(c);
        }
    }
    out
}

/// Common first names: a capitalised word after one of these is taken as a surname.
const FIRST_NAMES: &[&str] = &[
    "aarav", "aditi", "adith", "aisha", "alice", "amit", "ananya", "anjali", "anna", "arjun",
    "bob", "carlos", "charlie", "chen", "daniel", "dana", "david", "deepak", "divya", "elena",
    "emily", "emma", "fatima", "george", "hannah", "james", "jane", "jennifer", "john", "jose",
    "kavya", "kumar", "laura", "lisa", "maria", "mary", "michael", "mohammed", "muhammad", "neha",
    "olivia", "omar", "pooja", "priya", "rahul", "raj", "ravi", "robert", "sarah", "sneha",
    "sophia", "sunil", "thomas", "vijay", "vikram", "wei", "william", "yuki",
];

/// Capitalised words that aren't part of a person's name: sentence openers, schema and enum
/// words, months and days, and products.
const NOT_NAMES: &[&str] = &[
    // Sentence openers and connectives.
    "a",
    "all",
    "also",
    "an",
    "and",
    "any",
    "as",
    "at",
    "be",
    "both",
    "but",
    "by",
    "do",
    "each",
    "every",
    "for",
    "from",
    "if",
    "in",
    "is",
    "it",
    "its",
    "never",
    "no",
    "not",
    "note",
    "of",
    "on",
    "only",
    "or",
    "see",
    "some",
    "the",
    "then",
    "these",
    "this",
    "those",
    "to",
    "use",
    "when",
    "where",
    "with",
    // Schema words.
    "account",
    "accounts",
    "amount",
    "api",
    "app",
    "audit",
    "boolean",
    "client",
    "code",
    "codes",
    "column",
    "columns",
    "count",
    "currency",
    "customer",
    "customers",
    "database",
    "date",
    "default",
    "enum",
    "false",
    "foreign",
    "id",
    "index",
    "integer",
    "invoice",
    "invoices",
    "item",
    "items",
    "job",
    "key",
    "line",
    "lines",
    "log",
    "logs",
    "event",
    "events",
    "session",
    "sessions",
    "setting",
    "settings",
    "config",
    "feature",
    "flag",
    "flags",
    "group",
    "groups",
    "category",
    "categories",
    "tag",
    "tags",
    "notes",
    "legacy",
    "migration",
    "migrations",
    "null",
    "order",
    "orders",
    "payment",
    "payments",
    "plan",
    "price",
    "primary",
    "product",
    "products",
    "query",
    "report",
    "reports",
    "revenue",
    "role",
    "row",
    "rows",
    "schema",
    "server",
    "service",
    "status",
    "sum",
    "table",
    "tables",
    "team",
    "text",
    "time",
    "timestamp",
    "total",
    "true",
    "type",
    "user",
    "users",
    "value",
    "values",
    "view",
    "worker",
    // Enum and status words.
    "active",
    "admin",
    "approved",
    "archived",
    "basic",
    "blocked",
    "business",
    "canceled",
    "cancelled",
    "closed",
    "complete",
    "completed",
    "critical",
    "debug",
    "deleted",
    "delivered",
    "dev",
    "done",
    "draft",
    "enterprise",
    "error",
    "expired",
    "external",
    "failed",
    "free",
    "fulfilled",
    "guest",
    "high",
    "inactive",
    "info",
    "internal",
    "live",
    "low",
    "medium",
    "member",
    "new",
    "off",
    "open",
    "owner",
    "paid",
    "partial",
    "pending",
    "premium",
    "private",
    "pro",
    "processing",
    "production",
    "public",
    "published",
    "queued",
    "received",
    "refunded",
    "rejected",
    "returned",
    "running",
    "sandbox",
    "scheduled",
    "sent",
    "shipped",
    "staging",
    "standard",
    "succeeded",
    "success",
    "suspended",
    "test",
    "trial",
    "unfulfilled",
    "unpaid",
    "unverified",
    "verified",
    "warning",
    "yes",
    // Months and days.
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    // Products and places.
    "amazon",
    "apple",
    "aws",
    "azure",
    "bigquery",
    "claude",
    "clickhouse",
    "cursor",
    "django",
    "excel",
    "firebase",
    "github",
    "gitlab",
    "google",
    "hubspot",
    "india",
    "java",
    "javascript",
    "jira",
    "kafka",
    "laravel",
    "linear",
    "mariadb",
    "microsoft",
    "mongo",
    "mongodb",
    "mysql",
    "node",
    "notion",
    "oracle",
    "paypal",
    "php",
    "postgres",
    "postgresql",
    "prisma",
    "python",
    "rails",
    "razorpay",
    "react",
    "redis",
    "redshift",
    "ruby",
    "rust",
    "salesforce",
    "sayql",
    "shopify",
    "slack",
    "snowflake",
    "sql",
    "sqlite",
    "stripe",
    "supabase",
    "twilio",
    "typescript",
    "zendesk",
];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The six suggest_note calls refused in the real audit run of 2026-10-28, text and
    /// evidence as Claude Code sent them. All are schema-level facts.
    pub(crate) const REFUSED_IN_THE_REAL_RUN: [(&str, &str); 6] = [
        (
            "orders.status is an integer code: 1=Pending, 2=Paid, 3=Shipped, 4=Refunded. (Corrects the earlier note: 2 is Paid, not Shipped; Shipped is 3.)",
            "src/orders/status.ts defines `enum OrderStatus { Pending = 1, Paid = 2, Shipped = 3, Refunded = 4 }` with the comment \"Order status codes stored in orders.status (INTEGER)\". src/orders/repo.ts markShipped() writes OrderStatus.Shipped (3) on UPDATE orders SET status.",
        ),
        (
            "An \"open\" order means orders.status IN (1,2) (Pending or Paid); Shipped (3) and Refunded (4) are closed.",
            "src/orders/status.ts isOpen(): `return status === OrderStatus.Pending || status === OrderStatus.Paid;` (codes 1 and 2).",
        ),
        (
            "Revenue convention: the app counts only orders.status IN (2,3) (Paid, Shipped) as revenue, excluding Pending (1) and Refunded (4). Match this filter to reproduce app revenue figures.",
            "src/orders/repo.ts revenueCents(): `SELECT SUM(total_cents) AS c FROM orders WHERE status IN (?, ?) AND created_at >= ?` bound with OrderStatus.Paid and OrderStatus.Shipped.",
        ),
        (
            "orders.total_cents is an integer amount in cents, USD. Divide by 100 for dollars; never SUM it alongside orders_legacy.amount, which is in dollars.",
            "src/orders/repo.ts comment \"orders.total_cents: integer amount in cents, USD.\" migrations/2026_03_01_merge_orders_legacy.sql converts with CAST(ROUND(amount * 100) AS INTEGER), noting \"orders_legacy.amount was dollars (REAL); orders.total_cents is integer cents\".",
        ),
        (
            "An open order is one whose status is Pending or Paid; Shipped and Refunded count as closed.",
            "src/orders/status.ts isOpen() returns true only for the Pending and Paid members of the OrderStatus enum.",
        ),
        (
            "orders_legacy is retired and read-only: a migration copied all its rows into orders and nothing has written it since. Query orders instead. SQLite keeps no usage statistics, so this rests on the code alone.",
            "migrations/2026_03_01_merge_orders_legacy.sql inserts every orders_legacy row into orders and states that after the migration nothing writes orders_legacy any more and it is kept read-only for audit. No other reference to orders_legacy anywhere in the project (searched src/, migrations/, docs/). SQLite exposes no table usage statistics, and I cannot run git log in this environment.",
        ),
    ];

    #[test]
    fn schema_facts_from_the_real_audit_pass() {
        for (text, evidence) in REFUSED_IN_THE_REAL_RUN {
            assert_eq!(note_value(text), None, "{text}");
            assert_eq!(note_value(evidence), None, "{evidence}");
        }
        for fact in [
            "status 3 = cancelled (appointment_statuses)",
            "owner = memberships.role = 'owner'",
            "orders_legacy is unused since March 2026; use orders_v2 (status 3 = cancelled in Stripe)",
            "no references in api/ since commit a1b2c3d (2026-03-12); pg stats: 0 writes since 2026-01-01",
            "db/migrate/20260301120000_add_refunds.rb added refunds.reason",
            "WHERE id = $1 AND tenant_id = $2",
            "reviews.rating is 1 to 5; 0 means removed",
            "amounts are in paise: divide by 100 for rupees",
            "Order Status values: Active Trial, Paid Plan",
            "plans.tier is Free, Pro or Enterprise",
            "`Priya Raman` is the seed user in fixtures/users.ts",
            "orders.total is 19.99-style decimals, two places",
            "version 4.5 of the API dropped the field",
        ] {
            assert_eq!(note_value(fact), None, "{fact}");
        }
    }

    #[test]
    fn values_from_rows_are_refused() {
        for (value, what) in [
            ("owner is dana@example.com", "an email address"),
            ("call +1 555 014 2233 for access", "a phone number"),
            ("call +1 (555) 014-2233", "a phone number"),
            ("test card 4111 1111 1111 1111", "a card number"),
            ("card 4111111111111111 is on file", "a card number"),
            (
                "tenant 550e8400-e29b-41d4-a716-446655440000 is the main one",
                "a UUID",
            ),
            ("the admin is John Smith", "what looks like a person's name"),
            (
                "Priya Raman owns the data",
                "what looks like a person's name",
            ),
            (
                "the VIP customer is Alice Smith",
                "what looks like a person's name",
            ),
            (
                "Ask Priya Order about it",
                "what looks like a person's name",
            ),
            ("customer 1234567 is a test account", "a long number"),
            ("top salary is 130000", "a long number"),
            (
                "top salary is 130,000",
                "a number with thousands separators",
            ),
            ("the biggest order was $1,250.00", "a money amount"),
            ("the biggest order was €300", "a money amount"),
            ("payouts go to GB82WEST12345698765432", "an account number"),
            (
                "payouts go to GB82 WEST 1234 5698 7654 32",
                "an account number",
            ),
            ("SSN 123-45-6789", "an ID number"),
            ("the API key is sk_live_abc123def456ghi789jkl", "a long id"),
            ("the server is at 10.0.0.12", "an IP address"),
            ("owner is ‹name-1›", "a masked value"),
        ] {
            assert_eq!(note_value(value), Some(what), "{value}");
        }
    }
}
