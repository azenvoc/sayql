//! What a query's SQL says about its result columns: the top-level SELECT's items, read well
//! enough to tell a counted or summed column from one that passes values through.

use super::*;

/// What a query's SQL says about its result columns.
#[derive(Debug, Clone, Default)]
pub(super) struct Shape {
    /// The SQL can move a value to a result column without the column's name in that column's
    /// expression (UNION, VALUES, column alias lists, LATERAL, whole-row values, JSON access,
    /// `(row).field`, functions in FROM, `U&` names): result columns can't be traced.
    pub(super) opaque: bool,
    /// The top-level select list, when it can be read.
    pub(super) items: Option<Vec<Item>>,
    /// Result positions holding never-send values (from a Postgres plan).
    pub(super) withheld: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct Item {
    /// `*` or `t.*`.
    pub(super) star: bool,
    /// The result column's name: its alias, or the column's own name.
    pub(super) name: Option<String>,
    /// A table's own column, a count, or a sum/average/min/max of a column (with arithmetic and
    /// rounding around them), none of them personal: min and max of it are safe to show.
    pub(super) plain: bool,
}

/// Functions whose results are about a column (or a number), not text built from it.
pub(super) const NUMERIC_WRAPPERS: &[&str] = &[
    "sum", "avg", "min", "max", "round", "coalesce", "nullif", "abs", "floor", "ceil", "ceiling",
    "trunc", "greatest", "least",
];

/// Function names that turn rows or JSON into values (or rows into many).
pub(super) const OPAQUE_FUNCTION_PREFIXES: &[&str] = &[
    "json",
    "to_json",
    "row_to_json",
    "array_to_json",
    "xml",
    "hstore",
    "openjson",
    "unnest",
    "string_to_table",
    "regexp_split_to_table",
    "string_split",
    "crosstab",
    "query_to_xml",
    "table_to_xml",
];

impl Shape {
    pub(super) fn of(sql: &str, personal: &dyn Fn(&str) -> bool) -> Shape {
        let lower = sql.to_lowercase();
        let mut opaque = lower.contains("u&\"") || lower.contains("u&'");
        let mut items = None;
        for d in [Dialect::Postgres, Dialect::Mssql] {
            for (n, l) in lex(sql, d).into_iter().enumerate() {
                opaque |= is_opaque(&l.toks);
                if d == Dialect::Postgres && n == 0 {
                    items = top_items(&l.toks, personal);
                }
            }
        }
        Shape {
            opaque,
            items,
            withheld: Vec::new(),
        }
    }

    /// For each result column, whether it may get min and max.
    pub(super) fn min_max_columns(&self, columns: &[String]) -> Vec<bool> {
        let none = vec![false; columns.len()];
        let Some(items) = self.items.as_ref().filter(|_| !self.opaque) else {
            return none;
        };
        if !items.iter().any(|it| it.star) {
            return if items.len() == columns.len() {
                items.iter().map(|it| it.plain).collect()
            } else {
                none
            };
        }
        // With `*`, columns not named by an explicit item come from the star: a table's own.
        columns
            .iter()
            .map(|c| {
                let c = c.to_lowercase();
                match items
                    .iter()
                    .find(|it| !it.star && it.name.as_deref() == Some(c.as_str()))
                {
                    Some(it) => it.plain,
                    None => items.iter().any(|it| it.star && it.plain),
                }
            })
            .collect()
    }
}

/// Words that may follow `)` and come before `(` without naming an alias list.
pub(super) const NOT_ALIAS_LIST: &[&str] = &[
    "as",
    "over",
    "filter",
    "within",
    "and",
    "or",
    "not",
    "in",
    "on",
    "exists",
    "then",
    "else",
    "when",
    "is",
    "like",
    "ilike",
    "using",
    "join",
    "inner",
    "left",
    "right",
    "full",
    "cross",
    "from",
    "where",
    "select",
    "between",
    "case",
    "end",
    "any",
    "all",
    "some",
    "group",
    "order",
    "having",
    "by",
    "limit",
    "offset",
    "with",
    "outer",
    "natural",
    "similar",
    "escape",
    "nulls",
    "asc",
    "desc",
    "collate",
    "at",
    "returning",
    "option",
    "window",
    "partition",
    "rows",
    "range",
    "groups",
    "fetch",
    "top",
    "distinct",
    "into",
    "set",
    "for",
    "except",
    "union",
    "intersect",
    "values",
    "lateral",
    "apply",
];

pub(super) fn is_opaque(t: &[Tok]) -> bool {
    let word = |i: usize, w: &str| t.get(i).is_some_and(|x| x.is_word(w));
    let punct = |i: usize, c: char| t.get(i).is_some_and(|x| x.is_punct(c));
    let mut from_depth: Vec<bool> = vec![false];
    for (i, tok) in t.iter().enumerate() {
        if let Tok::Word(w) = tok {
            let w = w.to_lowercase();
            if [
                "union",
                "intersect",
                "except",
                "minus",
                "values",
                "lateral",
                "apply",
                "pivot",
                "unpivot",
                "uescape",
            ]
            .contains(&w.as_str())
            {
                return true;
            }
            if punct(i + 1, '(') && OPAQUE_FUNCTION_PREFIXES.iter().any(|p| w.starts_with(p)) {
                return true;
            }
            let top = from_depth.last_mut().expect("never empty");
            if w == "from" || w == "join" {
                *top = true;
            } else if ["where", "group", "order", "having", "limit", "select", "on"]
                .contains(&w.as_str())
            {
                *top = false;
            }
        }
        match tok {
            Tok::Punct('(') => {
                // A function in FROM (`FROM unnest(...) x`, `JOIN regexp_matches(name, ...) m`).
                let in_from = *from_depth.last().expect("never empty");
                let prev = i.checked_sub(1).map(|p| &t[p]);
                let before = i.checked_sub(2).map(|p| &t[p]);
                if in_from
                    && prev.is_some_and(|p| p.ident().is_some())
                    && before.is_some_and(|b| {
                        b.is_word("from") || b.is_word("join") || b.is_punct(',') || b.is_punct('.')
                    })
                {
                    return true;
                }
                from_depth.push(false);
            }
            Tok::Punct(')') => {
                from_depth.pop();
                if from_depth.is_empty() {
                    from_depth.push(false);
                }
                // `(row).field`, and alias lists: `) AS t(a)`, `) t(a)`, a CTE's `x(a) AS (`.
                if punct(i + 1, '.')
                    || (word(i + 1, "as") && punct(i + 2, '('))
                    || (matches!(t.get(i + 1), Some(Tok::Quoted(_)))
                        || t.get(i + 1).is_some_and(|a| matches!(a, Tok::Word(w) if !NOT_ALIAS_LIST.contains(&w.to_lowercase().as_str()))))
                        && punct(i + 2, '(')
                    || (word(i + 1, "as")
                        && t.get(i + 2).and_then(Tok::ident).is_some()
                        && punct(i + 3, '('))
                {
                    return true;
                }
            }
            // JSON access: -> ->> #> #>>
            Tok::Punct('-' | '#') if punct(i + 1, '>') => return true,
            _ => {}
        }
    }
    // Whole-row references and column alias lists on tables.
    crate::safety::never_send::whole_rows_or_alias_lists(t)
}

/// The top-level SELECT's items, if the SQL has one that can be read.
pub(super) fn top_items(t: &[Tok], personal: &dyn Fn(&str) -> bool) -> Option<Vec<Item>> {
    let mut depth = 0usize;
    let mut start = None;
    let mut derived = false;
    for (i, tok) in t.iter().enumerate() {
        match tok {
            Tok::Punct('(') => {
                if i > 0 && (t[i - 1].is_word("from") || t[i - 1].is_word("join")) {
                    derived = true;
                }
                depth += 1;
            }
            Tok::Punct(')') => depth = depth.saturating_sub(1),
            Tok::Word(_) if depth == 0 && tok.is_word("with") => derived = true,
            Tok::Word(_) if depth == 0 && tok.is_word("select") && start.is_none() => {
                start = Some(i + 1)
            }
            _ => {}
        }
    }
    let mut i = start?;
    // DISTINCT [ON (...)], ALL, TOP n [PERCENT] [WITH TIES].
    loop {
        if t.get(i)
            .is_some_and(|x| x.is_word("distinct") || x.is_word("all"))
        {
            i += 1;
            if t.get(i).is_some_and(|x| x.is_word("on"))
                && t.get(i + 1).is_some_and(|x| x.is_punct('('))
            {
                i = close_of(t, i + 1)? + 1;
            }
        } else if t.get(i).is_some_and(|x| x.is_word("top")) {
            i += 1;
            if t.get(i).is_some_and(|x| x.is_punct('(')) {
                i = close_of(t, i)? + 1;
            } else {
                i += 1;
            }
            while t
                .get(i)
                .is_some_and(|x| x.is_word("percent") || x.is_word("with") || x.is_word("ties"))
            {
                i += 1;
            }
        } else {
            break;
        }
    }
    let ends = [
        "from",
        "into",
        "where",
        "group",
        "having",
        "order",
        "limit",
        "union",
        "intersect",
        "except",
        "window",
        "fetch",
        "offset",
        "for",
    ];
    let mut items = Vec::new();
    let mut cur: Vec<&Tok> = Vec::new();
    let mut depth = 0usize;
    while let Some(tok) = t.get(i) {
        match tok {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') if depth == 0 => break,
            Tok::Punct(')') => depth -= 1,
            Tok::Word(w) if depth == 0 && ends.contains(&w.to_lowercase().as_str()) => break,
            Tok::Punct(',') if depth == 0 => {
                items.push(item(&std::mem::take(&mut cur), derived, personal));
                i += 1;
                continue;
            }
            _ => {}
        }
        cur.push(tok);
        i += 1;
    }
    items.push(item(&cur, derived, personal));
    Some(items)
}

/// Index of the `)` closing the `(` at `open`.
pub(super) fn close_of(t: &[Tok], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, tok) in t.iter().enumerate().skip(open) {
        if tok.is_punct('(') {
            depth += 1;
        } else if tok.is_punct(')') {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

pub(super) fn item(toks: &[&Tok], derived: bool, personal: &dyn Fn(&str) -> bool) -> Item {
    let n = toks.len();
    let is_ident = |t: &Tok| matches!(t, Tok::Word(_) | Tok::Quoted(_));
    if matches!(toks, [Tok::Punct('*')])
        || (n == 3 && toks[1].is_punct('.') && toks[2].is_punct('*'))
    {
        return Item {
            star: true,
            name: None,
            plain: !derived,
        };
    }
    let (name, expr) = if n >= 3 && toks[n - 2].is_word("as") {
        (toks[n - 1].ident(), &toks[..n - 2])
    } else if n >= 2
        && is_ident(toks[n - 1])
        && (is_ident(toks[n - 2]) || toks[n - 2].is_punct(')'))
        && !toks[n - 1].is_word("end")
    {
        (toks[n - 1].ident(), &toks[..n - 1])
    } else {
        (toks.last().and_then(|t| t.ident()), toks)
    };
    // Counts are fine whatever they count (a count is like a row count): drop their insides.
    let mut kept: Vec<&Tok> = Vec::new();
    let mut i = 0;
    while i < expr.len() {
        if expr[i].is_word("count") && expr.get(i + 1).is_some_and(|x| x.is_punct('(')) {
            let mut depth = 0usize;
            let mut j = i + 1;
            while j < expr.len() {
                if expr[j].is_punct('(') {
                    depth += 1;
                } else if expr[j].is_punct(')') {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            kept.push(&Tok::Num);
            i = j + 1;
            // count(*) FILTER (WHERE ...) is still a count.
            if expr.get(i).is_some_and(|x| x.is_word("filter")) {
                i += 1;
                let mut depth = 0usize;
                while i < expr.len() {
                    if expr[i].is_punct('(') {
                        depth += 1;
                    } else if expr[i].is_punct(')') {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            }
            continue;
        }
        kept.push(expr[i]);
        i += 1;
    }
    let plain = !kept.is_empty()
        && kept.iter().enumerate().all(|(k, tok)| match tok {
            Tok::Num => true,
            Tok::Punct(c) => "+-*/%(),.".contains(*c),
            Tok::Str(_) => false,
            Tok::Word(_) | Tok::Quoted(_) => {
                let id = tok.ident().unwrap_or_default();
                let call = kept.get(k + 1).is_some_and(|x| x.is_punct('('));
                if call {
                    NUMERIC_WRAPPERS.contains(&id.as_str())
                } else if tok.is_word("distinct") {
                    true
                } else {
                    // A column of a derived table or CTE may be computed.
                    !derived && !personal(&id)
                }
            }
        });
    Item {
        star: false,
        name,
        plain,
    }
}
