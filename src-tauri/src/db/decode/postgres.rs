use super::{bytes, clock, f32_to_f64, float, printable};
use crate::db::MAX_ROWS;
use serde_json::Value;
use sqlx::postgres::{PgRow, PgTypeInfo, PgTypeKind, PgValueFormat};
use sqlx::{Row, TypeInfo, ValueRef};

// Prepared statements return Postgres values in binary. They are decoded from the wire
// format directly, so arrays, enums, ranges and composites of any element type work and
// out-of-range values (huge NUMERIC, 'infinity' timestamps) cannot panic inside sqlx.

pub(crate) fn pg_cell(row: &PgRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if raw.is_null() {
        return Value::Null;
    }
    let ty = raw.type_info().into_owned();
    let v = match raw.format() {
        PgValueFormat::Binary => raw.as_bytes().ok().and_then(|b| pg_binary(&ty, b)),
        PgValueFormat::Text => raw.as_str().ok().map(|s| Value::String(s.to_string())),
    };
    v.unwrap_or_else(|| pg_placeholder(&ty))
}

fn pg_placeholder(ty: &PgTypeInfo) -> Value {
    Value::String(format!("<{}>", ty.name().to_lowercase()))
}

fn pg_base(mut ty: &PgTypeInfo) -> &PgTypeInfo {
    while let PgTypeKind::Domain(base) = ty.kind() {
        ty = base;
    }
    ty
}

fn pg_oid(ty: &PgTypeInfo) -> u32 {
    ty.oid().map(|o| o.0).unwrap_or(0)
}

struct Buf<'a>(&'a [u8]);

impl<'a> Buf<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(a)
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.arr::<1>()?[0])
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_be_bytes(self.arr()?))
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.arr()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.arr()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.arr()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.arr()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_be_bytes(self.arr()?))
    }
    /// A length-prefixed field; `Some(None)` is SQL NULL.
    fn field(&mut self) -> Option<Option<&'a [u8]>> {
        let len = self.i32()?;
        if len < 0 {
            Some(None)
        } else {
            self.take(len as usize).map(Some)
        }
    }
}

fn pg_binary(ty: &PgTypeInfo, b: &[u8]) -> Option<Value> {
    let ty = pg_base(ty);
    match ty.kind() {
        PgTypeKind::Array(elem) => pg_array(elem, b),
        PgTypeKind::Enum(_) => printable(std::str::from_utf8(b).ok().map(String::from)),
        PgTypeKind::Composite(fields) => pg_record(Some(fields), b),
        PgTypeKind::Range(elem) => pg_range(elem, b),
        _ if pg_oid(ty) == 2249 => pg_record(None, b),
        _ => pg_scalar(pg_oid(ty), b)
            .or_else(|| printable(std::str::from_utf8(b).ok().map(String::from))),
    }
}

fn pg_array(elem: &PgTypeInfo, b: &[u8]) -> Option<Value> {
    let mut r = Buf(b);
    let ndim = r.i32()?;
    let (_has_nulls, _elem_oid) = (r.i32()?, r.u32()?);
    if !(0..=6).contains(&ndim) {
        return None;
    }
    let mut dims = Vec::new();
    for _ in 0..ndim {
        dims.push(usize::try_from(r.i32()?).ok()?);
        r.i32()?; // lower bound
    }
    // A zero dimension holds nothing, however large the others claim to be.
    if dims.is_empty() || dims.contains(&0) {
        return Some(Value::Array(vec![]));
    }
    // Every element takes at least its 4-byte length, so more elements than that leaves room
    // for is nonsense, as is a product that overflows. With no zero dimension, the nested
    // arrays number at most `ndim` times the elements, so this bounds them too.
    let total = dims.iter().try_fold(1usize, |n, &d| n.checked_mul(d))?;
    if total > r.0.len() / 4 {
        return None;
    }
    let mut flat = Vec::with_capacity(total.min(MAX_ROWS));
    for _ in 0..total {
        flat.push(match r.field()? {
            None => Value::Null,
            Some(e) => pg_binary(elem, e).unwrap_or_else(|| pg_placeholder(elem)),
        });
    }
    fn nest(dims: &[usize], it: &mut std::vec::IntoIter<Value>) -> Value {
        let Some((n, rest)) = dims.split_first() else {
            return Value::Array(vec![]);
        };
        Value::Array(
            (0..*n)
                .map(|_| {
                    if rest.is_empty() {
                        it.next().unwrap_or(Value::Null)
                    } else {
                        nest(rest, it)
                    }
                })
                .collect(),
        )
    }
    Some(nest(&dims, &mut flat.into_iter()))
}

/// Named composite types become objects; anonymous `ROW(...)` records become arrays.
fn pg_record(fields: Option<&[(String, PgTypeInfo)]>, b: &[u8]) -> Option<Value> {
    let mut r = Buf(b);
    let n = r.i32()?;
    let mut obj = serde_json::Map::new();
    let mut arr = Vec::new();
    for k in 0..usize::try_from(n).ok()? {
        let oid = r.u32()?;
        let field = fields.and_then(|f| f.get(k));
        let v = match r.field()? {
            None => Value::Null,
            Some(e) => match field {
                Some((_, t)) => pg_binary(t, e).unwrap_or_else(|| pg_placeholder(t)),
                None => pg_scalar(oid, e)
                    .or_else(|| printable(std::str::from_utf8(e).ok().map(String::from)))
                    .unwrap_or_else(|| Value::String(format!("<oid {oid}>"))),
            },
        };
        match field {
            Some((name, _)) => {
                obj.insert(name.clone(), v);
            }
            None => arr.push(v),
        }
    }
    Some(if fields.is_some() {
        Value::Object(obj)
    } else {
        Value::Array(arr)
    })
}

fn pg_range(elem: &PgTypeInfo, b: &[u8]) -> Option<Value> {
    let mut r = Buf(b);
    let flags = r.u8()?;
    if flags & 0x01 != 0 {
        return Some(Value::String("empty".into()));
    }
    let mut bound = |infinite: u8| -> Option<String> {
        if flags & infinite != 0 {
            return Some(String::new());
        }
        match pg_binary(elem, r.field()??)? {
            Value::String(s) => Some(s),
            v => Some(v.to_string()),
        }
    };
    let (lo, hi) = (bound(0x08)?, bound(0x10)?);
    let open = if flags & 0x02 != 0 { '[' } else { '(' };
    let close = if flags & 0x04 != 0 { ']' } else { ')' };
    Some(Value::String(format!("{open}{lo},{hi}{close}")))
}

fn pg_scalar(oid: u32, b: &[u8]) -> Option<Value> {
    use chrono::{NaiveDate, TimeDelta};
    use std::fmt::Write;
    let mut r = Buf(b);
    let epoch = NaiveDate::from_ymd_opt(2000, 1, 1)?;
    let text = || {
        std::str::from_utf8(b)
            .ok()
            .map(|s| Value::String(s.to_string()))
    };
    let timestamp = |us: i64| -> Option<String> {
        match us {
            i64::MAX => Some("infinity".into()),
            i64::MIN => Some("-infinity".into()),
            _ => {
                let t = epoch
                    .and_hms_opt(0, 0, 0)?
                    .checked_add_signed(TimeDelta::microseconds(us))?;
                Some(format!(
                    "{} {}",
                    t.date(),
                    clock(us.rem_euclid(86_400_000_000))
                ))
            }
        }
    };
    let v = match oid {
        16 => Value::Bool(r.u8()? != 0),
        17 => bytes(Some(b.to_vec()))?,
        18 => Value::String((r.u8()? as char).to_string()),
        19 | 25 | 142 | 705 | 1042 | 1043 => text()?,
        20 => Value::from(r.i64()?),
        21 => Value::from(r.i16()?),
        23 => Value::from(r.i32()?),
        // oid, xid, cid and the reg* aliases
        24 | 26 | 28 | 29 | 2202 | 2203 | 2204 | 2205 | 2206 | 3734 | 3769 | 4089 | 4096 => {
            Value::from(r.u32()?)
        }
        700 => float(Some(f32_to_f64(f32::from_bits(r.u32()?))))?,
        701 => float(Some(r.f64()?))?,
        114 => serde_json::from_slice(b).ok()?,
        3802 => {
            r.u8()?; // jsonb format version
            serde_json::from_slice(r.0).ok()?
        }
        4072 => {
            r.u8()?;
            printable(std::str::from_utf8(r.0).ok().map(String::from))?
        }
        1700 => Value::String(pg_numeric(&mut r)?),
        790 => {
            let c = i128::from(r.i64()?);
            Value::String(format!(
                "{}{}.{:02}",
                if c < 0 { "-" } else { "" },
                c.abs() / 100,
                c.abs() % 100
            ))
        }
        2950 => Value::String(uuid::Uuid::from_slice(b).ok()?.to_string()),
        1082 => Value::String(match r.i32()? {
            i32::MAX => "infinity".into(),
            i32::MIN => "-infinity".into(),
            d => epoch
                .checked_add_signed(TimeDelta::days(d.into()))?
                .to_string(),
        }),
        1083 => Value::String(clock(r.i64()?)),
        1114 => Value::String(timestamp(r.i64()?)?),
        1184 => Value::String(match timestamp(r.i64()?)? {
            t if t.ends_with("infinity") => t,
            t => format!("{t} UTC"),
        }),
        1266 => {
            let (us, west) = (r.i64()?, r.i32()?);
            let east = -west;
            let a = east.unsigned_abs();
            let mut s = format!(
                "{}{}{:02}",
                clock(us),
                if east < 0 { '-' } else { '+' },
                a / 3600
            );
            if a % 3600 != 0 {
                write!(s, ":{:02}", a / 60 % 60).ok()?;
            }
            if a % 60 != 0 {
                write!(s, ":{:02}", a % 60).ok()?;
            }
            Value::String(s)
        }
        1186 => {
            let (us, days, months) = (r.i64()?, r.i32()?, r.i32()?);
            Value::String(pg_interval(us, days, months))
        }
        650 | 869 => {
            let (_family, bits, _is_cidr, n) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            let (ip, max): (std::net::IpAddr, u8) = match n {
                4 => (std::net::Ipv4Addr::from(r.arr::<4>()?).into(), 32),
                16 => (std::net::Ipv6Addr::from(r.arr::<16>()?).into(), 128),
                _ => return None,
            };
            Value::String(if oid == 650 || bits != max {
                format!("{ip}/{bits}")
            } else {
                ip.to_string()
            })
        }
        829 | 774 => Value::String(
            b.iter()
                .map(|x| format!("{x:02x}"))
                .collect::<Vec<_>>()
                .join(":"),
        ),
        1560 | 1562 => {
            let n = usize::try_from(r.i32()?).ok()?;
            let bits = r.0;
            let s: Option<String> = (0..n)
                .map(|i| {
                    Some(if bits.get(i / 8)? & (0x80 >> (i % 8)) != 0 {
                        '1'
                    } else {
                        '0'
                    })
                })
                .collect();
            Value::String(s?)
        }
        600 => Value::String(format!("({},{})", r.f64()?, r.f64()?)),
        3220 => {
            let v = u64::from_be_bytes(r.arr()?);
            Value::String(format!("{:X}/{:X}", v >> 32, v & 0xffff_ffff))
        }
        2278 => Value::Null,
        _ => return None,
    };
    Some(v)
}

/// Exact decimal text from Postgres' base-10000 binary NUMERIC, any size.
fn pg_numeric(r: &mut Buf) -> Option<String> {
    use std::fmt::Write;
    let n = usize::try_from(r.i16()?).ok()?;
    let weight = i32::from(r.i16()?);
    let sign = r.u16()?;
    let dscale = usize::from(r.u16()?);
    match sign {
        0xC000 => return Some("NaN".into()),
        0xD000 => return Some("Infinity".into()),
        0xF000 => return Some("-Infinity".into()),
        _ => {}
    }
    let digits: Vec<i16> = (0..n).map(|_| r.i16()).collect::<Option<_>>()?;
    // digits[0] is worth 10000^weight; digit(k) is the group k places after it.
    let digit = |k: i32| {
        usize::try_from(k)
            .ok()
            .and_then(|k| digits.get(k))
            .copied()
            .unwrap_or(0)
    };
    let mut s = String::new();
    if sign == 0x4000 {
        s.push('-');
    }
    if weight < 0 {
        s.push('0');
    } else {
        for k in 0..=weight {
            if k == 0 {
                write!(s, "{}", digit(k))
            } else {
                write!(s, "{:04}", digit(k))
            }
            .ok()?;
        }
    }
    if dscale > 0 {
        let mut frac = String::new();
        let mut j = 1;
        while frac.len() < dscale {
            write!(frac, "{:04}", digit(weight + j)).ok()?;
            j += 1;
        }
        frac.truncate(dscale);
        s.push('.');
        s.push_str(&frac);
    }
    Some(s)
}

/// Same layout as Postgres' default `IntervalStyle`, e.g. `1 year 2 mons 3 days 04:05:06.5`.
fn pg_interval(us: i64, days: i32, months: i32) -> String {
    let unit = |n: i64, u: &str| format!("{n} {u}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    let (years, months) = (months / 12, months % 12);
    if years != 0 {
        parts.push(unit(years.into(), "year"));
    }
    if months != 0 {
        parts.push(unit(months.into(), "mon"));
    }
    if days != 0 {
        parts.push(unit(days.into(), "day"));
    }
    if us != 0 || parts.is_empty() {
        let plus = us > 0 && (years < 0 || months < 0 || days < 0);
        parts.push(format!("{}{}", if plus { "+" } else { "" }, clock(us)));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::pg_array;
    use serde_json::json;
    use sqlx::Postgres;

    /// An `int4[]` in Postgres' binary format: the header, `dims` (lower bound 1) and `items`.
    fn int4_array(dims: &[i32], items: &[i32]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(i32::try_from(dims.len()).unwrap().to_be_bytes());
        b.extend(0i32.to_be_bytes()); // has nulls
        b.extend(23u32.to_be_bytes()); // element oid
        for d in dims {
            b.extend(d.to_be_bytes());
            b.extend(1i32.to_be_bytes());
        }
        for v in items {
            b.extend(4i32.to_be_bytes());
            b.extend(v.to_be_bytes());
        }
        b
    }

    fn decode(b: &[u8]) -> Option<serde_json::Value> {
        pg_array(&<i32 as sqlx::Type<Postgres>>::type_info(), b)
    }

    #[test]
    fn decodes_arrays_of_any_shape() {
        assert_eq!(
            decode(&int4_array(&[2, 2], &[1, 2, 3, 4])),
            Some(json!([[1, 2], [3, 4]]))
        );
        assert_eq!(
            decode(&int4_array(&[1, 1, 1, 1, 1, 1], &[7])),
            Some(json!([[[[[[7]]]]]]))
        );
        assert_eq!(decode(&int4_array(&[], &[])), Some(json!([])));
    }

    #[test]
    fn crafted_array_dimensions_decode_as_empty_or_not_at_all() {
        // 2^31 empty arrays used to be built for this, ~64 GB.
        assert_eq!(decode(&int4_array(&[i32::MAX, 0], &[])), Some(json!([])));
        assert_eq!(decode(&int4_array(&[3, 0, 5], &[])), Some(json!([])));
        // 65536^4 overflows usize: a panic in debug, a wrapped count in release.
        assert_eq!(decode(&int4_array(&[65536; 4], &[1])), None);
        // More elements than the bytes could hold.
        assert_eq!(decode(&int4_array(&[i32::MAX, 2], &[1, 2])), None);
        assert_eq!(decode(&int4_array(&[3], &[1, 2])), None);
        assert_eq!(decode(&int4_array(&[-1], &[])), None);
    }
}
