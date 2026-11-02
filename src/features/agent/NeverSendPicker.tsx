import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, Plus, Search, X } from "lucide-react";
import { commands, type DatabaseList, type Schema, type Table, type Target } from "../../api";
import { Alert, Button, Select, Spinner, TextInput } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import {
  addEntries,
  entryFor,
  inSchema,
  namesColumn,
  parseEntry,
  splitTyped,
  suggestedColumns,
  type NeverSendEntry,
} from "./neverSend";
import styles from "./NeverSendPicker.module.css";

export interface NeverSendPickerProps {
  /** The database whose tables to show; never-send itself applies to the whole connection. */
  target: Target;
  /** Entries as stored: `table.column` or a bare `column`. */
  value: string[];
  onChange: (value: string[]) => void;
  /** Columns the team admin keeps out: shown ticked and locked. */
  locked?: string[];
  /** Label for the list of chips, e.g. "Never send these columns". */
  label: string;
  /** Focus the table filter once the tables load (when the picker is the dialog's only content). */
  autoFocus?: boolean;
}

type Loaded =
  { state: "loading" } | { state: "ready"; schema: Schema } | { state: "failed"; error: string };

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** Picks the columns that never leave SayQL from the database's tables, with likely personal ones suggested. */
export function NeverSendPicker({
  target,
  value,
  onChange,
  locked = [],
  label,
  autoFocus = false,
}: NeverSendPickerProps) {
  const [database, setDatabase] = useState<string | null>(target.database);
  const [databases, setDatabases] = useState<DatabaseList | null>(null);
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });

  useEffect(() => {
    let live = true;
    // Through a promise, so a missing or throwing command lands in `failed` too.
    Promise.resolve()
      .then(() => commands.listDatabases(target.connectionId))
      .then(
        (list) => live && setDatabases(list && list.databases.length > 1 ? list : null),
        () => live && setDatabases(null),
      );
    return () => {
      live = false;
    };
  }, [target.connectionId]);

  useEffect(() => {
    let live = true;
    setLoaded({ state: "loading" });
    Promise.resolve()
      .then(() => commands.openDatabase({ connectionId: target.connectionId, database }))
      .then(
        (schema) =>
          live &&
          setLoaded(
            schema && Array.isArray(schema.tables)
              ? { state: "ready", schema }
              : { state: "failed", error: "No schema" },
          ),
        (e) => live && setLoaded({ state: "failed", error: errorText(e) }),
      );
    return () => {
      live = false;
    };
  }, [target.connectionId, database]);

  const entries = useMemo(() => value.map(parseEntry), [value]);
  const lockedEntries = useMemo(() => locked.map(parseEntry), [locked]);

  if (loaded.state === "failed")
    return <TypedFallback label={label} value={value} onChange={onChange} error={loaded.error} />;

  const schema = loaded.state === "ready" ? loaded.schema : null;
  const remove = (raw: string) => onChange(value.filter((v) => v !== raw));

  return (
    <div className={styles.picker}>
      <ChipList label={label} entries={entries} schema={schema} onRemove={remove} />
      {databases && (
        <div className={styles.databaseRow}>
          <span className={styles.subtle}>Tables from</span>
          <Select
            aria-label="Database to pick columns from"
            className={styles.databaseSelect}
            value={database ?? databases.current ?? databases.databases[0]}
            options={databases.databases.map((d) => ({ value: d, label: d }))}
            onChange={setDatabase}
          />
          <span className={styles.subtle}>
            Never send applies to every database on this connection.
          </span>
        </div>
      )}
      {!schema ? (
        <div className={styles.loading}>
          <Spinner size={14} /> Loading tables…
        </div>
      ) : (
        <SchemaLists
          schema={schema}
          entries={entries}
          locked={lockedEntries}
          value={value}
          onChange={onChange}
          autoFocus={autoFocus}
        />
      )}
      <BareNameInput onAdd={(name) => onChange(addEntries(value, [name]))} />
      <p className={styles.subtle}>
        A picked column is kept out of that table only; a name added for every table is kept out
        wherever it appears. Queries that use one are refused.
      </p>
    </div>
  );
}

/** The chosen entries as removable chips. */
function ChipList({
  label,
  entries,
  schema,
  onRemove,
}: {
  label: string;
  entries: NeverSendEntry[];
  schema: Schema | null;
  onRemove: (raw: string) => void;
}) {
  return (
    <div className={styles.chipsBlock}>
      <span className={styles.label} id="never-send-chips">
        {label} <span className={styles.subtle}>({entries.length})</span>
      </span>
      <ul className={styles.chips} aria-labelledby="never-send-chips">
        {entries.length === 0 && (
          <li className={styles.subtle}>
            None yet. Tick columns below, or add the suggested ones.
          </li>
        )}
        {entries.map((e) => {
          const missing = schema !== null && !inSchema(e, schema);
          return (
            <li key={e.raw} className={cx(styles.chip, missing && styles.chipMissing)}>
              <EntryLabel entry={e} />
              {missing && (
                <span
                  className={styles.chipNote}
                  title="No such column in this database. Kept: it may be in another database, or added later."
                >
                  not found
                </span>
              )}
              <button
                type="button"
                className={styles.chipRemove}
                aria-label={`Remove ${e.raw}`}
                title="Remove"
                onClick={() => onRemove(e.raw)}
              >
                <X size={12} />
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** `table.column`, or `any table · column` for a bare name. */
export function EntryLabel({ entry }: { entry: NeverSendEntry | string }) {
  const e = typeof entry === "string" ? parseEntry(entry) : entry;
  return e.table === null ? (
    <span className={styles.entry}>
      <span className={styles.anyTable}>any table · </span>
      {e.column}
    </span>
  ) : (
    <span className={styles.entry}>{e.raw}</span>
  );
}

function SchemaLists({
  schema,
  entries,
  locked,
  value,
  onChange,
  autoFocus,
}: {
  schema: Schema;
  entries: NeverSendEntry[];
  locked: NeverSendEntry[];
  value: string[];
  onChange: (v: string[]) => void;
  autoFocus: boolean;
}) {
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState<Set<string>>(new Set());
  const suggestions = useMemo(() => suggestedColumns(schema), [schema]);

  // How a column stands: picked here, covered by a bare name or the team admin, or free.
  const status = (table: string, column: string): { checked: boolean; lockedBy: string | null } => {
    const admin = locked.find(
      (e) => (e.table === null || same(e.table, table)) && same(e.column, column),
    );
    if (admin) return { checked: true, lockedBy: "Set by your team admin" };
    if (entries.some((e) => namesColumn(e, table, column)))
      return { checked: true, lockedBy: null };
    const bare = entries.find((e) => e.table === null && same(e.column, column));
    if (bare) return { checked: true, lockedBy: `Kept out in every table by “${bare.column}”` };
    return { checked: false, lockedBy: null };
  };
  const setColumns = (cols: { table: string; column: string }[], on: boolean) => {
    if (on)
      onChange(
        addEntries(
          value,
          cols.map((c) => entryFor(c.table, c.column)),
        ),
      );
    else
      onChange(
        value.filter((raw) => !cols.some((c) => namesColumn(parseEntry(raw), c.table, c.column))),
      );
  };

  const open_ = (key: string, on: boolean) =>
    setOpen((s) => {
      const n = new Set(s);
      if (on) n.add(key);
      else n.delete(key);
      return n;
    });

  const q = filter.trim().toLowerCase();
  const shown = schema.tables
    .map((t) => {
      if (!q || t.name.toLowerCase().includes(q)) return { t, cols: t.columns, byColumn: false };
      const cols = t.columns.filter((c) => c.name.toLowerCase().includes(q));
      return cols.length ? { t, cols, byColumn: true } : null;
    })
    .filter((x): x is { t: Table; cols: Table["columns"]; byColumn: boolean } => x !== null);

  const pending = suggestions.filter((s) => !status(s.table, s.column).checked);

  return (
    <>
      {suggestions.length > 0 && (
        <section className={styles.section} aria-labelledby="never-send-suggested">
          <div className={styles.sectionHead}>
            <h3 id="never-send-suggested" className={styles.sectionTitle}>
              Suggested
            </h3>
            <span className={styles.subtle}>Likely personal or sensitive, by name</span>
            <span className={styles.spacer} />
            <Button
              size="sm"
              icon={Plus}
              disabled={pending.length === 0}
              onClick={() => setColumns(pending, true)}
            >
              {pending.length === 0
                ? "All suggested added"
                : `Add all suggested (${pending.length})`}
            </Button>
          </div>
          <ul className={cx(styles.list, styles.suggested)}>
            {suggestions.map((s) => {
              const st = status(s.table, s.column);
              return (
                <li key={`${s.table}.${s.column}`}>
                  <ColumnRow
                    label={
                      <>
                        <span className={styles.tableName}>{s.table}.</span>
                        {s.column}
                      </>
                    }
                    aria={`${s.table}.${s.column}`}
                    type={s.type}
                    kind={s.kind}
                    checked={st.checked}
                    lockedBy={st.lockedBy}
                    onChange={(on) => setColumns([s], on)}
                  />
                </li>
              );
            })}
          </ul>
        </section>
      )}
      <section className={styles.section} aria-labelledby="never-send-tables">
        <div className={styles.sectionHead}>
          <h3 id="never-send-tables" className={styles.sectionTitle}>
            All tables
          </h3>
          <span className={styles.subtle}>{schema.tables.length}</span>
        </div>
        <div className={styles.filter}>
          <Search size={13} className={styles.filterIcon} aria-hidden />
          <TextInput
            type="search"
            className={styles.filterInput}
            placeholder="Filter tables and columns"
            aria-label="Filter tables and columns"
            autoFocus={autoFocus}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
        <ul className={cx(styles.list, styles.tables)} aria-label="Tables">
          {shown.length === 0 && (
            <li className={cx(styles.subtle, styles.empty)}>
              {schema.tables.length
                ? "No tables or columns match."
                : "This database has no tables."}
            </li>
          )}
          {shown.map(({ t, cols, byColumn }) => {
            const key = t.schema ? `${t.schema}.${t.name}` : t.name;
            const expanded = byColumn || open.has(key);
            const states = t.columns.map((c) => status(t.name, c.name));
            const free = t.columns.filter((_, i) => !states[i].lockedBy);
            const picked = states.filter((s) => s.checked).length;
            const all = picked === t.columns.length && t.columns.length > 0;
            return (
              <li key={key}>
                <div className={styles.tableRow}>
                  <button
                    type="button"
                    className={styles.caret}
                    aria-expanded={expanded}
                    aria-label={`${expanded ? "Collapse" : "Expand"} ${t.name}`}
                    onClick={() => open_(key, !expanded)}
                  >
                    {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  </button>
                  <TriCheckbox
                    label={`All columns of ${t.name}`}
                    checked={all}
                    mixed={picked > 0 && !all}
                    disabled={free.length === 0}
                    onChange={(on) =>
                      setColumns(
                        free.map((c) => ({ table: t.name, column: c.name })),
                        on,
                      )
                    }
                  />
                  <button
                    type="button"
                    className={styles.tableLabel}
                    onClick={() => open_(key, !expanded)}
                    tabIndex={-1}
                  >
                    <span className={cx(styles.mono, t.kind !== "table" && styles.view)}>
                      {key}
                    </span>
                    {t.kind !== "table" && <span className={styles.type}>{t.kind}</span>}
                  </button>
                  <span className={styles.count}>
                    {picked ? `${picked} of ${t.columns.length}` : t.columns.length}
                  </span>
                </div>
                {expanded && (
                  <ul className={styles.columns}>
                    {cols.map((c) => {
                      const st = status(t.name, c.name);
                      return (
                        <li key={c.name}>
                          <ColumnRow
                            label={c.name}
                            aria={`${t.name}.${c.name}`}
                            type={c.data_type}
                            checked={st.checked}
                            lockedBy={st.lockedBy}
                            onChange={(on) => setColumns([{ table: t.name, column: c.name }], on)}
                          />
                        </li>
                      );
                    })}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      </section>
    </>
  );
}

function ColumnRow({
  label,
  aria,
  type,
  kind,
  checked,
  lockedBy,
  onChange,
}: {
  label: ReactNode;
  aria: string;
  type: string;
  kind?: string;
  checked: boolean;
  lockedBy: string | null;
  onChange: (on: boolean) => void;
}) {
  return (
    <label
      className={cx(styles.columnRow, lockedBy && styles.lockedRow)}
      title={lockedBy ?? undefined}
    >
      <input
        type="checkbox"
        aria-label={aria}
        checked={checked}
        disabled={!!lockedBy}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className={styles.mono}>{label}</span>
      {kind && <span className={styles.kind}>{kind}</span>}
      <span className={styles.spacer} />
      {lockedBy && (
        <span className={styles.lockedNote}>
          {lockedBy.startsWith("Set") ? "team admin" : "any table"}
        </span>
      )}
      <span className={styles.type}>{type}</span>
    </label>
  );
}

function TriCheckbox({
  label,
  checked,
  mixed,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  mixed: boolean;
  disabled: boolean;
  onChange: (on: boolean) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = mixed;
  }, [mixed]);
  return (
    <input
      ref={ref}
      type="checkbox"
      className={styles.tableCheck}
      aria-label={label}
      aria-checked={mixed ? "mixed" : checked}
      checked={checked}
      disabled={disabled}
      // A partly picked table picks the rest first; a second click clears it.
      onChange={() => onChange(!checked)}
    />
  );
}

/** For the power user: a column name kept out in every table. */
function BareNameInput({ onAdd }: { onAdd: (name: string) => void }) {
  const [name, setName] = useState("");
  const add = () => {
    const n = name.trim();
    if (!n) return;
    onAdd(n);
    setName("");
  };
  return (
    <div className={styles.bare}>
      <TextInput
        mono
        aria-label="Add a column name for every table"
        placeholder="Add a column name for every table, e.g. diagnosis"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            add();
          }
        }}
      />
      <Button size="sm" disabled={!name.trim()} onClick={add}>
        Add
      </Button>
    </div>
  );
}

/** When the tables can't be loaded (the connection is offline): type the columns, as before. */
function TypedFallback({
  label,
  value,
  onChange,
  error,
}: {
  label: string;
  value: string[];
  onChange: (v: string[]) => void;
  error: string;
}) {
  // Keep the text as typed; the list updates on every change.
  const [text, setText] = useState(value.join("\n"));
  return (
    <div className={styles.picker}>
      <Alert tone="info">
        SayQL couldn't load this connection's tables ({error}), so type the columns instead.
      </Alert>
      <label className={styles.label} htmlFor="never-send-typed">
        {label}
      </label>
      <textarea
        id="never-send-typed"
        className={styles.textarea}
        rows={4}
        value={text}
        placeholder={"patients.notes\ndiagnosis"}
        onChange={(e) => {
          setText(e.target.value);
          onChange(splitTyped(e.target.value));
        }}
      />
      <span className={styles.subtle}>
        One per line, as column (every table) or table.column. Queries that use them are refused.
      </span>
    </div>
  );
}
