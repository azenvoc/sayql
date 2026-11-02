// Shapes returned by the Rust backend (src-tauri/src). Keep in sync with the serde structs there.

export type DatabaseKind = "postgres" | "mysql" | "sqlite" | "mssql";

export interface ConnectionConfig {
  id: string;
  name: string;
  kind: DatabaseKind;
  host: string;
  port: number | null;
  database: string;
  user: string;
  /** SQLite database file path. */
  file: string;
  /** Require an encrypted connection. */
  ssl: boolean;
  /** SQL Server: accept a self-signed server certificate. */
  trustCert: boolean;
  /** Tagged as production: shows a PROD badge. Display only; absent (off) on connections saved before 0.4.8. */
  production?: boolean;
  /** Set on a SayQL Team connection (id `team:…`): read-only here, managed by the team admin. */
  team?: TeamConnectionInfo;
}

/** A team connection's details from the team server; never its host, user or password. */
export interface TeamConnectionInfo {
  /** The database's name on the server, for display. */
  database: string;
  notes: string;
}

/** Who is signed in to SayQL Team, and where; mirrors `team::TeamAccount`. */
export interface TeamAccount {
  server: string;
  email: string;
  name: string;
  role: string;
}

/** The team server and its licence, from `/me`; mirrors `team::ServerInfo`. */
export interface TeamServerInfo {
  name: string;
  version: string;
  licensed: boolean;
  plan: string | null;
  licensee: string | null;
  expires: string | null;
  users: number | null;
  userLimit: number | null;
}

export interface TeamStatus {
  account: TeamAccount;
  server: TeamServerInfo;
  /** Team AI and this month's usage; null if the server couldn't say. */
  ai: TeamAiStatus | null;
}

/** Tokens used this month against a limit (`null`: no limit). */
export interface TeamAiUsage {
  tokens: number;
  limit: number | null;
}

/**
 * Team AI: the company's AI key on the team server, used for AI requests on team connections;
 * mirrors `team::ai::AiStatus`.
 */
export interface TeamAiStatus {
  enabled: boolean;
  provider: "anthropic" | "openai" | null;
  models: string[];
  defaultModel: string | null;
  fastModel: string | null;
  /** e.g. "2026-11". */
  month: string | null;
  you: TeamAiUsage | null;
  team: TeamAiUsage | null;
}

/** Where a query came from, for the team server's audit log. */
export type QuerySource = "ask" | "agent" | "editor";

/** Must match `team::SIGNED_OUT` in src-tauri/src/team.rs. */
export const TEAM_SIGNED_OUT = "Signed out of your team, sign in again.";

export type Dialect = "PostgreSQL" | "MySQL" | "SQLite" | "SQL Server";

export interface Column {
  name: string;
  data_type: string;
  nullable: boolean;
  primary_key: boolean;
  enum_values: string[] | null;
  /** The database's own description of the column. */
  comment?: string | null;
}

export interface ForeignKey {
  column: string;
  ref_table: string;
  ref_column: string;
}

export interface Table {
  schema: string | null;
  name: string;
  kind: "table" | "view" | "materialized view";
  columns: Column[];
  foreign_keys: ForeignKey[];
  row_estimate: number | null;
  /** The database's own description of the table. */
  comment?: string | null;
  /** CHECK constraints as the database writes them. */
  checks?: string[];
}

export interface Schema {
  dialect: Dialect;
  tables: Table[];
}

export type Cell = string | number | boolean | null | object;

export interface QueryResult {
  columns: string[];
  rows: Cell[][];
  rowsAffected: number | null;
  truncated: boolean;
  elapsedMs: number;
  /** Columns a team server left out because its admin marked them never send. */
  withheld?: string[];
}

/** The person's own Claude Code, checked for "Your Claude plan"; mirrors `PlanCheck` in src-tauri/src/ai/subscription.rs. */
export interface PlanCheck {
  version: string;
  /** "claude.ai" when signed in with a Claude plan. */
  authMethod: string;
  /** "pro", "max", "team"… */
  plan: string | null;
}

export interface ModelOption {
  id: string;
  label: string;
}

/** An AI service that can write SQL; mirrors `ProviderView` in src-tauri/src/commands/settings.rs. */
export interface AiProvider {
  id: string;
  label: string;
  note: string;
  /** Claude's own API, the OpenAI-compatible API every other provider speaks, or the person's own Claude plan through their installed Claude Code. */
  api: "anthropic" | "openai" | "plan";
  key: "required" | "optional" | "none";
  keyPlaceholder: string;
  /** Runs on this computer, so nothing is sent over the internet. */
  local: boolean;
  /** Empty when the person must enter one. */
  defaultBaseUrl: string;
  /** Empty means the default. */
  baseUrl: string;
  typingModel: string;
  /** Empty means the typing model does fixes too. */
  expertModel: string;
  /** Empty means the expert model runs agent mode. */
  agentModel: string;
  hasKey: boolean;
  /** Set up (model, key, address), so the Ask box can switch to it. */
  ready: boolean;
}

export interface Settings {
  /** The provider that writes SQL. */
  provider: string;
  /** Writes SQL while the person types. */
  typingModel: string;
  /** Fixes rejected queries and improves queries on request. */
  expertModel: string;
  /** The provider has what it needs (key, model) to write SQL. */
  aiReady: boolean;
  /** The first-run setup was finished or dismissed. */
  onboarded: boolean;
  workspaceId: string;
  /** Claude's models. */
  models: ModelOption[];
  providers: AiProvider[];
  /** The model that runs agent mode, as chosen; empty means the expert model. */
  agentModel: string;
  agent: AgentSettings;
  /** The SayQL Team server signed in to; absent when signed out. */
  team?: TeamAccount | null;
}

/** Run limits for agent mode. */
export interface AgentSettings {
  maxSteps: number;
  maxQueries: number;
  querySeconds: number;
  runSeconds: number;
  maxTokens: number;
  /** Start right away instead of showing the plan first. */
  skipPlan: boolean;
}

/** What the AI may see of query results on a connection in agent mode. */
export type AgentResults = "off" | "counts" | "rows";

/** What AI apps using SayQL over MCP may see of query results on a connection. */
export type AppResults = "off" | "counts" | "masked" | "all";

/** Agent mode's access to one connection, which is off until the person turns it on, and what AI apps may see of it. */
export interface AgentAccess {
  enabled: boolean;
  results: AgentResults;
  /** What AI apps over MCP may see; counts until the person chooses. Never-send columns apply too. */
  apps: AppResults;
  askBeforeSend: boolean;
  /** Columns never sent (`column` or `table.column`), the team admin's included. */
  neverSend: string[];
  /** On a team connection: what the team admin set, which can't be loosened here. */
  team?: TeamRules | null;
}

/** The team admin's rules for agent mode on a team connection. */
export interface TeamRules {
  /** The most agent mode may see of results. */
  maxResults: AgentResults;
  neverSend: string[];
}

/** A table, or one of its columns, that a note is about; mirrors `schema::anchors::Anchor`. */
export interface NoteAnchor {
  table: string;
  column?: string;
}

/** A read-only query that answers whether a note still holds: one row, one column, true or false. */
export interface NoteCheck {
  sql: string;
  /** The last run; `ok` is null when it couldn't run (`error` says why). */
  last?: { ok: boolean | null; at: number; error?: string } | null;
}

/** A fact about a database that helps answer questions on it. Never holds row values. */
export interface DbNote {
  /** Empty for a new note. */
  id: string;
  connectionId: string;
  database: string | null;
  text: string;
  updatedAt: number;
  createdAt?: number;
  /** Who wrote it: empty for the person, else "Investigate", "SayQL" or the AI app's name. */
  source?: string;
  evidence?: string | null;
  /** What it's about, found in the schema when it was saved. */
  anchors?: NoteAnchor[];
  /** Set when something suggests it may no longer be true. */
  suspect?: { reason: string; since: number } | null;
  check?: NoteCheck | null;
  /** When its check last passed. */
  verifiedAt?: number | null;
  /** Its earlier texts, newest first. */
  history?: { text: string; at: number; source: string }[];
}

/** A note an AI app suggested over MCP, waiting for the person to accept or dismiss it. */
export interface NoteSuggestion {
  id: string;
  connectionId: string;
  database: string | null;
  /** The note as it would be saved. */
  text: string;
  /** Why the app thinks it's true. */
  evidence: string;
  /** The AI app's name, e.g. "Claude Code". */
  app: string;
  /** Milliseconds since the Unix epoch. */
  createdAt: number;
  /** The id of the saved note this one replaces (or removes, with `remove`). */
  replaces?: string | null;
  /** That note's text when the suggestion was made. */
  replacedText?: string | null;
  /** Accepting deletes the note in `replaces`; `text` says why. */
  remove?: boolean;
  /** A read-only query that checks the note, kept with it once accepted. */
  check?: string | null;
}

/** When an AI app last fetched the audit prompt for a database; mirrors `store::audits::AuditRun`. */
export interface AuditRun {
  connectionId: string;
  database: string | null;
  app: string;
  /** Milliseconds since the Unix epoch. */
  at: number;
}

/** A coding agent "Run audit" knows; mirrors `commands::audit::AgentView`. */
export interface AuditAgent {
  id: string;
  label: string;
  /** Where it's installed; null when it isn't found. */
  path: string | null;
  version: string | null;
  /** SayQL can keep it read-only (only these can run an audit). */
  supported: boolean;
  /** Found, supported and recent enough. */
  ready: boolean;
  /** Why it can't run, when found but not ready. */
  problem: string | null;
  installUrl: string;
  /** SayQL's built-in OpenCode, rather than an agent the person installed. */
  builtIn?: boolean;
  /** What its first run downloads, in bytes, while it isn't downloaded yet. */
  downloadBytes?: number | null;
  /** The AI it talks to: "SayQL AI credits · claude-sonnet-5-5". */
  detail?: string | null;
}

/** Where a knowledge source is; mirrors `store::code_sources::SourceKind`. */
export type KnowledgeSourceKind =
  | { type: "local"; path: string }
  | {
      type: "git";
      url: string;
      /** Empty: the repository's default branch. */
      branch: string;
      /** Only this folder of the repository is read. */
      subdir: string | null;
      /** A read-only token is saved in the keychain for it. */
      hasToken: boolean;
      tokenUser: string | null;
    }
  /** Files on this computer, read again on each refresh. */
  | { type: "files"; paths: string[] }
  /** A web page, or the pages in its folder on the same site when `crawl`. */
  | { type: "url"; url: string; crawl: boolean };

/** Sources one connection can have in the free app; mirrors `MAX_SOURCES_FREE`. */
export const MAX_SOURCES_FREE = 3;
/** Why a 4th source is refused; mirrors `CAP_MESSAGE` in src-tauri/src/store/code_sources.rs. */
export const SOURCE_CAP_MESSAGE = "SayQL for teams keeps unlimited sources in sync for everyone.";

/** What an audit reads: a folder, a git repository, files or web pages; mirrors `store::code_sources::KnowledgeSource`. */
export interface KnowledgeSource {
  id: string;
  /** The short name the audit cites files by (`backend:src/db.ts`, `docs:billing.txt`). */
  label: string;
  kind: KnowledgeSourceKind;
  /** When it was last fetched (a repository), found (a folder) or read (files, web pages), in ms since the Unix epoch. */
  lastFetch: number | null;
  /** The commit read last time, or for files and web pages the version of their text. */
  lastCommit: string | null;
  /** Why the last fetch or check failed. */
  error: string | null;
}

/** The model an audit runs on; mirrors `audit_runner::AuditModel`. "default" is the person's own Claude Code default. */
export type AuditModel = "sonnet" | "opus" | "default";

/** Who pays for an audit run; mirrors `audit_runner::AuditPayment`. "credits" (paying with SayQL AI credits) is off for now: the app always runs on "subscription". */
export type AuditPayment = "subscription" | "credits";

/** The saved payment choice, and whether a SayQL credit key is saved; mirrors `commands::audit::AuditPaymentView`. */
export interface AuditPaymentView {
  payment: AuditPayment;
  hasCreditKey: boolean;
}

/** Progress of an audit run, as `audit://event`; mirrors `commands::audit::AuditEvent`. */
export type AuditEvent =
  | { type: "line"; runId: string; kind: "step" | "note" | "warn"; text: string }
  /** Downloading SayQL's built-in OpenCode before the run. */
  | { type: "download"; runId: string; received: number; total: number }
  | {
      type: "done";
      runId: string;
      outcome: "ok" | "error" | "cancelled" | "timeout";
      /** Suggestions SayQL received for review. */
      suggested: number;
      summary: string | null;
      error: string | null;
    };

export interface AgentUsage {
  input: number;
  cached: number;
  output: number;
}

/** One agent step. `sql`, `columns` and `rows` are real values that stay on this computer; `sent` is what went to the AI. */
export interface AgentStep {
  id: number;
  tool: string;
  text: string;
  status: "running" | "done" | "error";
  sql: string | null;
  rowCount: number | null;
  truncated: boolean;
  elapsedMs: number | null;
  columns: string[];
  rows: Cell[][];
  sent: string | null;
  sentSummary: string | null;
  checks: string[];
  error: string | null;
}

export interface AgentSentence {
  text: string;
  steps: number[];
  unverified: string[];
}

export interface AgentAnswer {
  text: string;
  sentences: AgentSentence[];
  confidence: "high" | "medium" | "low";
  assumptions: string[];
  notes: string[];
  /** The notes (and fixes to saved ones) wait with the other suggestions instead of being offered here. */
  notesQueued?: boolean;
  /** The notes went to the team server, for its knowledge owners to approve. */
  notesShared?: boolean;
  proposedSql: string | null;
  /** The proposed statement could change data; otherwise it is a query to run yourself. */
  proposedChangesData: boolean;
  cannotAnswer: boolean;
  unverified: string[];
  model: string;
  usage: AgentUsage;
}

/** Events from src-tauri/src/ai/agent/run.rs, all tagged with `runId`. */
export type AgentEvent =
  | { type: "plan"; runId: string; steps: string[]; waiting: boolean }
  | { type: "step"; runId: string; step: AgentStep }
  | { type: "send"; runId: string; stepId: number; payload: string }
  | { type: "question"; runId: string; stepId: number; question: string; choices: string[] }
  | {
      type: "progress";
      runId: string;
      turns: number;
      maxTurns: number;
      queries: number;
      maxQueries: number;
      usage: AgentUsage;
      elapsedMs: number;
    }
  | {
      type: "done";
      runId: string;
      answer: AgentAnswer | null;
      error: string | null;
      stopped: boolean;
    };

/** The person's reply to a paused run. */
export type AgentReply =
  | { type: "go"; plan: string[] | null }
  | { type: "send" }
  | { type: "skip" }
  | { type: "answer"; text: string };

/** What Settings saves for one provider. `apiKey: null` keeps the saved key. */
export interface ProviderChoice {
  provider: string;
  typingModel: string;
  expertModel: string;
  baseUrl: string;
  apiKey: string | null;
  workspaceId: string;
}

export interface DatabaseList {
  /** Databases this login can open, sorted by name. */
  databases: string[];
  /** The database the connection opens by default. */
  current: string | null;
}

/** One database on a saved connection. `database: null` is the connection's default database. */
export interface Target {
  connectionId: string;
  database: string | null;
}

/** The query last run in a tab; a follow-up request ("remove the middle name") edits it. */
export interface PreviousQuery {
  question: string;
  sql: string;
}

/** What the AI is asked to do; mirrors `ai::Task` in src-tauri/src/ai/prompt.rs. */
export type GenerationTask =
  /** `forceFollowUp` (`/followup`): treat the request as a change to `previous`. */
  | { kind: "draft"; previous: PreviousQuery | null; forceFollowUp?: boolean }
  /** An empty `error` means the person says the query is wrong. */
  | { kind: "fix"; sql: string; error: string }
  | { kind: "improve"; sql: string }
  /** The reply is plain English, not SQL. */
  | { kind: "explain"; sql: string };

/** This copy of SayQL, for pre-filling feedback; mirrors `AppInfo` in src-tauri/src/commands/app.rs. */
export interface AppInfo {
  version: string;
  /** `macos`, `windows` or `linux`. */
  os: string;
  /** `aarch64` or `x86_64`. */
  arch: string;
}

/** A newer SayQL release on the update feed. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
}

/** Where SayQL's MCP server binary is, for connecting Claude. */
export interface McpSetup {
  path: string;
  exists: boolean;
  /** Running from an AppImage, whose path changes on every launch. */
  appimage: boolean;
  /** Which config paths and shell quoting the setup shows. */
  os: "macos" | "windows" | "linux";
}

/** Something an AI app did, or tried, over MCP; mirrors `store::apps::Activity`. Holds SQL and counts, never rows. */
/** A query you ran; mirrors `MyActivity` in src-tauri/src/store/my_activity.rs. */
export interface MyActivity {
  /** Milliseconds since the Unix epoch. */
  at: number;
  connectionId: string;
  database?: string | null;
  source: "editor" | "ask" | "investigate";
  sql: string;
  status: "ok" | "error" | "cancelled";
  rows?: number | null;
  ms?: number | null;
  /** The error, or for Investigate what went to the AI. */
  detail?: string | null;
  question?: string | null;
}

export interface McpActivity {
  /** Milliseconds since the Unix epoch. */
  at: number;
  app: string;
  tool: string;
  connectionId: string;
  connection: string;
  sql: string;
  status: "ok" | "refused" | "error";
  detail?: string | null;
  rows?: number | null;
  maySee?: AppResults | null;
}

/** An AI app in the activity log, or on the blocked or approved list. */
export interface McpApp {
  name: string;
  blocked: boolean;
  /** On the approved list; what counts while only approved apps are let in. */
  approved: boolean;
  queries: number;
  lastUsed?: number | null;
}

/** The AI apps, and whether only the ones the user approved are let in. */
export interface McpApps {
  onlyApproved: boolean;
  apps: McpApp[];
}

/** A query saved from a tab; mirrors `store::Script` in src-tauri/src/store/mod.rs. */
export interface Script {
  id: string;
  name: string;
  connectionId: string;
  database: string | null;
  question: string;
  sql: string;
  /** Milliseconds since the Unix epoch. */
  updatedAt: number;
}

/** Saved instructions added to a question with `/slug`; mirrors `store::Skill` in src-tauri/src/store/mod.rs. */
export interface Skill {
  slug: string;
  description: string;
  instruction: string;
}

export interface Bootstrap {
  connections: ConnectionConfig[];
  settings: Settings;
}

/** Sentinel error from `run_query`: the statement writes data and needs confirmation. */
export const WRITE_CONFIRM = "WRITE_CONFIRM";

/** The error of a query stopped with `cancel_query`. Must match `CANCELLED` in db/execute.rs. */
export const QUERY_CANCELLED = "Query cancelled.";

export interface AiDelta {
  reqId: number;
  text: string;
  /** Present on the first chunk only. */
  firstTokenMs: number | null;
}

export interface AiDone {
  reqId: number;
  error: string | null;
  totalMs: number;
  cacheReadTokens: number | null;
}

/** New connection fields from an AI app, minus the password; mirrors `handoff::ConnectionDraft`. */
export interface ConnectionDraft {
  name: string;
  kind: DatabaseKind;
  host: string;
  port: number | null;
  database: string;
  user: string;
  file: string;
  /** null keeps the form's default for the kind. */
  ssl: boolean | null;
  trustCert: boolean | null;
}

/** SQL from an AI app, to open unrun in a new tab; mirrors `handoff::QueryDraft`. */
export interface QueryDraft {
  connectionId: string;
  database: string | null;
  sql: string;
  title: string | null;
  question: string | null;
}

/** Work handed over from an AI app through `sayql-mcp`; mirrors `handoff::Handoff`. */
export type Handoff = { source: string | null } & (
  | ({ type: "newConnection" } & ConnectionDraft)
  | ({ type: "openQuery" } & QueryDraft)
  | { type: "openScript"; scriptId: string }
  /** A SayQL Team invite link: the sign-in dialog, filled in, for the person to confirm. */
  | { type: "teamJoin"; server: string; code: string }
  /** The end of "Sign in with your company", only while SayQL waits for one it started. */
  | { type: "teamSso"; server: string; code: string }
);

/** Voice input's on-device speech model. */
export interface VoiceStatus {
  ready: boolean;
  downloading: boolean;
  modelBytes: number;
}

export interface VoiceProgress {
  received: number;
  total: number;
}
