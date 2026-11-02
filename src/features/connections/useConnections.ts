import { useCallback, useRef, useState } from "react";
import {
  commands,
  type ConnectionConfig,
  type DatabaseList,
  type Schema,
  type Target,
} from "../../api";
import { errorText } from "../../lib/errors";
import { targetKey } from "./target";

/** Loading state of one database's schema. */
export interface SchemaState {
  schema: Schema | null;
  loading: boolean;
  error: string | null;
}

const EMPTY: SchemaState = { schema: null, loading: false, error: null };

export interface ConnectionsState {
  connections: ConnectionConfig[];
  connection: (id: string) => ConnectionConfig | null;
  /** The server's databases per connection; `null` for SQLite, absent while loading. */
  databases: Record<string, DatabaseList | null>;
  schemaState: (t: Target | null) => SchemaState;
  /** Loads the target's schema unless it is loaded or loading (`refresh` always reloads). */
  ensure: (t: Target, refresh?: boolean) => void;
  /**
   * Whether the person connected to this connection in this session. Nothing connects on its
   * own when SayQL starts: restored tabs wait for a click on the connection, Connect, Run or Ask.
   */
  isConnected: (id: string) => boolean;
  /** Connects (an explicit action by the person): remembers it for this session and loads the target's schema. */
  connect: (t: Target) => void;
  /** Name of the target's database, for display (the server default when none was chosen). */
  databaseName: (t: Target | null) => string | null;
  /** Record a connection saved by the connection dialog; its schemas reload on next use. */
  upsert: (c: ConnectionConfig) => void;
  /** Record a connection deleted by the connection dialog. */
  remove: (id: string) => void;
  /** Replaces the SayQL Team connections (empty when signed out); saved connections stay. */
  setTeam: (list: ConnectionConfig[]) => void;
  dismissError: (t: Target) => void;
}

/** Saved connections, each server's database list, and the schemas of opened databases. */
export function useConnections(initial: ConnectionConfig[]): ConnectionsState {
  const [connections, setConnections] = useState(initial);
  const [databases, setDatabases] = useState<Record<string, DatabaseList | null>>({});
  const [schemas, setSchemas] = useState<Record<string, SchemaState>>({});
  // Connections the person opened this session; empty on start, so nothing auto-connects.
  const [connected, setConnected] = useState<ReadonlySet<string>>(() => new Set());
  const markConnected = (id: string, on: boolean) =>
    setConnected((s) => {
      if (s.has(id) === on) return s;
      const next = new Set(s);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  // What is loaded or loading, readable synchronously so `ensure` never double-loads.
  const requested = useRef(new Set<string>());
  const listed = useRef(new Set<string>());

  const patch = (key: string, s: Partial<SchemaState>) =>
    setSchemas((all) => ({ ...all, [key]: { ...(all[key] ?? EMPTY), ...s } }));

  const loadDatabaseList = useCallback((c: ConnectionConfig) => {
    if (c.kind === "sqlite" || listed.current.has(c.id)) return;
    listed.current.add(c.id);
    commands.listDatabases(c.id).then(
      (list) => setDatabases((all) => ({ ...all, [c.id]: list })),
      // Listing needs extra permissions on some servers; the open database still works.
      () => setDatabases((all) => ({ ...all, [c.id]: null })),
    );
  }, []);

  const ensure = useCallback(
    (t: Target, refresh = false) => {
      const key = targetKey(t);
      const c = connections.find((x) => x.id === t.connectionId);
      if (!c || (!refresh && requested.current.has(key))) return;
      requested.current.add(key);
      if (refresh) listed.current.delete(c.id);
      loadDatabaseList(c);
      patch(key, { loading: true, error: null });
      (refresh ? commands.refreshSchema(t) : commands.openDatabase(t)).then(
        (schema) => patch(key, { schema, loading: false }),
        (e) => {
          requested.current.delete(key); // allow a retry
          patch(key, { loading: false, error: `Could not open the database: ${errorText(e)}` });
        },
      );
    },
    [connections, loadDatabaseList],
  );

  /** Drops everything cached for a connection. */
  const forget = (id: string) => {
    const prefix = `${id}/`;
    requested.current.forEach((k) => k.startsWith(prefix) && requested.current.delete(k));
    listed.current.delete(id);
    setSchemas((all) =>
      Object.fromEntries(Object.entries(all).filter(([k]) => !k.startsWith(prefix))),
    );
    setDatabases((all) => {
      const { [id]: _, ...rest } = all;
      return rest;
    });
  };

  return {
    connections,
    connection: (id) => connections.find((c) => c.id === id) ?? null,
    databases,
    schemaState: (t) => (t ? (schemas[targetKey(t)] ?? EMPTY) : EMPTY),
    ensure,
    isConnected: (id) => connected.has(id),
    connect: (t) => {
      markConnected(t.connectionId, true);
      ensure(t);
    },
    databaseName: (t) => (t ? (t.database ?? databases[t.connectionId]?.current ?? null) : null),
    upsert: (c) => {
      setConnections((all) =>
        all.some((x) => x.id === c.id) ? all.map((x) => (x.id === c.id ? c : x)) : [...all, c],
      );
      forget(c.id);
    },
    remove: (id) => {
      setConnections((all) => all.filter((x) => x.id !== id));
      markConnected(id, false);
      forget(id);
    },
    dismissError: (t) => patch(targetKey(t), { error: null }),
    setTeam: (list) => {
      if (list.length === 0 && !connections.some((c) => c.team)) return;
      setConnections((all) => [...all.filter((c) => !c.team), ...list]);
      // Signing in again (or as someone else) reloads their schemas.
      connections.filter((c) => c.team).forEach((c) => forget(c.id));
    },
  };
}
