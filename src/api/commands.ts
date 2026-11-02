import { invoke } from "@tauri-apps/api/core";
import type {
  AgentAccess,
  AgentReply,
  AgentSettings,
  AuditAgent,
  AuditModel,
  AuditPayment,
  AuditPaymentView,
  AuditRun,
  Bootstrap,
  CodeSource,
  DbNote,
  ConnectionConfig,
  DatabaseList,
  GenerationTask,
  Handoff,
  McpActivity,
  McpApps,
  McpSetup,
  NoteSuggestion,
  QueryResult,
  QuerySource,
  TeamAccount,
  TeamAiStatus,
  TeamStatus,
  Script,
  Skill,
  Schema,
  ProviderChoice,
  Settings,
  UpdateInfo,
  AppInfo,
  Target,
  VoiceStatus,
  PlanCheck,
  MyActivity,
} from "./types";

/** Typed wrappers for the Tauri commands registered in src-tauri/src/lib.rs. */
export const commands = {
  init: () => invoke<Bootstrap>("init"),

  saveConnection: (config: ConnectionConfig, password: string | null) =>
    invoke<ConnectionConfig>("save_connection", { config, password }),
  deleteConnection: (id: string) => invoke<void>("delete_connection", { id }),
  testConnection: (config: ConnectionConfig, password: string | null) =>
    invoke<string>("test_connection", { config, password }),
  openDatabase: (t: Target) => invoke<Schema>("open_database", { id: t.connectionId, database: t.database }),
  refreshSchema: (t: Target) => invoke<Schema>("refresh_schema", { id: t.connectionId, database: t.database }),
  /** `null` for SQLite, where a file is a single database. */
  listDatabases: (connectionId: string) => invoke<DatabaseList | null>("list_databases", { id: connectionId }),

  /**
   * `runId` lets `cancelQuery` stop the query while it runs. On a team connection, `source` and
   * `question` go with the query to the team server's audit log.
   */
  runQuery: (t: Target, sql: string, allowWrite: boolean, runId?: string, source?: QuerySource, question?: string) =>
    invoke<QueryResult>("run_query", { id: t.connectionId, database: t.database, sql, allowWrite, runId, source, question }),
  /** Stops the query started with `runId` on the server; it then fails with QUERY_CANCELLED. */
  cancelQuery: (runId: string) => invoke<void>("cancel_query", { runId }),

  /**
   * Asks the AI for SQL; the answer streams back as `ai://` events (see `subscribeToAi`).
   * - `slot` scopes cancellation: a new request replaces the previous one in the same slot (editor tab).
   * - `focusTables`: qualified table names the person picked; drafts see only those and their neighbours.
   * - `skill`: the slug of a saved skill whose instruction goes with the request.
   */
  generateSql: (t: Target, question: string, reqId: number, slot: string, task: GenerationTask, focusTables: string[], skill: string | null) =>
    invoke<void>("generate_sql", { id: t.connectionId, database: t.database, question, reqId, slot, task, focusTables, skill }),
  cancelGeneration: (slot: string) => invoke<void>("cancel_generation", { slot }),

  /**
   * Starts an agent run in `slot` (the tab), stopping the one there; progress arrives as
   * `agent://event`s (see `subscribeToAgent`). `followUp` continues the tab's last run.
   */
  /** `auto`: Auto mode, which doesn't stop for the plan or before each send. */
  startAgent: (t: Target, question: string, runId: string, slot: string, focusTables: string[], skill: string | null, followUp: boolean, auto: boolean) =>
    invoke<void>("start_agent", { id: t.connectionId, database: t.database, question, runId, slot, focusTables, skill, followUp, auto }),
  /** Stops the model and the running query, on the server too. */
  stopAgent: (runId: string) => invoke<void>("stop_agent", { runId }),
  replyAgent: (runId: string, reply: AgentReply) => invoke<void>("reply_agent", { runId, reply }),
  agentAccess: (connectionId: string) => invoke<AgentAccess>("agent_access", { id: connectionId }),
  saveAgentAccess: (connectionId: string, access: AgentAccess) => invoke<AgentAccess>("save_agent_access", { id: connectionId, access }),
  listNotes: (t: Target) => invoke<DbNote[]>("list_notes", { id: t.connectionId, database: t.database }),
  /** The team's shared notes on a team connection whose server keeps them (read-only); null to use the notes saved here. */
  listSharedNotes: (connectionId: string) => invoke<DbNote[] | null>("list_shared_notes", { id: connectionId }),
  /** Suggests a note on a team connection for the team's knowledge owners to approve; resolves with what to tell the person. */
  suggestNote: (connectionId: string, text: string, check: string) =>
    invoke<string>("suggest_note", { id: connectionId, text, check: check.trim() || null }),
  /** Creates the note when `id` is empty; refuses notes that hold values from the data. */
  saveNote: (note: DbNote) => invoke<DbNote>("save_note", { note }),
  deleteNote: (id: string) => invoke<void>("delete_note", { id }),
  /** The note is still true: it's no longer doubtful. */
  confirmNote: (id: string) => invoke<DbNote>("confirm_note", { id }),
  /** Runs one note's check now; returns the note with the result. */
  checkNote: (id: string) => invoke<DbNote>("check_note", { id }),
  /** Reads the schema again, reviews the notes against it and runs every check. */
  checkNotes: (t: Target) => invoke<DbNote[]>("check_notes", { id: t.connectionId, database: t.database }),
  /** Notes AI apps suggested over MCP, on every connection, oldest first. */
  listNoteSuggestions: () => invoke<NoteSuggestion[]>("list_note_suggestions"),
  /**
   * Applies the suggestion and removes it: saves it as a note (the usual note checks apply),
   * replaces the note it corrects, or deletes the note it removes (then null).
   */
  acceptNoteSuggestion: (id: string) => invoke<DbNote | null>("accept_note_suggestion", { id }),
  dismissNoteSuggestion: (id: string) => invoke<void>("dismiss_note_suggestion", { id }),
  /** From the Ask box: switch to a ready provider and use `model` for typing (Quick) or agent mode. */
  chooseModel: (provider: string, model: string, purpose: "typing" | "agent") =>
    invoke<Settings>("choose_model", { provider, model, purpose }),
  /** The agent model (empty: the expert model) and the run limits. */
  saveAgentSettings: (agentModel: string, agent: AgentSettings) => invoke<Settings>("save_agent_settings", { agentModel, agent }),

  listScripts: () => invoke<Script[]>("list_scripts"),
  /** Creates the script when `id` is empty; returns it as stored. */
  saveScript: (script: Script) => invoke<Script>("save_script", { script }),
  deleteScript: (id: string) => invoke<void>("delete_script", { id }),

  listSkills: () => invoke<Skill[]>("list_skills"),
  /** Creates a skill, or replaces the one whose slug was `original`; returns it as stored. */
  saveSkill: (skill: Skill, original: string | null) => invoke<Skill>("save_skill", { skill, original }),
  deleteSkill: (slug: string) => invoke<void>("delete_skill", { slug }),

  /** Redeems a SayQL Team invite code; the token goes to the keychain. */
  teamSignIn: (server: string, code: string) => invoke<TeamAccount>("team_sign_in", { server, code }),
  /** "Sign in with your company": opens the team server's sign-in page in the browser. */
  teamSsoStart: (server: string) => invoke<void>("team_sso_start", { server }),
  /** The browser came back (`sayql://team-sso`): swaps its one-time code for a sign-in. */
  teamSsoFinish: (code: string) => invoke<TeamAccount>("team_sso_finish", { code }),
  /** The signed-in account and the server's licence; null when signed out (nothing is sent then). */
  teamStatus: () => invoke<TeamStatus | null>("team_status"),
  teamSignOut: () => invoke<void>("team_sign_out"),
  /** The team's shared connections; empty when signed out (nothing is sent then). */
  teamConnections: () => invoke<ConnectionConfig[]>("team_connections"),
  /** Team AI, asked again; null when signed out (nothing is sent then). */
  teamAi: () => invoke<TeamAiStatus | null>("team_ai"),

  mcpSetup: () => invoke<McpSetup>("mcp_setup"),
  /** Ids (from features/agent/agents.ts) of the coding agents that look installed; nothing is run to find them. */
  installedCodingAgents: () => invoke<string[]>("installed_coding_agents"),
  /** Opens Cursor's or VS Code's link that adds SayQL's MCP server (refuses any other link). */
  openInstallLink: (url: string) => invoke<void>("open_install_link", { url }),
  mcpActivity: (limit?: number) => invoke<McpActivity[]>("mcp_activity", { limit }),
  mcpApps: () => invoke<McpApps>("mcp_apps"),
  setMcpAppBlocked: (name: string, blocked: boolean) => invoke<McpApps>("set_mcp_app_blocked", { name, blocked }),
  approveMcpApp: (name: string) => invoke<McpApps>("approve_mcp_app", { name }),
  /** Turning it on approves the apps already seen that aren't blocked. */
  setMcpOnlyApproved: (on: boolean) => invoke<McpApps>("set_mcp_only_approved", { on }),
  clearMcpActivity: () => invoke<void>("clear_mcp_activity"),
  /** Your own queries, newest first; for one connection when given. */
  myActivity: (connectionId: string | null, limit?: number) => invoke<MyActivity[]>("my_activity", { connectionId, limit }),
  clearMyActivity: () => invoke<void>("clear_my_activity"),
  /** When an AI app last fetched the audit prompt, per connection and database. */
  mcpAudits: () => invoke<AuditRun[]>("mcp_audits"),
  /** The audit instructions for a connection, to paste into an AI app without MCP prompts. */
  auditPrompt: (connectionId: string, database: string | null) => invoke<string>("audit_prompt", { id: connectionId, database }),
  /** The coding agents SayQL knows, whether each is installed and can run an audit read-only. */
  auditAgents: () => invoke<AuditAgent[]>("audit_agents"),
  /** The code an audit of this connection reads: local folders and git repositories. */
  codeSources: (connectionId: string) => invoke<CodeSource[]>("code_sources", { id: connectionId }),
  /** Asks for a folder and adds it as a code source; null when cancelled. */
  addCodeFolder: (connectionId: string) => invoke<CodeSource | null>("add_code_folder", { id: connectionId }),
  /** Adds a git repository once git can read it. `token`: optional, read-only, kept in the keychain. */
  addCodeRepo: (connectionId: string, repo: { url: string; branch: string; subdir: string; token: string | null; tokenUser: string | null }) =>
    invoke<CodeSource>("add_code_repo", { id: connectionId, ...repo }),
  removeCodeSource: (connectionId: string, sourceId: string) => invoke<void>("remove_code_source", { id: connectionId, sourceId }),
  /** Checks the folder is there, or that git can reach the repository; returns its new status. */
  checkCodeSource: (connectionId: string, sourceId: string) => invoke<CodeSource>("check_code_source", { id: connectionId, sourceId }),
  /** Runs the audit with `agent` over the connection's code sources, read-only; progress comes as `audit://event`. */
  startAudit: (runId: string, agent: string, connectionId: string, database: string | null) =>
    invoke<void>("start_audit", { runId, agent, id: connectionId, database }),
  cancelAudit: (runId: string) => invoke<void>("cancel_audit", { runId }),
  /** The model audits run on (Sonnet until the person picks another). */
  auditModel: () => invoke<AuditModel>("audit_model"),
  setAuditModel: (model: AuditModel) => invoke<void>("set_audit_model", { model }),
  /** Who pays for audits: always the person's own Claude Code sign-in while paying with credits is off (unused by the UI). */
  auditPayment: () => invoke<AuditPaymentView>("audit_payment"),
  setAuditPayment: (payment: AuditPayment) => invoke<void>("set_audit_payment", { payment }),
  /** Opens the system terminal in the first code source running the same read-only audit interactively. */
  openAuditInTerminal: (agent: string, connectionId: string, database: string | null) =>
    invoke<void>("open_audit_in_terminal", { agent, id: connectionId, database }),
  /** Handoffs from AI apps waiting to be shown (see the `handoff://new` event). */
  takeHandoffs: () => invoke<Handoff[]>("take_handoffs"),

  /** Version, OS and chip, for pre-filling feedback. */
  appInfo: () => invoke<AppInfo>("app_info"),

  /** A newer release, or null (also when offline, and always in development builds). */
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  /** Downloads and installs the update, then restarts the app. */
  installUpdate: () => invoke<void>("install_update"),

  /** Saves the choices for one provider and makes it the one that writes SQL. */
  saveSettings: (choice: ProviderChoice) => invoke<Settings>("save_settings", { ...choice }),
  /** Marks the first-run setup as done so it isn't shown again. */
  finishOnboarding: () => invoke<Settings>("finish_onboarding"),
  /** Model ids an AI provider offers; also checks the key and base URL. */
  listModels: (provider: string, baseUrl: string, apiKey: string | null) =>
    invoke<string[]>("list_models", { provider, baseUrl, apiKey }),
  /** What's left on the saved SayQL AI credit key, in US dollars; null without one. */
  creditsBalance: () => invoke<number | null>("credits_balance"),
  /** "Your Claude plan": checks the installed Claude Code is recent enough and signed in. */
  checkClaudePlan: () => invoke<PlanCheck>("check_claude_plan"),

  /** Voice input: whether the on-device speech model is downloaded. */
  voiceStatus: () => invoke<VoiceStatus>("voice_status"),
  /** Downloads the speech model once; progress arrives as `voice://progress` events. */
  voiceDownload: () => invoke<void>("voice_download"),
  voiceCancelDownload: () => invoke<void>("voice_cancel_download"),
  voiceDeleteModel: () => invoke<void>("voice_delete_model"),
  /** Starts recording from the default microphone. */
  voiceStart: () => invoke<void>("voice_start"),
  /** Stops recording and returns the transcript. `prompt`: table and column names to help spelling. */
  voiceStop: (prompt: string) => invoke<string>("voice_stop", { prompt }),
  /** Stops recording and discards the audio. */
  voiceCancel: () => invoke<void>("voice_cancel"),
};
