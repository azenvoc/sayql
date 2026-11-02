// Behind a transaction pooler (Supabase on port 6543, PgBouncer), statements outside a transaction can
// land on different server connections. So these run inside one read-only transaction, which keeps them on
// one server connection, after clearing any prepared statements another client left there.
use super::{Described, assemble};
use crate::schema::Table;
use sqlx::PgPool;
use std::collections::HashMap;

pub(super) async fn introspect(pool: &PgPool) -> Result<Vec<Table>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql("SET TRANSACTION READ ONLY; DEALLOCATE ALL")
        .execute(&mut *tx)
        .await?;
    let p = &mut *tx;
    // information_schema has no materialized views, so they come from the catalog. Partitions
    // are left out: the parent table is what queries use, and listing every partition would
    // crowd the AI's schema.
    let cols: Vec<(String, String, String, String, bool, String)> = sqlx::query_as(
        r#"SELECT s, t, c, ty, nullable, kind FROM (
           SELECT c.table_schema::text AS s, c.table_name::text AS t, c.column_name::text AS c,
                  CASE WHEN c.data_type = 'USER-DEFINED' AND c.udt_schema NOT IN ('public', 'pg_catalog')
                       THEN c.udt_schema::text || '.' || c.udt_name::text
                       WHEN c.data_type IN ('USER-DEFINED', 'ARRAY') THEN c.udt_name::text
                       ELSE c.data_type::text END AS ty,
                  c.is_nullable = 'YES' AS nullable, t.table_type::text AS kind, c.ordinal_position::int AS pos
           FROM information_schema.columns c
           JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name
           WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema') AND c.table_schema NOT LIKE 'pg_toast%'
             AND NOT EXISTS (SELECT 1 FROM pg_class pc JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                             WHERE pn.nspname = c.table_schema AND pc.relname = c.table_name AND pc.relispartition)
           UNION ALL
           SELECT n.nspname::text, cl.relname::text, a.attname::text,
                  CASE WHEN ty.typtype IN ('e', 'c', 'd') AND tn.nspname NOT IN ('public', 'pg_catalog')
                       THEN tn.nspname::text || '.' || ty.typname::text
                       WHEN ty.typtype IN ('e', 'c', 'd') OR ty.typcategory = 'A' THEN ty.typname::text
                       ELSE format_type(a.atttypid, NULL) END,
                  NOT a.attnotnull, 'MATERIALIZED VIEW', a.attnum::int
           FROM pg_class cl
           JOIN pg_namespace n ON n.oid = cl.relnamespace
           JOIN pg_attribute a ON a.attrelid = cl.oid AND a.attnum > 0 AND NOT a.attisdropped
           JOIN pg_type ty ON ty.oid = a.atttypid
           JOIN pg_namespace tn ON tn.oid = ty.typnamespace
           WHERE cl.relkind = 'm' AND has_table_privilege(cl.oid, 'SELECT')
           ) cols ORDER BY s, t, pos"#,
    )
    .fetch_all(&mut *p)
    .await?;
    // From the catalog, not information_schema.table_constraints, which hides constraints
    // from roles that only have SELECT on a table: read-only logins would see no keys.
    let pks: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT ns.nspname::text, cl.relname::text, a.attname::text
           FROM pg_constraint con
           JOIN pg_class cl ON cl.oid = con.conrelid
           JOIN pg_namespace ns ON ns.oid = cl.relnamespace
           CROSS JOIN LATERAL unnest(con.conkey) AS k(col)
           JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.col
           WHERE con.contype = 'p'"#,
    )
    .fetch_all(&mut *p)
    .await?;
    let fks: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        r#"SELECT ns.nspname::text, cl.relname::text, a.attname::text, fns.nspname::text, fcl.relname::text, fa.attname::text
           FROM pg_constraint con
           JOIN pg_class cl ON cl.oid = con.conrelid
           JOIN pg_namespace ns ON ns.oid = cl.relnamespace
           JOIN pg_class fcl ON fcl.oid = con.confrelid
           JOIN pg_namespace fns ON fns.oid = fcl.relnamespace
           CROSS JOIN LATERAL unnest(con.conkey, con.confkey) AS k(col, fcol)
           JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.col
           JOIN pg_attribute fa ON fa.attrelid = con.confrelid AND fa.attnum = k.fcol
           WHERE con.contype = 'f' AND con.conparentid = 0"#,
    )
    .fetch_all(&mut *p)
    .await?;
    let enum_rows: Vec<(String, String)> = sqlx::query_as(
        // Keyed like the column types above, so same-named enums in different schemas stay apart.
        r#"SELECT CASE WHEN n.nspname IN ('public', 'pg_catalog') THEN t.typname::text
                       ELSE n.nspname::text || '.' || t.typname::text END,
                  e.enumlabel::text
           FROM pg_type t
           JOIN pg_namespace n ON n.oid = t.typnamespace
           JOIN pg_enum e ON e.enumtypid = t.oid ORDER BY t.oid, e.enumsortorder"#,
    )
    .fetch_all(&mut *p)
    .await?;
    let mut enums: HashMap<String, Vec<String>> = HashMap::new();
    for (t, v) in enum_rows {
        enums.entry(t).or_default().push(v);
    }
    // A partitioned table's own estimate is 0 or -1 on older servers; add up its partitions.
    let est: Vec<(String, String, i64)> = sqlx::query_as(
        r#"SELECT n.nspname::text, c.relname::text,
                  CASE WHEN c.relkind = 'p' THEN
                       (SELECT COALESCE(sum(GREATEST(l.reltuples, 0)), -1)::bigint FROM pg_partition_tree(c.oid) pt
                        JOIN pg_class l ON l.oid = pt.relid WHERE pt.isleaf)
                       ELSE c.reltuples::bigint END
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p', 'm') AND NOT c.relispartition"#,
    )
    .fetch_all(&mut *p)
    .await?;
    // reltuples is -1 until the table is first vacuumed or analyzed: unknown, not empty.
    let rows = est
        .into_iter()
        .filter(|(_, _, n)| *n >= 0)
        .map(|(s, t, n)| ((s, t), n))
        .collect();
    // pg_description is what col_description and obj_description read: objsubid is the
    // column number, or 0 for the table itself.
    let comments: Vec<(String, String, i32, Option<String>, String)> = sqlx::query_as(
        r#"SELECT n.nspname::text, c.relname::text, d.objsubid, a.attname::text, d.description
           FROM pg_description d
           JOIN pg_class c ON c.oid = d.objoid
           JOIN pg_namespace n ON n.oid = c.relnamespace
           LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = d.objsubid AND d.objsubid > 0
                                     AND NOT a.attisdropped
           WHERE d.classoid = 'pg_class'::regclass AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
             AND n.nspname NOT IN ('pg_catalog', 'information_schema')"#,
    )
    .fetch_all(&mut *p)
    .await?;
    let checks: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT ns.nspname::text, cl.relname::text, pg_get_constraintdef(con.oid)
           FROM pg_constraint con
           JOIN pg_class cl ON cl.oid = con.conrelid
           JOIN pg_namespace ns ON ns.oid = cl.relnamespace
           WHERE con.contype = 'c' AND ns.nspname NOT IN ('pg_catalog', 'information_schema')
           ORDER BY con.conname"#,
    )
    .fetch_all(&mut *p)
    .await?;
    let mut described = Described::default();
    for (s, t, sub, c, text) in comments {
        match c {
            Some(c) => described.column(s, t, c, text),
            None if sub == 0 => described.table(s, t, text),
            None => {}
        }
    }
    for (s, t, def) in checks {
        described.check(s, t, &def);
    }
    tx.rollback().await?;
    Ok(assemble(cols, pks, fks, enums, rows, described, true))
}
