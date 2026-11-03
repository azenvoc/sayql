// The backend's types under the names the UI uses. Every shape here comes from
// `./bindings` (generated from the Rust types by `pnpm gen:types`); this file only renames them
// and adds the few frontend-only helpers at the end. Never write a backend shape by hand here:
// change the Rust type and regenerate (AGENTS.md, "Types shared with the backend").
//
// `X_Serialize` is the JSON the backend sends (results, events); `X_Deserialize` what a command
// accepts as an argument. Types that are the same both ways have one name.

import type {
  COMMAND_ARGS,
  commands,
  ConnConfig_Serialize,
  Schema_Serialize,
  TeamConnectionView_Serialize,
} from "./bindings";

export type {
  AccessView_Serialize as AgentAccess,
  AgentAccess_Deserialize as AgentAccessSettings,
  Activity_Serialize as McpActivity,
  AgentSettings_Serialize as AgentSettings,
  AgentView_Serialize as AuditAgent,
  AiStatus_Serialize as TeamAiStatus,
  AiUsage_Serialize as TeamAiUsage,
  Anchor_Serialize as NoteAnchor,
  Answer_Serialize as AgentAnswer,
  AppError,
  AppInfo,
  AppList_Serialize as McpApps,
  AppResults,
  AppSummary_Serialize as McpApp,
  AuditEvent_Serialize as AuditEvent,
  AuditModel,
  AuditPayment,
  AuditPaymentView,
  AuditRun_Serialize as AuditRun,
  Column_Serialize as Column,
  ConnectionDraft_Serialize as ConnectionDraft,
  DatabaseList_Serialize as DatabaseList,
  Delta_Serialize as AiDelta,
  Done_Serialize as AiDone,
  Event_Serialize as AgentEvent,
  ForeignKey,
  Handoff_Serialize as Handoff,
  InitView_Serialize as Bootstrap,
  Kind as DatabaseKind,
  KnowledgeSource_Serialize as KnowledgeSource,
  McpSetup,
  ModelOption,
  MyActivity_Serialize as MyActivity,
  NoteCheck_Serialize as NoteCheck,
  NoteSuggestion_Serialize as NoteSuggestion,
  Note_Serialize as DbNote,
  Note_Deserialize as DbNoteInput,
  PlanCheck_Serialize as PlanCheck,
  PreviousQuery,
  Progress as VoiceProgress,
  ProviderView as AiProvider,
  QueryDraft_Serialize as QueryDraft,
  QueryResult_Serialize as QueryResult,
  Reply_Deserialize as AgentReply,
  Results as AgentResults,
  Schema_Serialize as Schema,
  Script_Serialize as Script,
  Sentence as AgentSentence,
  ServerInfo_Serialize as TeamServerInfo,
  SettingsView_Serialize as Settings,
  Skill_Serialize as Skill,
  Source as QuerySource,
  SourceKind_Serialize as KnowledgeSourceKind,
  Step_Serialize as AgentStep,
  Table_Serialize as Table,
  Task_Deserialize as GenerationTask,
  TeamAccount_Serialize as TeamAccount,
  TeamInfo as TeamConnectionInfo,
  TeamRules,
  TeamStatus_Serialize as TeamStatus,
  UpdateInfo_Serialize as UpdateInfo,
  UpdateProgress_Serialize as UpdateProgress,
  Usage as AgentUsage,
  Value as Cell,
  VoiceStatus,
} from "./bindings";

export {
  COMMAND_ARGS,
  MAX_SOURCES_FREE,
  QUERY_CANCELLED,
  SOURCE_CAP_MESSAGE,
  TEAM_SIGNED_OUT,
  UPDATE_PROGRESS_EVENT,
  WRITE_CONFIRM,
} from "./bindings";

// --- Frontend-only helpers -------------------------------------------------------------------

/**
 * A connection as the UI lists it: a saved one, or a team connection (id `team:…`), which also
 * carries its team details (`team_connections`).
 */
export type ConnectionConfig = ConnConfig_Serialize &
  Partial<Pick<TeamConnectionView_Serialize, "team">>;

/** One database on a saved connection. `database: null` is the connection's default database. */
export interface Target {
  connectionId: string;
  database: string | null;
}

/** The SQL dialect of a schema (`Schema.dialect`). */
export type Dialect = Schema_Serialize["dialect"];

// --- Each command's arguments and result, by the name the webview invokes it with -------------
// For code that answers `invoke` itself (the browser preview's mock backend), so it is checked
// against the commands like the rest of the UI.

type Commands = typeof commands;
type Camel<S extends string> = S extends `${infer H}_${infer T}`
  ? `${H}${Camel<Capitalize<T>>}`
  : S;

/** A command's name as `invoke` takes it (`run_query`). */
export type CommandName = keyof typeof COMMAND_ARGS & string;

type Wrapper<C extends CommandName> = Commands[Camel<C> & keyof Commands];

/** The arguments object `invoke` sends for command `C` (`{ id, database, sql, … }`). */
export type CommandArgs<C extends CommandName> = {
  [
    I in keyof (typeof COMMAND_ARGS)[C] & `${number}` as (typeof COMMAND_ARGS)[C][I] & string
  ]: Parameters<Wrapper<C>>[I & keyof Parameters<Wrapper<C>>];
};

/** What command `C` resolves with. A command returning `()` sends `null`. */
export type CommandResult<C extends CommandName> =
  Awaited<ReturnType<Wrapper<C>>> extends infer R ? (R extends void ? null | void : R) : never;
