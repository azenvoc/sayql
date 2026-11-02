// Development only: lets the UI run in a plain browser (`pnpm dev`) with fake data,
// for design work without the Rust backend. Never loaded inside the Tauri app.
import { emit } from "@tauri-apps/api/event";
import { MAX_SOURCES_FREE, SOURCE_CAP_MESSAGE } from "../api/types";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { MyActivity, AgentAccess, AgentReply, AgentStep, AiProvider, AuditAgent, AuditRun, Bootstrap, KnowledgeSource, DbNote, NoteSuggestion, QueryResult, Schema, Settings, Skill, Table } from "../api";
import { EXAMPLE_SKILLS, skillError } from "../features/skills";
import { version } from "../../package.json";
import { isReadOnly } from "./readOnly";
import { setStorageNamespace, storage } from "../lib/storage";

/** `?video`: answers and a saved skill for the explainer video (scripts/video), with slower streaming. */
const VIDEO = new URLSearchParams(window.location.search).has("video");
/**
 * `?firstrun`: a real first run. Only the demo connection, AI not set up, the setup not done,
 * no notes, suggestions, audits or AI-app activity, and remembered tabs and preferences kept
 * apart from the normal preview's and emptied on every load (see `setStorageNamespace`).
 */
const FIRSTRUN = new URLSearchParams(window.location.search).has("firstrun");
/** ?noplan: the Claude plan check says Claude Code isn't signed in. */
const NOPLAN = new URLSearchParams(window.location.search).has("noplan");

/** Your activity in the preview: a few queries from each place. */
const MIN = 60_000;
const myActivity: MyActivity[] = [
  { at: Date.now() - 3 * MIN, connectionId: "demo", source: "ask", sql: "SELECT name, SUM(total) AS spend FROM customers JOIN orders USING (customer_id) GROUP BY name ORDER BY spend DESC LIMIT 10;", status: "ok", rows: 10, ms: 14, question: "Top 10 customers by total spend" },
  { at: Date.now() - 9 * MIN, connectionId: "demo", source: "investigate", sql: "SELECT status, COUNT(*) FROM orders WHERE created_at >= date('now', '-14 days') GROUP BY status;", status: "ok", rows: 4, ms: 8, detail: "sent row count and column summaries", question: "why did paid orders drop last week?" },
  { at: Date.now() - 26 * MIN, connectionId: "pg", source: "editor", sql: "SELECT * FROM signups WHERE signup_date > now() - interval '7 days';", status: "error", detail: "column \"signup_date\" does not exist" },
  { at: Date.now() - 2 * 24 * 60 * MIN, connectionId: "demo", source: "editor", sql: "SELECT COUNT(*) FROM products;", status: "ok", rows: 1, ms: 3 },
];

const col = (name: string, data_type: string, primary_key = false) => ({
  name,
  data_type,
  nullable: !primary_key,
  primary_key,
  enum_values: null,
});

const table = (name: string, rows: number, columns: Table["columns"], fks: Table["foreign_keys"] = []): Table => ({
  schema: null,
  name,
  kind: "table",
  columns,
  foreign_keys: fks,
  row_estimate: rows,
});

const schema: Schema = {
  dialect: "SQLite",
  tables: [
    table("categories", 8, [col("id", "integer", true), col("name", "text")]),
    table("customers", 500, [col("id", "integer", true), col("first_name", "text"), col("last_name", "text"), col("email", "text"), col("country", "text"), col("signed_up_at", "text")]),
    table("order_items", 7500, [col("id", "integer", true), col("order_id", "integer"), col("product_id", "integer"), col("quantity", "integer"), col("unit_price", "real")], [
      { column: "order_id", ref_table: "orders", ref_column: "id" },
      { column: "product_id", ref_table: "products", ref_column: "id" },
    ]),
    table("orders", 3000, [col("id", "integer", true), col("customer_id", "integer"), col("status", "text"), col("ordered_at", "text"), col("shipped_at", "text")], [
      { column: "customer_id", ref_table: "customers", ref_column: "id" },
    ]),
    table("products", 120, [col("id", "integer", true), col("name", "text"), col("category_id", "integer"), col("price", "real"), col("stock", "integer")], [
      { column: "category_id", ref_table: "categories", ref_column: "id" },
    ]),
    { ...table("top_customers", -1, [col("customer_id", "integer"), col("total", "real")]), kind: "view" },
  ],
};

const provider = (id: string, label: string, extra: Partial<AiProvider> = {}): AiProvider => ({
  id,
  label,
  note: "",
  api: "openai",
  key: "required",
  keyPlaceholder: "",
  local: false,
  defaultBaseUrl: `https://api.${id}.example/v1`,
  baseUrl: "",
  typingModel: "",
  expertModel: "",
  agentModel: "",
  hasKey: false,
  ready: false,
  ...extra,
});

const settings: Settings = {
  provider: "anthropic",
  typingModel: "claude-haiku-5-5",
  expertModel: "claude-opus-5-5",
  aiReady: true,
  onboarded: false,
  workspaceId: "",
  agentModel: "",
  agent: { maxSteps: 12, maxQueries: 10, querySeconds: 30, runSeconds: 180, maxTokens: 400000, skipPlan: false },
  models: [
    { id: "claude-opus-5-5", label: "Claude Opus 5.5 — most accurate" },
    { id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5 — balanced" },
    { id: "claude-haiku-5-5", label: "Claude Haiku 5.5 — fastest" },
  ],
  providers: [
    provider("anthropic", "Anthropic Claude", {
      api: "anthropic",
      note: "Tuned for SayQL, with prompt caching for fast drafts.",
      keyPlaceholder: "sk-ant-…",
      defaultBaseUrl: "https://api.anthropic.com",
      typingModel: "claude-haiku-5-5",
      expertModel: "claude-opus-5-5",
      hasKey: true,
      ready: true,
    }),
    provider("sayql", "SayQL AI credits", {
      api: "anthropic",
      note: "Claude without an Anthropic account: prepaid credit from SayQL. Requests go through SayQL to Anthropic.",
      keyPlaceholder: "sqlc_…",
      defaultBaseUrl: "https://sayql.dev/api/ai",
      typingModel: "claude-haiku-5-5",
      expertModel: "claude-opus-5-5",
    }),
    provider("claude-plan", "Your Claude plan", {
      api: "plan",
      note: "Uses your installed Claude Code, signed in with your own Pro or Max plan. No API key; requests count against your plan's limits.",
      key: "none",
      defaultBaseUrl: "",
      typingModel: "claude-haiku-5-5",
      expertModel: "claude-opus-5-5",
      ready: true,
    }),
    provider("openai", "OpenAI", { note: "GPT models with an OpenAI API key.", keyPlaceholder: "sk-…" }),
    provider("gemini", "Google Gemini", { note: "Gemini models with a Google AI Studio key.", keyPlaceholder: "AIza…" }),
    provider("ollama", "Ollama", {
      note: "Runs on this computer. Nothing leaves your machine.",
      key: "none",
      local: true,
      defaultBaseUrl: "http://localhost:11434/v1",
      typingModel: "qwen3",
      expertModel: "qwen3",
      ready: true,
    }),
    provider("custom", "Other (OpenAI-compatible)", { note: "Any server with an OpenAI-compatible API.", key: "optional", defaultBaseUrl: "" }),
  ],
};

const boot: Bootstrap = {
  connections: [
    { id: "demo", name: "Demo store (SQLite)", kind: "sqlite", host: "", port: null, database: "", user: "", file: "/demo.sqlite", ssl: false, trustCert: false },
    { id: "pg", name: "analytics@prod-replica", kind: "postgres", host: "db.internal", port: 5432, database: "analytics", user: "readonly", file: "", ssl: true, trustCert: false, production: true },
    { id: "my", name: "shop@localhost", kind: "mysql", host: "localhost", port: 3306, database: "shop", user: "root", file: "", ssl: false, trustCert: false },
    { id: "ms", name: "sales@azure", kind: "mssql", host: "contoso.database.windows.net", port: 1433, database: "sales", user: "reader", file: "", ssl: true, trustCert: false },
  ],
  settings,
};

if (FIRSTRUN) {
  boot.connections = boot.connections.filter((c) => c.id === "demo");
  settings.aiReady = false;
  settings.onboarded = false;
  settings.providers = settings.providers.map((p) => (p.id === "anthropic" ? { ...p, hasKey: false, ready: false } : p));
}

const names = ["Ava Smith", "Liam Patel", "Noah Garcia", "Emma Kim", "Olivia Nguyen", "Arjun Müller", "Priya Rossi", "Mateo Silva"];
const result: QueryResult = {
  columns: ["customer_id", "customer", "country", "orders", "total_spend", "last_order_at", "notes"],
  rows: Array.from({ length: 240 }, (_, i) => [
    i + 1,
    names[i % names.length],
    ["USA", "India", "UK", "Germany", "Japan"][i % 5],
    20 - (i % 17),
    Math.round((9800 - i * 31.7) * 100) / 100,
    `2026-10-${String(1 + (i % 20)).padStart(2, "0")} 14:${String(i % 60).padStart(2, "0")}:00`,
    i % 4 === 0 ? null : "repeat buyer",
  ]),
  rowsAffected: null,
  truncated: false,
  elapsedMs: 12,
};

/** What SAMPLE_SQL returns: the same columns and row count the query asks for. */
const topCustomers: QueryResult = {
  columns: ["customer_id", "customer", "country", "orders", "total_spend"],
  rows: [
    [214, "Ava Smith", "USA", 31, 18420.5],
    [87, "Liam Patel", "India", 27, 16935.0],
    [342, "Noah Garcia", "Spain", 26, 15210.75],
    [19, "Emma Kim", "South Korea", 24, 14388.2],
    [156, "Olivia Nguyen", "Australia", 22, 13702.4],
    [401, "Arjun Müller", "Germany", 21, 12950.0],
    [63, "Priya Rossi", "Italy", 20, 12118.65],
    [278, "Mateo Silva", "Brazil", 19, 11640.3],
    [125, "Sofia Haddad", "Canada", 18, 10987.1],
    [330, "Kenji Watanabe", "Japan", 17, 10402.85],
  ],
  rowsAffected: null,
  truncated: false,
  elapsedMs: 9,
};

/** What "… FROM orders" returns, so a script's result tabs differ. */
const orders: QueryResult = {
  columns: ["id", "customer_id", "status", "ordered_at"],
  rows: Array.from({ length: 60 }, (_, i) => [
    3000 - i,
    1 + ((i * 37) % 500),
    ["shipped", "pending", "cancelled", "refunded"][i % 4],
    `2026-10-${String(20 - (i % 20)).padStart(2, "0")} 09:${String(i % 60).padStart(2, "0")}:00`,
  ]),
  rowsAffected: null,
  truncated: false,
  elapsedMs: 7,
};

const europe: QueryResult = {
  ...topCustomers,
  rows: [
    [342, "Noah Garcia", "Spain", 26, 15210.75],
    [401, "Arjun Müller", "Germany", 21, 12950.0],
    [63, "Priya Rossi", "Italy", 20, 12118.65],
    [77, "Oliver Brown", "UK", 18, 10870.4],
    [209, "Chloe Dubois", "France", 17, 10233.9],
    [455, "Lars Johansson", "Sweden", 16, 9874.15],
    [12, "Hannah Schmidt", "Germany", 16, 9520.0],
    [388, "Isabella Conti", "Italy", 15, 9105.6],
    [146, "Jakub Nowak", "Poland", 14, 8790.25],
    [261, "Lucas Bernard", "France", 14, 8512.8],
  ],
};

/** New customers, without personal data: what /pii-safe asks for. */
const newCustomers: QueryResult = {
  columns: ["id", "first_name", "last_name", "country", "signed_up_at"],
  rows: [
    [500, "Mia", "Tanaka", "Japan", "2026-10-21"],
    [499, "Leo", "Fischer", "Germany", "2026-10-20"],
    [498, "Zara", "Ahmed", "UK", "2026-10-18"],
    [497, "Diego", "Ramos", "Mexico", "2026-10-17"],
    [496, "Nora", "Lindqvist", "Sweden", "2026-10-15"],
    [495, "Ethan", "Clarke", "USA", "2026-10-12"],
    [494, "Aisha", "Okafor", "Nigeria", "2026-10-09"],
    [493, "Ravi", "Iyer", "India", "2026-10-06"],
    [492, "Camille", "Moreau", "France", "2026-10-03"],
  ],
  rowsAffected: null,
  truncated: false,
  elapsedMs: 6,
};

const japan: QueryResult = {
  columns: ["first_name", "last_name", "signed_up_at"],
  rows: [
    ["Mia", "Tanaka", "2026-10-21"],
    ["Kenji", "Watanabe", "2026-09-28"],
    ["Yui", "Sato", "2026-09-14"],
    ["Haruto", "Kobayashi", "2026-08-30"],
    ["Aoi", "Yamamoto", "2026-08-02"],
  ],
  rowsAffected: null,
  truncated: false,
  elapsedMs: 5,
};

const SAMPLE_SQL = `SELECT c.id AS customer_id,
       c.first_name || ' ' || c.last_name AS customer,
       c.country,
       COUNT(DISTINCT o.id) AS orders,
       ROUND(SUM(oi.quantity * oi.unit_price), 2) AS total_spend
FROM customers c
JOIN orders o ON o.customer_id = c.id
JOIN order_items oi ON oi.order_id = o.id
WHERE o.status NOT IN ('cancelled', 'refunded')
GROUP BY c.id
ORDER BY total_spend DESC
LIMIT 10;`;

const EUROPE_SQL = SAMPLE_SQL.replace(
  "WHERE o.status NOT IN ('cancelled', 'refunded')",
  "WHERE o.status NOT IN ('cancelled', 'refunded')\n  AND c.country IN ('UK', 'France', 'Germany', 'Spain', 'Italy', 'Sweden', 'Poland')",
);

const NEW_CUSTOMERS_SQL = `SELECT id, first_name, last_name, country, signed_up_at
FROM customers
WHERE signed_up_at >= date('now', 'start of month')
ORDER BY signed_up_at DESC;`;

const JAPAN_BROKEN_SQL = `SELECT first_nme, last_name, signed_up_at
FROM customers
WHERE country = 'Japan'
ORDER BY signed_up_at DESC;`;

const MARK_SHIPPED_SQL = "UPDATE orders\nSET status = 'shipped'\nWHERE id = 1042;";

// Questions containing "broken" get a query the mock database rejects, to exercise Fix.
const BROKEN_SQL = "SELECT first_nme, last_name\nFROM customers\nLIMIT 10;";
const FIXED_SQL = "SELECT first_name, last_name\nFROM customers\nLIMIT 10;";

const EXPLANATION =
  "Lists the 10 customers who spent the most. It adds up the price of every item in their orders, " +
  "leaves out cancelled and refunded orders, and shows each customer's name, country, order count and total spend.";

/**
 * Voice input: a fake download and a fake microphone. `?voice=ready` starts with the model
 * downloaded, `?voice=nomic` and `?voice=denied` show those errors.
 */
const VOICE = new URLSearchParams(window.location.search).get("voice");
const VOICE_BYTES = 81_781_811;
const voice = { ready: VOICE === "ready" || VOICE === "nomic" || VOICE === "denied", downloading: false, cancel: false, meter: 0 };

async function fakeDownload() {
  voice.downloading = true;
  voice.cancel = false;
  for (let received = 0; received < VOICE_BYTES; received += 4_000_000) {
    await new Promise((r) => setTimeout(r, 120));
    if (voice.cancel) {
      voice.downloading = false;
      throw "Download canceled.";
    }
    await emit("voice://progress", { received, total: VOICE_BYTES });
  }
  await emit("voice://progress", { received: VOICE_BYTES, total: VOICE_BYTES });
  voice.downloading = false;
  voice.ready = true;
}

function fakeMic() {
  let t = 0;
  voice.meter = globalThis.setInterval(() => {
    t += 1;
    void emit("voice://level", 0.03 + 0.12 * Math.abs(Math.sin(t / 3)) * Math.random());
  }, 50);
  if (VOICE === "denied") setTimeout(() => void emit("voice://blocked", "SayQL can't use the microphone. Allow it in System Settings → Privacy & Security → Microphone."), 1000);
}

/** Skills saved in this preview session; lost on reload. */
let skills: Skill[] = VIDEO ? EXAMPLE_SKILLS.slice(0, 1) : [];

function mockAnswer(question: string, task: { kind: string; previous?: unknown; sql?: string }, skill: string | null): string {
  if (task.kind === "explain") return EXPLANATION;
  if (task.kind === "fix") return task.sql?.includes("first_nme") ? task.sql.replace(/first_nme/g, "first_name") : FIXED_SQL;
  if (VIDEO) {
    if (skill === "pii-safe") return NEW_CUSTOMERS_SQL;
    if (task.previous && /europe/i.test(question)) return EUROPE_SQL;
    if (/japan/i.test(question)) return JAPAN_BROKEN_SQL;
    if (/shipped/i.test(question)) return MARK_SHIPPED_SQL;
  }
  if (task.kind === "improve") return `-- reviewed: added ordering\n${SAMPLE_SQL}`;
  const instruction = skills.find((s) => s.slug === skill)?.instruction;
  if (instruction) return `-- following /${skill}: ${instruction}\n${SAMPLE_SQL}`;
  return /broken/i.test(question) ? BROKEN_SQL : SAMPLE_SQL;
}

async function streamSql(reqId: number, sql: string) {
  const started = performance.now();
  // The video streams token-sized pieces at a readable pace, after a short wait for the first one.
  const chunks = (VIDEO ? sql.match(/\s*[^\s]{1,6}/g) : sql.match(/[\s\S]{1,18}/g)) ?? [];
  if (VIDEO) await new Promise((r) => setTimeout(r, 380));
  for (const [i, text] of chunks.entries()) {
    await new Promise((r) => setTimeout(r, VIDEO ? 12 + Math.random() * 16 : 25));
    await emit("ai://delta", { reqId, text, firstTokenMs: i === 0 ? Math.round(performance.now() - started) : null });
  }
  await emit("ai://done", { reqId, error: null, totalMs: Math.round(performance.now() - started), cacheReadTokens: 1800 });
}

/** Agent mode in the preview: access per connection, notes, and replies runs wait for. */
// The video turns agent mode on for the demo store, counts only, so the scene goes straight to the plan.
const agentAccess = new Map<string, AgentAccess>(
  VIDEO ? [["demo", { enabled: true, results: "counts", apps: "counts", askBeforeSend: false, neverSend: [] }]] : [],
);
// Two saved notes on the demo store, which suggestions below correct and remove (outside the video).
let notes: DbNote[] = VIDEO || FIRSTRUN
  ? []
  : [
      { id: "1f3a9c2e-5b7d-4e8f-9a01-2c3d4e5f6a7b", connectionId: "demo", database: null, updatedAt: Date.now() - 90 * 86_400_000,
        text: "orders.status 'pending' means awaiting payment",
        anchors: [{ table: "orders" }, { table: "orders", column: "status" }],
        check: { sql: "SELECT count(*) > 0 FROM orders WHERE status = 'pending'", last: { ok: true, at: Date.now() - 2 * 86_400_000 } },
        verifiedAt: Date.now() - 2 * 86_400_000 },
      { id: "7c2e1b4d-8a9f-4c3e-b2d1-0e9f8a7b6c5d", connectionId: "demo", database: null, updatedAt: Date.now() - 120 * 86_400_000,
        text: "Use legacy_orders for orders before 2024", source: "Claude Code",
        evidence: "api/reports.ts reads legacy_orders for dates before 2024-01-01",
        anchors: [{ table: "legacy_orders" }],
        suspect: { reason: "the table legacy_orders no longer exists", since: Date.now() - 86_400_000 } },
    ];
// When AI apps last fetched the audit prompt (none in the video).
const audits: AuditRun[] = VIDEO || FIRSTRUN ? [] : [{ connectionId: "pg", database: "analytics", app: "Claude Code", at: Date.now() - 3 * 86_400_000 }];
// Notes AI apps suggested over MCP, waiting in the navigator and Settings → AI apps.
let suggestions: NoteSuggestion[] = FIRSTRUN
  ? []
  : VIDEO
  ? [
      { id: "sug-1", connectionId: "demo", database: null, app: "Claude Code", createdAt: Date.now() - 20_000,
        text: "Refunds are orders.status = 'refunded'; there is no refunds table",
        evidence: "Checked api/orders/refund.ts (commit 4f2c1a9): it sets orders.status; describe_schema lists no refunds table." },
    ]
  : [
      { id: "sug-1", connectionId: "demo", database: null, app: "Claude Code", createdAt: Date.now() - 120_000,
        text: "reviews.rating is 1 to 5; 0 means the review was removed",
        evidence: "api/reviews/moderate.ts sets rating = 0 on removal (commit 9b1e2d4, 2026-09-14); a CHECK in the migration allows 0 to 5." },
      { id: "sug-2", connectionId: "demo", database: null, app: "Cursor", createdAt: Date.now() - 900_000,
        text: "categories.parent_id is unused since the flat category redesign; ignore it",
        evidence: "No reads of parent_id in web/ or api/ since commit 51c0e7a (2026-06-02)." },
      { id: "sug-3", connectionId: "demo", database: null, app: "Claude Code", createdAt: Date.now() - 60_000,
        text: "orders.status 'pending' means paid, waiting to ship; awaiting payment is 'unpaid'",
        evidence: "OrderStatus in api/orders/status.ts (commit 2d8e4a1, 2026-08-30) renamed the states.",
        replaces: "1f3a9c2e-5b7d-4e8f-9a01-2c3d4e5f6a7b", replacedText: "orders.status 'pending' means awaiting payment", remove: false },
      { id: "sug-4", connectionId: "demo", database: null, app: "Claude Code", createdAt: Date.now() - 50_000,
        text: "legacy_orders was dropped from the code; orders holds every order since the 2025 backfill",
        evidence: "No references to legacy_orders in the repo since commit 8b0c3f5 (2025-11-02); the schema has no legacy_orders table.",
        replaces: "7c2e1b4d-8a9f-4c3e-b2d1-0e9f8a7b6c5d", replacedText: "Use legacy_orders for orders before 2024", remove: true },
    ];
const waiting = new Map<string, (r: AgentReply) => void>();
const stopped = new Set<string>();

/** A scripted agent run on the demo store: plan, two steps, an optional Send/Skip pause, a cited answer. */
// What AI apps did over MCP, for Settings → AI apps.
const mcpBlocked = new Set<string>();
const mcpApproved = new Set<string>();
let mcpOnlyApproved = false;
// The video's log matches its terminal scene: Claude Code gets a count, then is refused a private column.
const mcpActivity = FIRSTRUN
  ? []
  : VIDEO
  ? [
      { at: Date.now() - 30_000, app: "Claude Code", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "SELECT c.email FROM customers c JOIN orders o ON o.customer_id = c.id WHERE o.status = 'refunded'", status: "refused", detail: "never-send column email", maySee: "counts" },
      { at: Date.now() - 75_000, app: "Claude Code", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "SELECT COUNT(*) AS refunded, COUNT(DISTINCT customer_id) AS customers FROM orders WHERE status = 'refunded' AND ordered_at >= '2026-10-01'", status: "ok", rows: 1, maySee: "counts" },
      { at: Date.now() - 600_000, app: "Cursor", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "DELETE FROM orders WHERE status = 'cancelled'", status: "refused", detail: "could change data", maySee: "counts" },
    ]
  : [
      { at: Date.now() - 30_000, app: "Claude Code", tool: "suggest_note", connectionId: "demo", connection: "Demo store (SQLite)", sql: "", status: "ok", detail: "reviews.rating is 1 to 5; 0 means the review was removed" },
      { at: Date.now() - 60_000, app: "Claude Code", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "SELECT country, COUNT(*) FROM customers GROUP BY country", status: "ok", rows: 12, maySee: "counts" },
      { at: Date.now() - 150_000, app: "Cursor", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "SELECT c.country, strftime('%Y-%m', o.ordered_at) AS month, COUNT(DISTINCT o.id) AS orders, ROUND(SUM(oi.quantity * oi.unit_price), 2) AS revenue FROM orders o JOIN customers c ON c.id = o.customer_id JOIN order_items oi ON oi.order_id = o.id WHERE o.status = 'paid' AND o.ordered_at >= date('now', '-6 months') GROUP BY c.country, month ORDER BY month DESC, revenue DESC", status: "ok", rows: 48, maySee: "counts" },
      { at: Date.now() - 240_000, app: "Cursor", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "SELECT email FROM customers", status: "refused", detail: "never-send column email", maySee: "counts" },
      { at: Date.now() - 600_000, app: "Claude Code", tool: "run_query", connectionId: "demo", connection: "Demo store (SQLite)", sql: "DELETE FROM orders WHERE id = 7", status: "refused", detail: "could change data", maySee: "counts" },
      // Refused while only approved apps were let in; it waits for approval.
      { at: Date.now() - 900_000, app: "Windsurf", tool: "list_connections", connectionId: "", connection: "", sql: "", status: "refused", detail: "not approved" },
    ];
function mcpApps() {
  const names = new Set([...mcpActivity.map((e) => e.app), ...mcpBlocked, ...mcpApproved]);
  const apps = [...names].map((name) => {
    const mine = mcpActivity.filter((e) => e.app === name);
    const queries = mine.filter((e) => e.tool === "run_query" || e.tool === "export_csv").length;
    return { name, blocked: mcpBlocked.has(name), approved: mcpApproved.has(name), queries, lastUsed: mine.length ? Math.max(...mine.map((e) => e.at)) : null };
  });
  apps.sort((x, y) => (y.lastUsed ?? 0) - (x.lastUsed ?? 0) || x.name.localeCompare(y.name));
  return { onlyApproved: mcpOnlyApproved, apps };
}

async function mockAgentRun(runId: string, question: string, access: AgentAccess, auto: boolean) {
  const pause = () => new Promise((r) => setTimeout(r, VIDEO ? 900 : 500));
  const reply = () => new Promise<AgentReply>((resolve) => waiting.set(runId, resolve));
  const event = (e: Record<string, unknown>) => emit("agent://event", { runId, ...e });
  const usage = (turns: number, queries: number) =>
    event({ type: "progress", turns, maxTurns: 12, queries, maxQueries: 10, usage: { input: 900 * turns, cached: 4200 * turns, output: 180 * turns }, elapsedMs: turns * 1400 });
  const step = (id: number, patch: Partial<AgentStep>): AgentStep => ({
    id, tool: "run_query", text: "", status: "running", sql: null, rowCount: null, truncated: false, elapsedMs: null,
    columns: [], rows: [], sent: null, sentSummary: null, checks: [], error: null, ...patch,
  });
  await pause();
  await usage(1, 0);
  await event({ type: "plan", steps: ["Count paid orders per week", "Split the last week by category", "Compare with the week before"], waiting: !auto });
  if (!auto) await reply();
  if (stopped.has(runId)) return event({ type: "done", answer: null, error: null, stopped: true });
  const weekly = "SELECT strftime('%Y-%W', ordered_at) AS week, COUNT(*) AS paid FROM orders WHERE status = 'paid' GROUP BY week ORDER BY week DESC LIMIT 2";
  await event({ type: "step", step: step(1, { text: "Counting paid orders in the last two weeks", sql: weekly }) });
  await pause();
  const rows1 = [["2026-42", 31], ["2026-41", 52]];
  const sent1 =
    access.results === "rows" ? `Step 1: 2 rows.\nColumns: ["week","paid"]\n["2026-42",31]\n["2026-41",52]` : access.results === "counts" ? `Step 1: Summary: {"rowCount":2,"columns":[{"name":"week","type":"text","nulls":0,"distinct":2},{"name":"paid","type":"number","nulls":0,"distinct":2,"min":31,"max":52}]}` : `Step 1: 2 rows. Columns: week, paid. No values are sent on this connection.`;
  const summary1 = { rows: "sent 2 rows", counts: "sent counts and column summaries", off: "sent row count only" }[access.results];
  if (access.askBeforeSend && !auto && access.results !== "off") {
    await event({ type: "send", stepId: 1, payload: sent1 });
    const r = await reply();
    if (stopped.has(runId)) return event({ type: "done", answer: null, error: null, stopped: true });
    if (r.type === "skip") {
      await event({ type: "step", step: step(1, { text: "Counting paid orders in the last two weeks", sql: weekly, status: "done", rowCount: 2, elapsedMs: 4, columns: ["week", "paid"], rows: rows1, sent: "Step 1: 2 rows. The person chose not to send this result.", sentSummary: "skipped: sent row count only" }) });
    }
  }
  await event({ type: "step", step: step(1, { text: "Counting paid orders in the last two weeks", sql: weekly, status: "done", rowCount: 2, elapsedMs: 4, columns: ["week", "paid"], rows: rows1, sent: sent1, sentSummary: summary1 }) });
  await usage(2, 1);
  await pause();
  const byCat = "SELECT c.name, COUNT(DISTINCT o.id) AS paid FROM orders o JOIN order_items oi ON oi.order_id = o.id JOIN products p ON p.id = oi.product_id JOIN categories c ON c.id = p.category_id WHERE o.status = 'paid' AND strftime('%Y-%W', o.ordered_at) = '2026-42' GROUP BY c.name ORDER BY paid";
  await event({ type: "step", step: step(2, { text: "Splitting last week's paid orders by category", sql: byCat }) });
  await pause();
  if (stopped.has(runId)) return event({ type: "done", answer: null, error: null, stopped: true });
  await event({ type: "step", step: step(2, { text: "Splitting last week's paid orders by category", sql: byCat, status: "done", rowCount: 3, elapsedMs: 9, columns: ["name", "paid"], rows: [["Electronics", 2], ["Books", 11], ["Toys", 18]], sent: "Step 2: 3 rows.", sentSummary: summary1 }) });
  await usage(3, 2);
  await pause();
  await event({
    type: "done",
    error: null,
    stopped: false,
    answer: {
      text: "Paid orders fell from 52 to 31. Electronics had only 2 paid orders last week, the biggest drop.",
      sentences: [
        { text: "Paid orders fell from 52 to 31.", steps: [1], unverified: [] },
        { text: "Electronics had only 2 paid orders last week, the biggest drop.", steps: [2], unverified: [] },
      ],
      confidence: "medium",
      assumptions: ["A week runs Monday to Sunday."],
      notes: ["orders.status 'paid' = paid but not yet shipped"],
      proposedSql: null,
      proposedChangesData: false,
      cannotAnswer: false,
      unverified: [],
      model: "claude-opus-5-5",
      usage: { input: 2700, cached: 12600, output: 540 },
    },
  });
  void question;
}

/** `?os=windows` or `?os=linux` shows the MCP setup for that OS (macOS otherwise). */
const MOCK_OS = new URLSearchParams(window.location.search).get("os");
/**
 * Coding agents that look installed: `?installed=cursor,vscode` lists them, `?agents=none` none;
 * otherwise Claude Code, Cursor, VS Code, Codex and Claude Desktop.
 */
function installedCodingAgents(): string[] {
  const list = new URLSearchParams(window.location.search).get("installed");
  if (list !== null) return list.split(",").filter(Boolean);
  return AGENTS === "none" ? [] : ["claude-code", "claude-desktop", "cursor", "vscode", "codex"];
}

/**
 * "Run audit": `?agents=none` finds no coding agent, `?agents=old` an outdated Claude Code;
 * otherwise Claude Code is ready and Codex is found but not offered.
 */
const AGENTS = new URLSearchParams(window.location.search).get("agents");
function auditAgents(): AuditAgent[] {
  const none = AGENTS === "none";
  const old = AGENTS === "old";
  return [
    { id: "claude", label: "Claude Code", path: none ? null : "/Users/you/.local/bin/claude", version: none ? null : old ? "2.0.14 (Claude Code)" : "2.1.267 (Claude Code)",
      supported: true, ready: !none && !old, installUrl: "https://code.claude.com/docs/en/setup",
      problem: old ? "Claude Code 2.0.14 is too old to run read-only from SayQL. Update it to 2.1.259 or later: run `claude update` in a terminal." : null },
    { id: "codex", label: "Codex CLI", path: none ? null : "/opt/homebrew/bin/codex", version: null, supported: false, ready: false, installUrl: "https://developers.openai.com/codex/cli",
      problem: none ? null : "Codex reads code by running shell commands, so SayQL can't limit it to reading files. Copy the audit prompt into Codex instead." },
    { id: "gemini", label: "Gemini CLI", path: null, version: null, supported: false, ready: false, installUrl: "https://geminicli.com", problem: null },
    { id: "cursor", label: "Cursor CLI", path: null, version: null, supported: false, ready: false, installUrl: "https://cursor.com/cli", problem: null },
  ];
}
// What each connection's audit reads: a folder and a GitHub repository for analytics.
const knowledgeSources = new Map<string, KnowledgeSource[]>([
  [
    "pg",
    [
      { id: "src_analytics", label: "analytics", kind: { type: "local", path: "/Users/you/code/analytics" }, lastFetch: Date.now() - 3 * 86_400_000, lastCommit: "9f8e7d6c5b4a", error: null },
      { id: "src_api", label: "api", kind: { type: "git", url: "https://github.com/acme/api.git", branch: "main", subdir: null, hasToken: false, tokenUser: null },
        lastFetch: Date.now() - 3 * 86_400_000, lastCommit: "1a2b3c4d5e6f", error: null },
    ],
  ],
]);
let mockSourceId = 0;
const sourcesOf = (id: unknown) => knowledgeSources.get(id as string) ?? [];
/** Adds a source as the app does: a 4th is refused. */
const addSource = (id: unknown, source: KnowledgeSource) => {
  if (sourcesOf(id).length >= MAX_SOURCES_FREE) return Promise.reject(SOURCE_CAP_MESSAGE);
  knowledgeSources.set(id as string, [...sourcesOf(id), source]);
  return source;
};
const auditStops = new Set<string>();
let auditModel = "sonnet";
// `?nocredits` in the mock's URL: no SayQL credit key saved.
const mockCreditKey = typeof location === "undefined" || !location.search.includes("nocredits");

/** A scripted audit: schema, searches, reads, two suggestions, a summary. */
async function mockAuditRun(runId: string, connectionId: string, database: string | null) {
  const pause = () => new Promise((r) => setTimeout(r, 650));
  const line = (kind: "step" | "note" | "warn", text: string) => emit("audit://event", { type: "line", runId, kind, text });
  const done = (e: Record<string, unknown>) => emit("audit://event", { type: "done", runId, suggested: 0, summary: null, error: null, ...e });
  for (const s of sourcesOf(connectionId)) {
    if (s.kind.type !== "git") continue;
    await line("step", `Fetching ${s.label}…`);
    await pause();
    s.lastFetch = Date.now();
  }
  const steps: ["step" | "note" | "warn", string][] = [
    ["step", "Started, with SayQL's tools"],
    ["step", "Reading the schema, usage and saved notes"],
    ["step", "Listing files matching **/*.{ts,sql}"],
    ["step", "Searching for “legacy_orders” in api"],
    ["step", "Reading api/orders/status.ts"],
    ["step", "Searching for “status” in api/orders"],
    ["note", "Suggested a note: orders.status 'refunded' is set by api/orders/refund.ts; there is no refunds table"],
    ["step", "Reading db/migrations/0042_reviews.sql"],
    ["note", "Suggested a note: reviews.rating is 1 to 5; 0 means the review was removed"],
  ];
  let suggested = 0;
  for (const [kind, text] of steps) {
    await pause();
    if (auditStops.delete(runId)) return done({ outcome: "cancelled", suggested });
    await line(kind, text);
    if (kind === "note") {
      suggested += 1;
      suggestions = [...suggestions, { id: `sug-audit-${runId}-${suggested}`, connectionId, database, app: "Claude Code", createdAt: Date.now(),
        text: text.replace(/^Suggested a note: /, ""), evidence: "Found by the audit in the preview." }];
    }
  }
  await pause();
  const i = audits.findIndex((x) => x.connectionId === connectionId && x.database === database);
  if (i >= 0) audits.splice(i, 1);
  audits.push({ connectionId, database, app: "Claude Code", at: Date.now() });
  return done({
    outcome: "ok",
    suggested,
    summary:
      "## Audit of `analytics`\n\n**2 suggestions are waiting in SayQL.** Nothing is saved until you accept it.\n\n### What I found\n\n- `orders.status` 'refunded' is set by `api/orders/refund.ts`; there is no refunds table.\n- `reviews.rating` is 1 to 5; **0** means the review was removed.\n\n### What I couldn't check\n\n1. I couldn't run `git log`, so I left out when `legacy_orders` stopped being used.\n2. Usage figures cover only the last stats reset.",
  });
}

export function installMockBackend() {
  // Before the app reads its remembered tabs: a first run remembers nothing.
  if (FIRSTRUN) setStorageNamespace("sayql-firstrun:", true);
  // The video and the README screenshots: no What's new, no Get started over the app.
  if (VIDEO) {
    storage.set("sayql.whatsNewSeen", version);
    storage.set("sayql.checklistHidden", true);
  }
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "init":
          return boot;
        case "open_database":
        case "refresh_schema":
          await new Promise((r) => setTimeout(r, 300));
          return schema;
        case "list_databases":
          return a.id === "demo" ? null : { databases: ["analytics", "master", "sales", "staging"], current: "master" };
        case "run_query":
          if (String(a.sql).includes("first_nme")) return Promise.reject(VIDEO ? "no such column: first_nme" : "Invalid column name 'first_nme'.");
          if (!isReadOnly(String(a.sql), boot.connections.find((c) => c.id === a.id)?.kind)) {
            if (!a.allowWrite) return Promise.reject("WRITE_CONFIRM");
            return { columns: [], rows: [], rowsAffected: 1, truncated: false, elapsedMs: 3 };
          }
          if (/from\s+orders\s*;?\s*$/i.test(String(a.sql))) return orders;
          if (String(a.sql).includes("c.country IN")) return europe;
          if (String(a.sql).includes("start of month")) return newCustomers;
          if (String(a.sql).includes("'Japan'")) return japan;
          return String(a.sql).includes("ORDER BY total_spend DESC") ? topCustomers : result;
        case "generate_sql":
          if (a.skill && !skills.some((s) => s.slug === a.skill)) return Promise.reject(`There's no skill called /${a.skill}. Create it in Settings.`);
          void streamSql(Number(a.reqId), mockAnswer(String(a.question), a.task as { kind: string }, (a.skill as string | null) ?? null));
          return null;
        case "cancel_generation":
        case "cancel_query":
          return null;
        case "start_agent": {
          const access = agentAccess.get(String(a.id));
          if (!access?.enabled) return Promise.reject("Investigate is off for this connection. Turn it on in its Investigate settings first.");
          void mockAgentRun(String(a.runId), String(a.question), access, a.auto === true);
          return null;
        }
        case "stop_agent":
          stopped.add(String(a.runId));
          waiting.get(String(a.runId))?.({ type: "skip" });
          return null;
        case "reply_agent": {
          const resolve = waiting.get(String(a.runId));
          waiting.delete(String(a.runId));
          resolve?.(a.reply as AgentReply);
          return null;
        }
        case "agent_access":
          return agentAccess.get(String(a.id)) ?? { enabled: false, results: "off", apps: "counts", askBeforeSend: true, neverSend: [] };
        case "save_agent_access":
          agentAccess.set(String(a.id), a.access as AgentAccess);
          return a.access;
        case "list_notes":
          return notes.filter((n) => n.connectionId === a.id);
        // The dev UI has no team server: every connection uses the notes saved here.
        case "list_shared_notes":
          return null;
        case "suggest_note":
          return "Sent to your team's knowledge owners for approval.";
        case "save_note": {
          const note = a.note as DbNote;
          if (/@|‹/.test(note.text)) return Promise.reject("A note describes the database, not the data in it. Remove the value (an email, number or name) from it.");
          const saved = { ...note, id: note.id || crypto.randomUUID(), updatedAt: Date.now() };
          notes = [...notes.filter((n) => n.id !== saved.id), saved];
          return saved;
        }
        case "delete_note":
          notes = notes.filter((n) => n.id !== a.id);
          return null;
        case "confirm_note": {
          notes = notes.map((n) => (n.id === a.id ? { ...n, suspect: null } : n));
          return notes.find((n) => n.id === a.id);
        }
        case "check_note": {
          const at = Date.now();
          notes = notes.map((n) => (n.id === a.id && n.check ? { ...n, check: { ...n.check, last: { ok: true, at } }, verifiedAt: at } : n));
          return notes.find((n) => n.id === a.id);
        }
        case "check_notes":
          return notes.filter((n) => n.connectionId === a.id);
        case "list_note_suggestions":
          return suggestions;
        case "accept_note_suggestion": {
          const s = suggestions.find((x) => x.id === a.id);
          if (!s) return Promise.reject("This suggestion is no longer waiting.");
          suggestions = suggestions.filter((x) => x.id !== a.id);
          // A removal deletes the note it names; a replacement keeps that note's id.
          if (s.remove) {
            notes = notes.filter((n) => n.id !== s.replaces);
            return null;
          }
          const id = s.replaces && notes.some((n) => n.id === s.replaces) ? s.replaces : crypto.randomUUID();
          const saved: DbNote = { id, connectionId: s.connectionId, database: s.database, text: s.text, updatedAt: Date.now() };
          notes = [...notes.filter((n) => n.id !== id), saved];
          return saved;
        }
        case "mcp_audits":
          return audits;
        case "audit_prompt": {
          const c = boot.connections.find((x) => x.id === a.id);
          if (!c) return Promise.reject("No saved connection has this id.");
          const db = (a.database as string | null) ?? (c.database || null);
          const where = db ? `, database "${db}"` : "";
          return (
            `Audit the database of the SayQL connection "${c.name}" (connection_id "${c.id}"${where}) against the code in this project, ` +
            "and suggest notes that teach SayQL what the schema doesn't say.\n\n" +
            "Use the SayQL MCP server's tools for this (describe_schema, suggest_note). (The app gives the full instructions; the browser preview shortens them.)"
          );
        }
        case "audit_agents":
          await new Promise((r) => setTimeout(r, 250));
          return auditAgents();
        case "knowledge_sources":
          return sourcesOf(a.id).map((s) => ({ ...s }));
        case "add_knowledge_folder": {
          const path = a.id === "demo" ? "/Users/you/code/shop" : "/Users/you/code/analytics";
          if (sourcesOf(a.id).some((s) => s.kind.type === "local" && s.kind.path === path)) return Promise.reject("This connection already has that source.");
          return addSource(a.id, { id: `src_mock${++mockSourceId}`, label: path.split("/").pop() ?? "code", kind: { type: "local", path }, lastFetch: Date.now(), lastCommit: null, error: null });
        }
        case "add_knowledge_files":
          return addSource(a.id, {
            id: `src_mock${++mockSourceId}`, label: "docs", lastFetch: Date.now(), lastCommit: "4e5f6a7b8c9d", error: null,
            kind: { type: "files", paths: ["/Users/you/Documents/billing-spec.pdf", "/Users/you/Documents/status-codes.md"] },
          });
        case "add_knowledge_url": {
          const url = String(a.url ?? "").trim();
          if (!/^https?:\/\/[^/]+/.test(url)) return Promise.reject("Enter the page's full address, starting with https://.");
          if (/^https?:\/\/(localhost|127\.|10\.|192\.168\.)/.test(url)) return Promise.reject("SayQL reads public web pages only, and that address is on this computer or a private network.");
          const label = url.replace(/^https?:\/\//, "").split(/[/:]/)[0] || "docs";
          return addSource(a.id, { id: `src_mock${++mockSourceId}`, label, lastFetch: Date.now(), lastCommit: "a1b2c3d4e5f6", error: null, kind: { type: "url", url, crawl: !!a.crawl } });
        }
        case "add_knowledge_repo": {
          await new Promise((r) => setTimeout(r, 250));
          const url = String(a.url ?? "").trim();
          if (!/^(https:\/\/|ssh:\/\/|git@)/.test(url)) return Promise.reject("Use the repository's https:// or SSH address (git@host:owner/repo.git), as its Clone button shows it.");
          if (url.includes("private")) return Promise.reject("github.com didn't accept the sign-in. Add a read-only token for this repository, or set up git's credential helper for github.com.");
          const subdir = String(a.subdir ?? "").trim() || null;
          const label = (subdir ?? url).replace(/\.git$/, "").split(/[/:]/).pop() || "code";
          return addSource(a.id, { id: `src_mock${++mockSourceId}`, label, lastFetch: null, lastCommit: null, error: null,
            kind: { type: "git", url, branch: String(a.branch ?? "").trim(), subdir, hasToken: !!a.token, tokenUser: (a.tokenUser as string | null) ?? null } });
        }
        case "remove_knowledge_source":
          knowledgeSources.set(a.id as string, sourcesOf(a.id).filter((s) => s.id !== a.sourceId));
          return null;
        case "refresh_knowledge_source": {
          await new Promise((r) => setTimeout(r, 150));
          const s = sourcesOf(a.id).find((x) => x.id === a.sourceId);
          if (!s) return Promise.reject("That source was removed.");
          s.error = null;
          if (s.kind.type === "files" || s.kind.type === "url") s.lastFetch = Date.now();
          return { ...s };
        }
        case "start_audit": {
          const agent = auditAgents().find((x) => x.id === a.agent);
          if (!agent?.ready) return Promise.reject(agent?.problem ?? "Claude Code isn't installed on this computer, or SayQL can't find it.");
          if (sourcesOf(a.id).length === 0) return Promise.reject("Add a code source first: a folder or repository with this database's code.");
          void mockAuditRun(a.runId as string, a.id as string, (a.database as string | null) ?? null);
          return null;
        }
        case "audit_model":
          return auditModel;
        case "set_audit_model":
          auditModel = a.model as string;
          return null;
        case "audit_payment":
          // As the app: paying for the audit with credits is off, so it's always the person's own sign-in.
          return { payment: "subscription", hasCreditKey: mockCreditKey };
        case "set_audit_payment":
          return null;
        case "cancel_audit":
          auditStops.add(a.runId as string);
          return null;
        case "open_audit_in_terminal":
          return null;
        case "dismiss_note_suggestion":
          suggestions = suggestions.filter((x) => x.id !== a.id);
          return null;
        case "save_agent_settings":
          Object.assign(settings, { agentModel: a.agentModel, agent: a.agent });
          settings.providers = settings.providers.map((p) => (p.id === settings.provider ? { ...p, agentModel: a.agentModel as string } : p));
          return { ...settings };
        case "choose_model": {
          const field = a.purpose === "agent" ? "agentModel" : "typingModel";
          const chosen = settings.providers.find((p) => p.id === a.provider);
          if (!chosen?.ready) return Promise.reject(`${chosen?.label ?? a.provider} isn't set up yet.`);
          settings.providers = settings.providers.map((p) => (p.id === a.provider ? { ...p, [field]: a.model as string } : p));
          const p = settings.providers.find((q) => q.id === a.provider)!;
          Object.assign(settings, { provider: p.id, typingModel: p.typingModel, expertModel: p.expertModel, agentModel: p.agentModel });
          return { ...settings };
        }
        case "list_scripts":
          return [];
        case "save_script": {
          const script = a.script as { id: string };
          return { ...script, id: script.id || crypto.randomUUID(), updatedAt: Date.now() };
        }
        case "delete_script":
          return null;
        case "list_skills":
          return skills;
        case "save_skill": {
          const original = (a.original as string | null) ?? null;
          const raw = a.skill as Skill;
          const problem = skillError(raw, skills, original);
          if (problem) return Promise.reject(problem);
          const skill = { slug: raw.slug.trim().replace(/^\//, ""), description: raw.description.trim(), instruction: raw.instruction.trim() };
          const i = skills.findIndex((s) => s.slug === original);
          skills = i < 0 ? [...skills, skill] : skills.map((s, j) => (j === i ? skill : s));
          return skill;
        }
        case "delete_skill":
          skills = skills.filter((s) => s.slug !== a.slug);
          return null;
        case "app_info":
          return { version, os: "macos", arch: "aarch64" };
        case "install_update":
          return Promise.reject("Updates aren't available in the browser preview.");
        case "plugin:opener|open_url":
          window.open(String(a.url), "_blank");
          return null;
        case "check_update":
          return null;
        case "take_handoffs":
          return [];
        // Signed out of SayQL Team: there is no team server in the browser preview.
        case "team_status":
          return null;
        case "team_connections":
          return [];
        case "team_ai":
          return null;
        case "team_sign_out":
          return null;
        case "credits_balance":
          return null;
        case "team_sign_in":
        case "team_sso_start":
        case "team_sso_finish":
          return Promise.reject("Signing in to a team isn't available in the browser preview.");
        case "mcp_setup":
          return MOCK_OS === "windows"
            ? { path: "C:\\Program Files\\SayQL\\sayql-mcp.exe", exists: true, appimage: false, os: "windows" }
            : MOCK_OS === "linux"
              ? { path: "/usr/bin/sayql-mcp", exists: true, appimage: false, os: "linux" }
              : { path: "/Applications/SayQL.app/Contents/MacOS/sayql-mcp", exists: true, appimage: false, os: "macos" };
        case "installed_coding_agents":
          return installedCodingAgents();
        case "open_install_link":
          // The browser preview can't open Cursor or VS Code; the real app asks the OS to.
          console.info("open_install_link", a.url);
          return null;
        case "mcp_activity":
          return mcpActivity;
        case "mcp_apps":
          return mcpApps();
        case "set_mcp_app_blocked": {
          const name = String(a.name);
          if (a.blocked) mcpBlocked.add(name);
          else {
            mcpBlocked.delete(name);
            mcpApproved.add(name);
          }
          return mcpApps();
        }
        case "approve_mcp_app":
          mcpApproved.add(String(a.name));
          return mcpApps();
        case "set_mcp_only_approved":
          // Turning it on approves the apps already seen that aren't blocked, like the backend.
          if (a.on && !mcpOnlyApproved)
            for (const e of mcpActivity) if (e.detail !== "not approved" && !mcpBlocked.has(e.app)) mcpApproved.add(e.app);
          mcpOnlyApproved = !!a.on;
          return mcpApps();
        case "clear_mcp_activity":
          mcpActivity.length = 0;
          return null;
        case "test_connection":
          return "Connected. Found 6 tables and views.";
        case "save_connection":
          return { ...(a.config as object), id: (a.config as { id: string }).id || crypto.randomUUID() };
        case "delete_connection":
          boot.connections = boot.connections.filter((c) => c.id !== a.id);
          return null;
        case "save_settings": {
          const id = a.provider as string;
          const providers = settings.providers.map((p) =>
            p.id === id ? { ...p, typingModel: a.typingModel as string, expertModel: a.expertModel as string, baseUrl: a.baseUrl as string, hasKey: p.hasKey || !!a.apiKey } : p,
          );
          const chosen = providers.find((p) => p.id === id);
          Object.assign(settings, {
            provider: id,
            typingModel: a.typingModel,
            expertModel: (a.expertModel as string) || a.typingModel,
            workspaceId: a.workspaceId,
            providers,
            // Like the backend: ready once the provider has what it needs (a key, unless it needs none).
            aiReady: !!chosen && (chosen.key !== "required" || chosen.hasKey),
          });
          return { ...settings };
        }
        case "finish_onboarding":
          settings.onboarded = true;
          return { ...settings };
        case "voice_status":
          return { ready: voice.ready, downloading: voice.downloading, modelBytes: VOICE_BYTES };
        case "voice_download":
          return fakeDownload();
        case "voice_cancel_download":
          voice.cancel = true;
          return null;
        case "voice_delete_model":
          voice.ready = false;
          return null;
        case "voice_start":
          if (VOICE === "nomic") return Promise.reject("No microphone found.");
          fakeMic();
          return null;
        case "voice_stop":
          clearInterval(voice.meter);
          await new Promise((r) => setTimeout(r, 600));
          return "top ten customers by total spend";
        case "voice_cancel":
          clearInterval(voice.meter);
          return null;
        case "my_activity": {
          const id = a.connectionId as string | null;
          return myActivity.filter((e) => !id || e.connectionId === id);
        }
        case "clear_my_activity":
          myActivity.length = 0;
          return null;
        case "check_claude_plan":
          await new Promise((r) => setTimeout(r, 500));
          if (NOPLAN)
            throw "Claude Code isn't signed in. Run `claude` in a terminal and sign in, then check again.";
          return { version: "2.1.308", authMethod: "claude.ai", plan: "max" };
        case "list_models":
          if (a.provider === "anthropic") return settings.models.map((m) => m.id);
          return a.provider === "ollama" ? ["llama3.3:70b", "qwen3:32b", "sqlcoder:15b"] : ["model-small", "model-large"];
        default:
          // Fail loudly so a command added to src/api/commands.ts without a mock here shows up.
          throw new Error(`mockBackend: no mock for command "${cmd}"`);
      }
    },
    { shouldMockEvents: true },
  );
}
