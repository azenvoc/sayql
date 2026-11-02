use super::{Described, assemble};
use crate::db::MssqlPool;
use crate::error::AppResult;
use crate::schema::Table;
use std::collections::HashMap;
use tiberius::Row;

const COLUMNS: &str = r#"
SELECT c.TABLE_SCHEMA, c.TABLE_NAME, c.COLUMN_NAME,
       -- Only character and binary types take a length: xml, geography, geometry and hierarchyid
       -- also report one (-1 or a byte size), but `geography(max)` isn't a type anyone can write.
       CASE WHEN c.DATA_TYPE IN ('char', 'varchar', 'nchar', 'nvarchar', 'binary', 'varbinary') THEN
                 c.DATA_TYPE + '(' + CASE WHEN c.CHARACTER_MAXIMUM_LENGTH = -1 THEN 'max'
                                          ELSE CAST(c.CHARACTER_MAXIMUM_LENGTH AS varchar(10)) END + ')'
            WHEN c.DATA_TYPE IN ('decimal', 'numeric')
                 THEN c.DATA_TYPE + '(' + CAST(c.NUMERIC_PRECISION AS varchar(5)) + ',' + CAST(c.NUMERIC_SCALE AS varchar(5)) + ')'
            ELSE c.DATA_TYPE END,
       c.IS_NULLABLE, t.TABLE_TYPE
FROM INFORMATION_SCHEMA.COLUMNS c
JOIN INFORMATION_SCHEMA.TABLES t ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME
ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION"#;

const PRIMARY_KEYS: &str = r#"
SELECT kcu.TABLE_SCHEMA, kcu.TABLE_NAME, kcu.COLUMN_NAME
FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE kcu
  ON kcu.CONSTRAINT_NAME = tc.CONSTRAINT_NAME AND kcu.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA
WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'"#;

const FOREIGN_KEYS: &str = r#"
SELECT SCHEMA_NAME(t.schema_id), t.name, c.name, SCHEMA_NAME(rt.schema_id), rt.name, rc.name
FROM sys.foreign_key_columns fkc
JOIN sys.tables t ON t.object_id = fkc.parent_object_id
JOIN sys.columns c ON c.object_id = fkc.parent_object_id AND c.column_id = fkc.parent_column_id
JOIN sys.tables rt ON rt.object_id = fkc.referenced_object_id
JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id"#;

// Row counts from partition metadata: instant, unlike COUNT(*) on large tables.
const ROW_ESTIMATES: &str = r#"
SELECT SCHEMA_NAME(t.schema_id), t.name, CAST(SUM(p.rows) AS bigint)
FROM sys.tables t
JOIN sys.partitions p ON p.object_id = t.object_id AND p.index_id IN (0, 1)
GROUP BY t.schema_id, t.name"#;

// The descriptions SSMS and most tools write: minor_id is the column, or 0 for the table.
const DESCRIPTIONS: &str = r#"
SELECT SCHEMA_NAME(o.schema_id), o.name, ep.minor_id, c.name, CAST(ep.value AS nvarchar(4000))
FROM sys.extended_properties ep
JOIN sys.objects o ON o.object_id = ep.major_id
LEFT JOIN sys.columns c ON c.object_id = ep.major_id AND c.column_id = ep.minor_id
WHERE ep.class = 1 AND ep.name = 'MS_Description' AND o.type IN ('U', 'V')"#;

const CHECKS: &str = r#"
SELECT SCHEMA_NAME(t.schema_id), t.name, cc.definition
FROM sys.check_constraints cc
JOIN sys.tables t ON t.object_id = cc.parent_object_id
ORDER BY cc.name"#;

fn text(row: &Row, i: usize) -> String {
    row.try_get::<&str, _>(i)
        .ok()
        .flatten()
        .unwrap_or_default()
        .to_string()
}

pub(super) async fn introspect(pool: &MssqlPool) -> AppResult<Vec<Table>> {
    let mut conn = pool.acquire().await?;
    let client = conn.client();
    let mut query = async |sql: &str| -> AppResult<Vec<Row>> {
        Ok(client.simple_query(sql).await?.into_first_result().await?)
    };

    let cols = query(COLUMNS)
        .await?
        .iter()
        .map(|r| {
            (
                text(r, 0),
                text(r, 1),
                text(r, 2),
                text(r, 3),
                text(r, 4) == "YES",
                text(r, 5),
            )
        })
        .collect();
    let pks = query(PRIMARY_KEYS)
        .await?
        .iter()
        .map(|r| (text(r, 0), text(r, 1), text(r, 2)))
        .collect();
    let fks = query(FOREIGN_KEYS)
        .await?
        .iter()
        .map(|r| {
            (
                text(r, 0),
                text(r, 1),
                text(r, 2),
                text(r, 3),
                text(r, 4),
                text(r, 5),
            )
        })
        .collect();
    let rows = query(ROW_ESTIMATES)
        .await?
        .iter()
        .filter_map(|r| Some(((text(r, 0), text(r, 1)), r.try_get::<i64, _>(2).ok()??)))
        .collect();
    let mut described = Described::default();
    for r in query(DESCRIPTIONS).await? {
        let minor = r.try_get::<i32, _>(2).ok().flatten().unwrap_or(-1);
        let column = r.try_get::<&str, _>(3).ok().flatten().map(str::to_string);
        match column {
            Some(c) => described.column(text(&r, 0), text(&r, 1), c, text(&r, 4)),
            None if minor == 0 => described.table(text(&r, 0), text(&r, 1), text(&r, 4)),
            None => {}
        }
    }
    for r in query(CHECKS).await? {
        described.check(text(&r, 0), text(&r, 1), &text(&r, 2));
    }
    Ok(assemble(
        cols,
        pks,
        fks,
        HashMap::new(),
        rows,
        described,
        true,
    ))
}
