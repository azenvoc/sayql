use super::{bytes, f32_to_f64, float, num, string};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, SubsecRound};
use serde_json::Value;
use tiberius::{ColumnData, ColumnType, Row};

pub(crate) fn ms_cell(row: &Row, i: usize) -> Value {
    let Some((_, data)) = row.cells().nth(i) else {
        return Value::Null;
    };
    let v = match data {
        ColumnData::U8(v) => num(*v),
        ColumnData::I16(v) => num(*v),
        ColumnData::I32(v) => num(*v),
        ColumnData::I64(v) => num(*v),
        ColumnData::F32(v) => float(v.map(f32_to_f64)),
        ColumnData::F64(v) => float(*v),
        ColumnData::Bit(v) => v.map(Value::Bool),
        ColumnData::String(v) => string(v.as_deref()),
        ColumnData::Guid(v) => string(v.as_ref()),
        // geometry, geography and hierarchyid all arrive as "UDT" bytes, without their type name.
        ColumnData::Binary(Some(b))
            if row
                .columns()
                .get(i)
                .is_some_and(|c| c.column_type() == ColumnType::Udt) =>
        {
            Some(
                planar_geometry(b)
                    .map(Value::String)
                    .unwrap_or_else(|| bytes(Some(b.to_vec())).unwrap_or(Value::Null)),
            )
        }
        ColumnData::Binary(v) => bytes(v.as_ref().map(|b| b.to_vec())),
        // Exact decimal text, no float rounding.
        ColumnData::Numeric(v) => string(v.as_ref()),
        ColumnData::Xml(v) => string(v.as_deref()),
        // `datetime` counts in 1/300 s; SQL Server's own tools show it rounded to milliseconds.
        ColumnData::DateTime(_) => string(
            row.try_get::<NaiveDateTime, _>(i)
                .ok()
                .flatten()
                .map(|t| t.round_subsecs(3)),
        ),
        ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            string(row.try_get::<NaiveDateTime, _>(i).ok().flatten())
        }
        ColumnData::Date(_) => string(row.try_get::<NaiveDate, _>(i).ok().flatten()),
        ColumnData::Time(_) => string(row.try_get::<NaiveTime, _>(i).ok().flatten()),
        ColumnData::DateTimeOffset(_) => {
            string(row.try_get::<DateTime<FixedOffset>, _>(i).ok().flatten())
        }
    };
    v.unwrap_or(Value::Null)
}

/// A `geometry` value as SQL Server's `STAsText()` writes it. Only SRID 0 is decoded: that is
/// always `geometry` (a `geography` always has a real SRID, and stores latitude first, which
/// these bytes alone can't tell apart). Anything else, curves included, stays as bytes.
/// Format: [MS-SSCLRT] 2.1, the CLR serialization of geometry and geography.
fn planar_geometry(b: &[u8]) -> Option<String> {
    let mut r = Reader(b);
    let srid = i32::from_le_bytes(r.take::<4>()?);
    let version = r.u8()?;
    let props = r.u8()?;
    if srid != 0 || !(1..=2).contains(&version) || props & 0xC0 != 0 {
        return None;
    }
    let (has_z, has_m) = (props & 0x01 != 0, props & 0x02 != 0);
    let extra = usize::from(has_z) + usize::from(has_m);
    let wkt = if props & 0x08 != 0 {
        let p = r.points(1, extra)?;
        format!("POINT ({})", p[0])
    } else if props & 0x10 != 0 {
        let p = r.points(2, extra)?;
        format!("LINESTRING ({}, {})", p[0], p[1])
    } else {
        let n = r.u32()? as usize;
        let points = r.points(n, extra)?;
        let figures: Vec<(u8, usize)> = (0..r.u32()?)
            .map(|_| Some((r.u8()?, r.u32()? as usize)))
            .collect::<Option<_>>()?;
        let shapes: Vec<(i32, i32, u8)> = (0..r.u32()?)
            .map(|_| Some((r.i32()?, r.i32()?, r.u8()?)))
            .collect::<Option<_>>()?;
        // Version 2 adds arcs (figure attributes 2 and 3, and a segment list): not decoded.
        if figures.iter().any(|&(attr, _)| attr > 1 && version == 2) || shapes.is_empty() {
            return None;
        }
        Shapes::new(&points, &figures, &shapes).wkt(0, 0)?
    };
    // Version 2 may end with a segment count; nothing else may follow.
    (r.0.is_empty() || version == 2 && r.0.len() == 4 && r.0 == [0, 0, 0, 0]).then_some(wkt)
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_at_checked(N)?;
        self.0 = rest;
        head.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take::<1>()?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take()?))
    }
    /// `n` points as "x y", skipping their Z and M values (stored after all the X/Y pairs).
    fn points(&mut self, n: usize, extra: usize) -> Option<Vec<String>> {
        if n > self.0.len() / 16 {
            return None;
        }
        let xy: Vec<String> = (0..n)
            .map(|_| Some(format!("{} {}", self.f64()?, self.f64()?)))
            .collect::<Option<_>>()?;
        for _ in 0..n * extra {
            self.f64()?;
        }
        Some(xy)
    }
}

/// How deep collections may nest. SQL Server allows any depth, but real values stay a few levels
/// deep, and each level is a recursive call: a crafted value nesting thousands of
/// GEOMETRYCOLLECTIONs would overflow the stack. Deeper values stay as bytes.
const MAX_DEPTH: usize = 32;

struct Shapes<'a> {
    points: &'a [String],
    figures: &'a [(u8, usize)],
    /// (parent shape, first figure or -1, OGC type)
    shapes: &'a [(i32, i32, u8)],
    /// Each shape's members, found once rather than by scanning every shape per collection.
    /// A member always follows its parent, so a crafted parent can't make a cycle.
    children: Vec<Vec<usize>>,
    /// Where each shape's figures end: the first figure of the next shape that has one.
    figures_end: Vec<usize>,
}

impl<'a> Shapes<'a> {
    fn new(points: &'a [String], figures: &'a [(u8, usize)], shapes: &'a [(i32, i32, u8)]) -> Self {
        let mut children = vec![Vec::new(); shapes.len()];
        for (c, &(parent, _, _)) in shapes.iter().enumerate() {
            let parent = usize::try_from(parent).ok().filter(|&p| p < c);
            if let Some(list) = parent.and_then(|p| children.get_mut(p)) {
                list.push(c);
            }
        }
        let mut figures_end = vec![figures.len(); shapes.len()];
        let mut next = figures.len();
        for (s, &(_, first, _)) in shapes.iter().enumerate().rev() {
            if let Some(end) = figures_end.get_mut(s) {
                *end = next;
            }
            if let Ok(f) = usize::try_from(first) {
                next = f;
            }
        }
        Shapes {
            points,
            figures,
            shapes,
            children,
            figures_end,
        }
    }

    /// The points of figure `f`.
    fn figure(&self, f: usize) -> Option<String> {
        let start = self.figures.get(f)?.1;
        let end = self
            .figures
            .get(f.checked_add(1)?)
            .map_or(self.points.len(), |x| x.1);
        Some(self.points.get(start..end)?.join(", "))
    }

    /// Shape `s`'s figures: from its first figure to the next shape's.
    fn figures_of(&self, s: usize) -> Option<std::ops::Range<usize>> {
        let first = usize::try_from(self.shapes.get(s)?.1).ok()?;
        Some(first..*self.figures_end.get(s)?)
    }

    /// Shape `s` as WKT, `depth` collections down from the top.
    fn wkt(&self, s: usize, depth: usize) -> Option<String> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (_, first, kind) = *self.shapes.get(s)?;
        let name = [
            "",
            "POINT",
            "LINESTRING",
            "POLYGON",
            "MULTIPOINT",
            "MULTILINESTRING",
            "MULTIPOLYGON",
            "GEOMETRYCOLLECTION",
        ]
        .get(usize::from(kind))
        .filter(|n| !n.is_empty())?;
        let children = self.children.get(s)?;
        if first < 0 && kind <= 3 {
            return Some(format!("{name} EMPTY"));
        }
        let body = match kind {
            1 | 2 => self.figures_of(s)?.next().and_then(|f| self.figure(f))?,
            3 => self
                .figures_of(s)?
                .map(|f| Some(format!("({})", self.figure(f)?)))
                .collect::<Option<Vec<_>>>()?
                .join(", "),
            // Members of MULTI* collections are written without their type name.
            4..=6 => children
                .iter()
                .map(|&c| {
                    self.wkt(c, depth + 1).map(|w| {
                        w.split_once(' ')
                            .map_or(w.clone(), |(_, rest)| rest.to_string())
                    })
                })
                .collect::<Option<Vec<_>>>()?
                .join(", "),
            _ => children
                .iter()
                .map(|&c| self.wkt(c, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .join(", "),
        };
        if body.is_empty() {
            return Some(format!("{name} EMPTY"));
        }
        Some(format!("{name} ({body})"))
    }
}

#[cfg(test)]
mod tests {
    use super::planar_geometry;

    fn point_bytes(x: f64, y: f64) -> Vec<u8> {
        let mut b = vec![0, 0, 0, 0, 1, 0x0C];
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
        b
    }

    #[test]
    fn decodes_planar_geometry_and_leaves_the_rest() {
        assert_eq!(
            planar_geometry(&point_bytes(1.5, -2.0)).as_deref(),
            Some("POINT (1.5 -2)")
        );
        // A geography (SRID 4326) stays as bytes: its axis order can't be known from them.
        let mut geog = point_bytes(47.6, -122.3);
        geog[..4].copy_from_slice(&4326i32.to_le_bytes());
        assert_eq!(planar_geometry(&geog), None);
        // hierarchyid bytes aren't geometry.
        assert_eq!(planar_geometry(&[0x5b, 0x40]), None);
        assert_eq!(planar_geometry(&[0x5b, 0x40, 0, 0, 1, 0x0C]), None);
    }

    /// A geometry with no points or figures, only the given (parent, OGC type) shapes.
    fn shapes_bytes(shapes: impl IntoIterator<Item = (i32, u8)>) -> Vec<u8> {
        let shapes: Vec<(i32, u8)> = shapes.into_iter().collect();
        let mut b = vec![0, 0, 0, 0, 1, 0x04];
        b.extend(0u32.to_le_bytes()); // points
        b.extend(0u32.to_le_bytes()); // figures
        b.extend(u32::try_from(shapes.len()).unwrap().to_le_bytes());
        for (parent, kind) in shapes {
            b.extend(parent.to_le_bytes());
            b.extend((-1i32).to_le_bytes());
            b.push(kind);
        }
        b
    }

    #[test]
    fn deeply_nested_collections_stay_bytes_without_overflowing_the_stack() {
        assert_eq!(
            planar_geometry(&shapes_bytes((-1..2).map(|p| (p, 7)))).as_deref(),
            Some("GEOMETRYCOLLECTION (GEOMETRYCOLLECTION (GEOMETRYCOLLECTION EMPTY))")
        );
        // ~900 KB: 100,000 GEOMETRYCOLLECTIONs, each inside the one before, on a small stack.
        let deep = shapes_bytes((-1..99_999).map(|p| (p, 7)));
        let decoded = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || planar_geometry(&deep))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(decoded, None);
    }

    #[test]
    fn many_members_of_one_collection_decode_in_linear_time() {
        // Finding each collection's members by scanning every shape took ~10^10 steps here.
        let flat = shapes_bytes(std::iter::once((-1, 7)).chain((0..100_000).map(|_| (0, 7))));
        let start = std::time::Instant::now();
        let wkt = planar_geometry(&flat).unwrap();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(wkt.matches("GEOMETRYCOLLECTION EMPTY").count(), 100_000);
    }
}
