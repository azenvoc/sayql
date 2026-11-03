import { useMemo, useRef, useState, type MouseEvent } from "react";
import {
  LayoutDashboard,
  Bot,
  CircleCheck,
  Copy,
  Database,
  Eye,
  FileCode2,
  FilePlus2,
  Folder,
  FolderOpen,
  Hash,
  KeyRound,
  Link2,
  Pencil,
  Plug,
  Plus,
  RefreshCw,
  Search,
  Server,
  Sparkles,
  StickyNote,
  Table2,
  Trash2,
  TriangleAlert,
  Type,
  Users,
  Wand2,
  X,
} from "lucide-react";
import type { ConnectionConfig, DatabaseList, DbNote, Schema, Table } from "../../api";
import {
  ContextMenu,
  IconButton,
  Panel,
  Spinner,
  TextInput,
  TreeRow,
  useContextMenu,
  type MenuEntry,
} from "../../components/ui";
import { usePersistentState } from "../../hooks/usePersistentState";
import { cx } from "../../lib/cx";
import { DATABASE_KINDS, ProdBadge } from "../connections";
import { countRowsSql, qualifiedName, tableRef, templateSql, type SqlTemplate } from "../schema";
import styles from "./DatabaseNavigator.module.css";

export interface DatabaseNavigatorProps {
  connections: ConnectionConfig[];
  activeId: string | null;
  /** The server's databases per connection; `null` for SQLite, absent while loading. */
  databases: Record<string, DatabaseList | null>;
  /** The open database on the active connection. */
  databaseName: string | null;
  schema: Schema | null;
  loading: boolean;
  /** Whether the active connection is connected; restored tabs wait for the person to connect. */
  connected: boolean;
  /** Connects the active connection. */
  onConnect: () => void;
  onOpen: (id: string) => void;
  onOpenDatabase: (database: string) => void;
  onAdd: () => void;
  onEdit: (c: ConnectionConfig) => void;
  /** Asks before deleting the connection. */
  onDeleteConnection: (c: ConnectionConfig) => void;
  onRefresh: () => void;
  /** Double-click on a table. */
  onPreviewTable: (t: Table) => void;
  /** A new SQL tab on the connection's database (`null`: its default database). */
  onNewTab: (connectionId: string, database: string | null) => void;
  /** Opens `sql` in a new tab on the open database; `run` runs it straight away (read-only SQL only). */
  onOpenSql: (title: string, sql: string, run: boolean) => void;
  /** Tables picked as AI context (qualified names). */
  selectedTables: string[];
  onSelectTables: (names: string[]) => void;
  scripts: NavigatorScript[];
  onOpenScript: (id: string) => void;
  onDeleteScript: (id: string) => void;
  /** Database notes of the open database (facts agent mode, the Ask box and MCP use). */
  notes: DbNote[];
  /** Edits a note, or (`null`) adds one to the open database. */
  onEditNote: (note: DbNote | null) => void;
  onDeleteNote: (note: DbNote) => void;
  /** Reads the schema again and runs every note's check. */
  onCheckNotes?: () => void;
  /**
   * The notes are the team's, from the team server: shown read-only, and `onEditNote(null)`
   * suggests a new one to the team's knowledge owners instead of saving it here.
   */
  sharedNotes?: boolean;
  /** Notes AI apps suggested for the open database, waiting for the person. */
  suggestedNotes?: number;
  /** Opens the suggested notes to accept or dismiss. */
  onReviewSuggestions?: () => void;
  /** Opens "Teach SayQL your database"; set while it should show under the open database. */
  onTeach?: () => void;
  /** Hides "Teach SayQL your database" for good. */
  onHideTeach?: () => void;
  /** Agent mode's settings for a connection: what it may send to the AI. */
  onAgentSettings: (c: ConnectionConfig) => void;
  /** The connection's overview: what it is, what can reach an AI, AI apps and your queries. */
  onOverview: (c: ConnectionConfig) => void;
  /** Opens "Sign in to your team"; set only while not signed in to a team. */
  onTeamSignIn?: () => void;
}

/** A saved script as listed in the navigator. */
export interface NavigatorScript {
  id: string;
  name: string;
  /** Where it runs, e.g. "Sales". */
  location: string;
  /** Its connection was deleted, so it can't be opened. */
  orphaned: boolean;
  /** It runs on the database that's open now. */
  here: boolean;
}

/** The hint under a team's shared notes. */
export const SHARED_HINT = "Shared by your team. Owners approve changes on the team server.";
const SUGGEST = "Suggest a note…";

/** A note's tooltip: its text, and why it may be out of date or when it was checked. */
function noteTitle(n: DbNote, shared?: boolean): string {
  if (n.suspect)
    return `${n.text}\n\nMay be out of date: ${n.suspect.reason}.${shared ? "" : " Click to review."}`;
  if (n.verifiedAt)
    return `${n.text}\n\nChecked against the data ${new Date(n.verifiedAt).toLocaleDateString()}.`;
  return n.text;
}

/**
 * Connection → Databases → database → Tables/Views → table → columns, like DBeaver's
 * Database Navigator. SQLite skips the Databases level: a file is one database.
 */
export function DatabaseNavigator(p: DatabaseNavigatorProps) {
  const [filter, setFilter] = useState("");
  /** Saved scripts: only the open database's, or all of them. */
  const [scriptsShown, setScriptsShown] = usePersistentState<"here" | "all">(
    "sayql.scriptsShown",
    "here",
  );
  const [teamHintHidden, setTeamHintHidden] = usePersistentState("sayql.teamHintHidden", false);
  const anyHere = p.scripts.some((s) => s.here);
  const shownScripts = scriptsShown === "all" ? p.scripts : p.scripts.filter((s) => s.here);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [expandedTables, setExpandedTables] = useState<Set<string>>(new Set());
  const query = filter.trim().toLowerCase();

  const { tables, views } = useMemo(() => {
    const all = (p.schema?.tables ?? []).filter(
      (t) =>
        !query ||
        t.name.toLowerCase().includes(query) ||
        t.columns.some((c) => c.name.toLowerCase().includes(query)),
    );
    // Materialized views go with views: both are queried like tables but are not written to.
    return {
      tables: all.filter((t) => t.kind === "table"),
      views: all.filter((t) => t.kind !== "table"),
    };
  }, [p.schema, query]);

  // Click selects one table, ⌘/Ctrl-click adds or removes one, ⇧-click selects a range.
  const anchor = useRef<string | null>(null);
  const order = [...tables, ...views].map(qualifiedName);
  const select = (key: string, e: MouseEvent) => {
    const current = p.selectedTables;
    if (e.shiftKey && anchor.current && order.includes(anchor.current)) {
      const [a, b] = [order.indexOf(anchor.current), order.indexOf(key)].sort((x, y) => x - y);
      p.onSelectTables(order.slice(a, b + 1));
      return;
    }
    anchor.current = key;
    if (e.metaKey || e.ctrlKey) {
      p.onSelectTables(
        current.includes(key) ? current.filter((k) => k !== key) : [...current, key],
      );
    } else {
      p.onSelectTables([key]); // also safe for the two clicks of a double-click preview
    }
  };

  const menu = useContextMenu();
  const copy = (text: string) => void navigator.clipboard?.writeText(text);

  const connectionMenu = (c: ConnectionConfig): MenuEntry[] => {
    const active = c.id === p.activeId;
    // A team connection is managed by the team admin: no editing or deleting it here.
    if (c.team) {
      return [
        {
          label: active && p.connected ? "Connected" : "Connect",
          icon: Plug,
          disabled: active && p.connected,
          onSelect: () => p.onOpen(c.id),
        },
        {
          label: "New SQL tab",
          icon: FilePlus2,
          shortcut: active ? "⌘T" : undefined,
          onSelect: () => p.onNewTab(c.id, null),
        },
        {
          label: "Refresh",
          icon: RefreshCw,
          disabled: !active || !p.connected || p.loading,
          onSelect: p.onRefresh,
        },
        "separator",
        { label: "Overview…", icon: LayoutDashboard, onSelect: () => p.onOverview(c) },
        { label: "Investigate settings…", icon: Bot, onSelect: () => p.onAgentSettings(c) },
        {
          label: active && p.sharedNotes ? SUGGEST : "Add database note…",
          icon: StickyNote,
          disabled: !active || !p.schema,
          onSelect: () => p.onEditNote(null),
        },
        { label: "Copy name", icon: Copy, onSelect: () => copy(c.name) },
      ];
    }
    return [
      {
        label: active && p.connected ? "Connected" : "Connect",
        icon: Plug,
        disabled: active && p.connected,
        onSelect: () => p.onOpen(c.id),
      },
      {
        label: "New SQL tab",
        icon: FilePlus2,
        shortcut: active ? "⌘T" : undefined,
        onSelect: () => p.onNewTab(c.id, null),
      },
      {
        label: "Refresh",
        icon: RefreshCw,
        disabled: !active || !p.connected || p.loading,
        onSelect: p.onRefresh,
      },
      "separator",
      { label: "Overview…", icon: LayoutDashboard, onSelect: () => p.onOverview(c) },
      { label: "Edit connection…", icon: Pencil, onSelect: () => p.onEdit(c) },
      { label: "Investigate settings…", icon: Bot, onSelect: () => p.onAgentSettings(c) },
      {
        label: "Add database note…",
        icon: StickyNote,
        disabled: !active || !p.schema,
        onSelect: () => p.onEditNote(null),
      },
      { label: "Copy name", icon: Copy, onSelect: () => copy(c.name) },
      "separator",
      {
        label: "Delete connection…",
        icon: Trash2,
        danger: true,
        onSelect: () => p.onDeleteConnection(c),
      },
    ];
  };

  const databaseMenu = (c: ConnectionConfig, name: string): MenuEntry[] => {
    const current = name === p.databaseName;
    return [
      {
        label: current ? "Open" : "Open database",
        icon: FolderOpen,
        disabled: current,
        onSelect: () => p.onOpenDatabase(name),
      },
      { label: "New SQL tab", icon: FilePlus2, onSelect: () => p.onNewTab(c.id, name) },
      { label: "Refresh", icon: RefreshCw, disabled: !current || p.loading, onSelect: p.onRefresh },
      {
        label: current && p.sharedNotes ? SUGGEST : "Add database note…",
        icon: StickyNote,
        disabled: !current,
        onSelect: () => p.onEditNote(null),
      },
      "separator",
      { label: "Copy name", icon: Copy, onSelect: () => copy(name) },
    ];
  };

  const tableMenu = (t: Table): MenuEntry[] => {
    const dialect = p.schema!.dialect;
    const key = qualifiedName(t);
    const picked = p.selectedTables.includes(key);
    const template = (kind: SqlTemplate, label: string): MenuEntry => ({
      label,
      onSelect: () => p.onOpenSql(`${label} ${t.name}`, templateSql(t, dialect, kind), false),
    });
    return [
      { label: "View data", icon: Table2, onSelect: () => p.onPreviewTable(t) },
      {
        label: "Count rows",
        icon: Hash,
        onSelect: () => p.onOpenSql(`Count ${t.name}`, countRowsSql(t, dialect), true),
      },
      {
        label: "Generate SQL",
        icon: Wand2,
        // Views are usually read-only, so they only offer SELECT.
        submenu:
          t.kind !== "table"
            ? [template("select", "SELECT")]
            : [
                template("select", "SELECT"),
                template("insert", "INSERT"),
                template("update", "UPDATE"),
                template("delete", "DELETE"),
              ],
      },
      "separator",
      {
        label: picked ? "Remove from AI context" : "Add to AI context",
        icon: Sparkles,
        onSelect: () =>
          p.onSelectTables(
            picked ? p.selectedTables.filter((k) => k !== key) : [...p.selectedTables, key],
          ),
      },
      { label: "Copy name", icon: Copy, onSelect: () => copy(t.name) },
      { label: "Copy as SQL reference", onSelect: () => copy(tableRef(t, dialect)) },
      "separator",
      { label: "Refresh", icon: RefreshCw, disabled: p.loading, onSelect: p.onRefresh },
    ];
  };

  const columnMenu = (t: Table, column: string): MenuEntry[] => [
    { label: "Copy name", icon: Copy, onSelect: () => copy(column) },
    { label: "Copy with table name", onSelect: () => copy(`${t.name}.${column}`) },
  ];

  const scriptMenu = (sc: NavigatorScript): MenuEntry[] => [
    {
      label: "Open",
      icon: FolderOpen,
      disabled: sc.orphaned,
      onSelect: () => p.onOpenScript(sc.id),
    },
    { label: "Copy name", icon: Copy, onSelect: () => copy(sc.name) },
    "separator",
    {
      label: "Delete script…",
      icon: Trash2,
      danger: true,
      onSelect: () => p.onDeleteScript(sc.id),
    },
  ];

  const doubtful = p.notes.filter((n) => n.suspect).length;

  const noteMenu = (n: DbNote): MenuEntry[] =>
    p.sharedNotes
      ? [
          { label: "Copy", icon: Copy, onSelect: () => copy(n.text) },
          { label: SUGGEST, icon: StickyNote, onSelect: () => p.onEditNote(null) },
        ]
      : [
          {
            label: n.suspect ? "Review note…" : "Edit note…",
            icon: Pencil,
            onSelect: () => p.onEditNote(n),
          },
          { label: "Copy", icon: Copy, onSelect: () => copy(n.text) },
          "separator",
          { label: "Delete note", icon: Trash2, danger: true, onSelect: () => p.onDeleteNote(n) },
        ];

  const toggle = (set: Set<string>, key: string) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  };

  const renderTable = (t: Table, depth: number) => {
    const key = qualifiedName(t);
    const open = !!query || expandedTables.has(key);
    const fks = new Map(t.foreign_keys.map((f) => [f.column, f]));
    return (
      <div key={key} role="group">
        <TreeRow
          depth={depth}
          expanded={open}
          icon={t.kind !== "table" ? <Eye size={14} /> : <Table2 size={14} />}
          label={<span className={cx(styles.mono, t.kind !== "table" && styles.view)}>{key}</span>}
          meta={
            t.row_estimate != null && t.row_estimate >= 0
              ? t.row_estimate.toLocaleString()
              : undefined
          }
          selected={p.selectedTables.includes(key)}
          title="Click to use as AI context (⌘-click for several, ⇧-click for a range) · double-click to preview rows"
          onClick={(e) => select(key, e)}
          onToggle={() => setExpandedTables((s) => toggle(s, key))}
          onDoubleClick={() => p.onPreviewTable(t)}
          onContextMenu={(e) => menu.open(e, tableMenu(t))}
        />
        {open &&
          t.columns.map((c) => {
            const fk = fks.get(c.name);
            const icon = c.primary_key ? (
              <KeyRound size={13} className={styles.pk} />
            ) : fk ? (
              <Link2 size={13} className={styles.fk} />
            ) : (
              <Type size={13} />
            );
            return (
              <TreeRow
                key={c.name}
                depth={depth + 1}
                icon={icon}
                label={<span className={styles.mono}>{c.name}</span>}
                meta={c.data_type}
                title={
                  fk
                    ? `References ${fk.ref_table}.${fk.ref_column}`
                    : c.nullable
                      ? "Nullable"
                      : "Not null"
                }
                onContextMenu={(e) => menu.open(e, columnMenu(t, c.name))}
              />
            );
          })}
      </div>
    );
  };

  const renderFolder = (id: string, label: string, items: Table[], depth: number) => {
    if (items.length === 0) return null;
    const open = !!query || !collapsed.has(id);
    return (
      <div role="group">
        <TreeRow
          depth={depth}
          expanded={open}
          icon={<Folder size={14} />}
          label={label}
          meta={items.length}
          onClick={() => setCollapsed((s) => toggle(s, id))}
        />
        {open && items.map((t) => renderTable(t, depth + 1))}
      </div>
    );
  };

  /** Tables and Views of the open schema, starting at `depth`. */
  const renderSchema = (key: string, depth: number) =>
    p.schema && (
      <>
        {renderFolder(`${key}:tables`, "Tables", tables, depth)}
        {renderFolder(`${key}:views`, "Views", views, depth)}
        {(p.notes.length > 0 || !!p.suggestedNotes || p.sharedNotes) && (
          <div role="group">
            <TreeRow
              depth={depth}
              expanded={!collapsed.has(`${key}:notes`)}
              icon={<StickyNote size={14} />}
              label="Notes"
              meta={
                doubtful > 0 ? (
                  <span className={styles.doubtfulBadge} title={`${doubtful} may be out of date`}>
                    {doubtful}
                  </span>
                ) : (
                  p.notes.length || undefined
                )
              }
              title={
                p.sharedNotes
                  ? SHARED_HINT
                  : "Facts about this database that the Ask box, Investigate and MCP use"
              }
              actions={
                p.onCheckNotes &&
                !p.sharedNotes &&
                p.notes.length > 0 && (
                  <IconButton
                    icon={RefreshCw}
                    label="Check notes against the database now"
                    size="sm"
                    onClick={p.onCheckNotes}
                  />
                )
              }
              onClick={() => setCollapsed((s) => toggle(s, `${key}:notes`))}
            />
            {!!p.suggestedNotes && (
              <TreeRow
                depth={depth + 1}
                icon={<Sparkles size={13} className={styles.suggested} />}
                label={<span className={styles.suggested}>Suggested notes and fixes</span>}
                meta={<span className={styles.badge}>{p.suggestedNotes}</span>}
                title="Notes and fixes suggested by SayQL (when the schema changed), Investigate, or an AI app such as Claude Code. Review them to accept or dismiss."
                onClick={p.onReviewSuggestions}
              />
            )}
            {p.sharedNotes && !collapsed.has(`${key}:notes`) && (
              <>
                <TreeRow
                  depth={depth + 1}
                  label={<span className={styles.message}>{SHARED_HINT}</span>}
                />
                <TreeRow
                  depth={depth + 1}
                  icon={<StickyNote size={13} className={styles.suggested} />}
                  label={<span className={styles.suggested}>{SUGGEST}</span>}
                  title="Your team's knowledge owners approve it on the team server."
                  onClick={() => p.onEditNote(null)}
                />
              </>
            )}
            {!collapsed.has(`${key}:notes`) &&
              p.notes.map((n) => (
                <TreeRow
                  key={n.id}
                  depth={depth + 1}
                  icon={
                    n.suspect ? (
                      <TriangleAlert
                        size={13}
                        className={styles.doubtful}
                        aria-label="May be out of date"
                      />
                    ) : n.verifiedAt ? (
                      <CircleCheck size={13} className={styles.verified} aria-label="Checked" />
                    ) : undefined
                  }
                  label={
                    <span className={n.suspect ? styles.noteDoubtful : styles.note}>{n.text}</span>
                  }
                  title={noteTitle(n, p.sharedNotes)}
                  onClick={n.suspect && !p.sharedNotes ? () => p.onEditNote(n) : undefined}
                  onDoubleClick={p.sharedNotes ? undefined : () => p.onEditNote(n)}
                  onContextMenu={(e) => menu.open(e, noteMenu(n))}
                />
              ))}
          </div>
        )}
        {p.onTeach && (
          <TreeRow
            depth={depth}
            icon={<Wand2 size={14} className={styles.suggested} />}
            label={<span className={styles.suggested}>Teach SayQL this database</span>}
            title="Let Claude Code or Cursor read your code and suggest notes: which tables are retired, what a status means."
            actions={
              p.onHideTeach && (
                <IconButton icon={X} label="Don't show again" size="sm" onClick={p.onHideTeach} />
              )
            }
            onClick={p.onTeach}
          />
        )}
        {tables.length + views.length === 0 && (
          <TreeRow
            depth={depth}
            label={<span className={styles.message}>{query ? "No matches" : "No tables"}</span>}
          />
        )}
      </>
    );

  const renderDatabases = (c: ConnectionConfig, list: DatabaseList) => {
    const folder = `${c.id}:databases`;
    const open = !collapsed.has(folder);
    return (
      <div role="group">
        <TreeRow
          depth={1}
          expanded={open}
          icon={<Folder size={14} />}
          label="Databases"
          meta={list.databases.length}
          onClick={() => setCollapsed((s) => toggle(s, folder))}
        />
        {open &&
          list.databases.map((name) => {
            const current = name === p.databaseName;
            return (
              <div key={name} role="group">
                <TreeRow
                  depth={2}
                  expanded={current}
                  selected={current}
                  icon={<Database size={14} />}
                  label={<span className={cx(current && styles.connLabel)}>{name}</span>}
                  meta={current && p.loading ? <Spinner size={12} /> : undefined}
                  title={current ? undefined : "Open this database"}
                  onClick={() => p.onOpenDatabase(name)}
                  onContextMenu={(e) => menu.open(e, databaseMenu(c, name))}
                />
                {current && renderSchema(`${c.id}/${name}`, 3)}
              </div>
            );
          })}
      </div>
    );
  };

  const own = p.connections.filter((c) => !c.team);
  const team = p.connections.filter((c) => c.team);

  const renderConnection = (c: ConnectionConfig) => {
    const active = c.id === p.activeId;
    const list = p.databases[c.id];
    return (
      <div key={c.id} role="group">
        <TreeRow
          depth={0}
          expanded={active}
          selected={active}
          icon={
            c.kind === "sqlite" ? (
              <Database size={14} color={DATABASE_KINDS[c.kind].color} />
            ) : (
              <Server size={14} color={DATABASE_KINDS[c.kind].color} />
            )
          }
          label={
            <span className={styles.connName}>
              <span className={styles.connLabel}>{c.name}</span>
              <ProdBadge connection={c} />
            </span>
          }
          meta={active && p.loading && !list ? <Spinner size={12} /> : DATABASE_KINDS[c.kind].short}
          // Team connections show no host or password, and can't be edited here.
          title={
            c.team
              ? [c.team.database, c.team.notes, "Shared by your team admin"]
                  .filter(Boolean)
                  .join(" · ")
              : undefined
          }
          actions={
            c.team ? undefined : (
              <IconButton
                icon={Pencil}
                label="Edit connection"
                size="sm"
                onClick={() => p.onEdit(c)}
              />
            )
          }
          onClick={() => p.onOpen(c.id)}
          onContextMenu={(e) => menu.open(e, connectionMenu(c))}
        />
        {/* Not connected yet (SayQL never connects on its own at start): one click connects. */}
        {active && !p.connected && (
          <TreeRow
            depth={1}
            icon={<Plug size={14} />}
            label={<span className={styles.message}>Not connected</span>}
            title={`Connect to ${c.name}`}
            meta={
              <button
                type="button"
                className={styles.connect}
                onClick={(e) => {
                  e.stopPropagation();
                  p.onConnect();
                }}
              >
                Connect
              </button>
            }
            onClick={p.onConnect}
          />
        )}
        {/* Servers list their databases; SQLite (or a server that won't list them) shows the schema directly. */}
        {active && p.connected && (list ? renderDatabases(c, list) : renderSchema(c.id, 1))}
      </div>
    );
  };

  return (
    <Panel
      title="Database Navigator"
      icon={Server}
      actions={
        <>
          <IconButton icon={Plus} label="New connection" size="sm" onClick={p.onAdd} />
          <IconButton
            icon={RefreshCw}
            label="Reload schema"
            size="sm"
            onClick={p.onRefresh}
            disabled={!p.activeId || p.loading}
          />
        </>
      }
    >
      <div className={styles.filter}>
        <Search size={13} className={styles.filterIcon} />
        <TextInput
          className={styles.filterInput}
          placeholder="Filter tables and columns"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>
      <div className={styles.tree} role="tree" aria-label="Database objects">
        {own.map(renderConnection)}
        {own.length === 0 && team.length === 0 && (
          <TreeRow depth={0} label={<span className={styles.message}>No connections yet</span>} />
        )}
        {/* Where team connections would be: a quiet way in for someone who lost their invite link. */}
        {team.length === 0 && p.onTeamSignIn && !teamHintHidden && (
          <div role="group" className={styles.scripts}>
            <TreeRow
              depth={0}
              icon={<Users size={14} />}
              label={<span className={styles.message}>Have a team invite? Sign in</span>}
              title="Sign in to your company's SayQL server to use the database connections your team shares."
              actions={
                <IconButton
                  icon={X}
                  label="Hide"
                  size="sm"
                  onClick={() => setTeamHintHidden(true)}
                />
              }
              onClick={p.onTeamSignIn}
            />
          </div>
        )}
        {team.length > 0 && (
          <div role="group" className={styles.scripts}>
            <TreeRow
              depth={0}
              expanded={!collapsed.has("team")}
              icon={<Users size={14} />}
              label={<span className={styles.connLabel}>Team</span>}
              meta={team.length}
              title="Shared by your team admin on your team's SayQL server. Queries on them run there."
              onClick={() => setCollapsed((s) => toggle(s, "team"))}
            />
            {!collapsed.has("team") && team.map(renderConnection)}
          </div>
        )}
        {p.scripts.length > 0 && (
          <div role="group" className={styles.scripts}>
            <TreeRow
              depth={0}
              expanded={!collapsed.has("scripts")}
              icon={<Folder size={14} />}
              label={<span className={styles.connLabel}>Saved scripts</span>}
              meta={
                <span className={styles.scriptFilter} role="radiogroup" aria-label="Which scripts">
                  {(["here", "all"] as const).map((v) => (
                    <button
                      key={v}
                      type="button"
                      role="radio"
                      aria-checked={scriptsShown === v}
                      className={cx(
                        styles.scriptFilterOption,
                        scriptsShown === v && styles.scriptFilterOn,
                      )}
                      title={
                        v === "here"
                          ? "Only scripts for the open database"
                          : "Scripts for every database"
                      }
                      onClick={(e) => {
                        e.stopPropagation();
                        setScriptsShown(v);
                      }}
                    >
                      {v === "here"
                        ? `This database${anyHere ? ` ${p.scripts.filter((s) => s.here).length}` : ""}`
                        : `All ${p.scripts.length}`}
                    </button>
                  ))}
                </span>
              }
              onClick={() => setCollapsed((s) => toggle(s, "scripts"))}
            />
            {!collapsed.has("scripts") && shownScripts.length === 0 && (
              <TreeRow
                depth={1}
                label={<span className={styles.message}>No scripts for this database</span>}
              />
            )}
            {!collapsed.has("scripts") &&
              shownScripts.map((s) => (
                <TreeRow
                  key={s.id}
                  depth={1}
                  icon={<FileCode2 size={14} />}
                  label={<span className={cx(s.orphaned && styles.message)}>{s.name}</span>}
                  meta={s.orphaned ? "connection removed" : s.location}
                  title={s.orphaned ? "Its connection was deleted" : `Open ${s.name}`}
                  onClick={() => !s.orphaned && p.onOpenScript(s.id)}
                  onContextMenu={(e) => menu.open(e, scriptMenu(s))}
                  actions={
                    <IconButton
                      icon={Trash2}
                      label={`Delete ${s.name}`}
                      size="sm"
                      onClick={() => p.onDeleteScript(s.id)}
                    />
                  }
                />
              ))}
          </div>
        )}
      </div>
      {menu.menu && <ContextMenu at={menu.menu.at} items={menu.menu.items} onClose={menu.close} />}
    </Panel>
  );
}
