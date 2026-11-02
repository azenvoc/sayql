// Records the explainer's scenes from the UI preview (vite + src/dev/mockBackend.ts in ?video mode)
// and from a throwaway team server's admin page (team-server.sh).
//
//   pnpm exec vite --port 1431 --strictPort        # in another terminal
//   scripts/video/team-server.sh                   # in another; a fresh one for each capture
//   SAYQL_ADMIN_LINK=… SAYQL_TEAM_DB=… node scripts/video/capture.mjs [dark|light]
//
// Writes scripts/video/out/<theme>/frames/*.jpg (a 30 fps screencast at 2×) and timeline.json:
// scene marks, camera targets (element rects, in CSS px), captions and cursor moves. render.mjs
// turns that into the videos, so the storyboard lives here and render.mjs stays generic.
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "./cdp.mjs";

const theme = process.argv[2] === "light" ? "light" : "dark";
const URL = process.env.SAYQL_URL ?? "http://localhost:1431/?video";
/** The app window in CSS px; frames are twice that. */
export const VIEW = { width: 1440, height: 810, scale: 2 };

const out = join(import.meta.dirname, "out", theme);
rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, "frames"), { recursive: true });

const page = await launch(VIEW);
await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
await page.goto(URL);
// A taller editor, so the longest query in the video fits without scrolling.
await page.evaluate(() => localStorage.setItem("split:editor", "440"));
await page.goto(URL);
await page.evaluate(() => document.fonts.ready.then(() => true));
await sleep(2500); // schema load and Monaco
// Headless Chrome only sends screencast frames when something repaints: keep a 1px dot in
// the corner changing (invisible in the video) so frames come steadily. Again after each page load.
const keepRepainting = () =>
  page.evaluate(() => {
    const dot = document.createElement("div");
    dot.style.cssText = "position:fixed;right:0;bottom:0;width:1px;height:1px;z-index:99999;pointer-events:none";
    document.body.appendChild(dot);
    let i = 0;
    setInterval(() => (dot.style.background = `rgba(128,128,128,${(i++ % 2) * 0.02 + 0.01})`), 16);
  });
await keepRepainting();
// Nothing connects on its own (0.4.8): open the demo store, off camera, before recording.
await page.evaluate(() => {
  const label = [...document.querySelectorAll('[role="treeitem"] span')].find((e) => e.textContent.trim() === "Demo store (SQLite)");
  label?.click();
  return !!label;
});
for (const end = Date.now() + 8000; Date.now() < end; await sleep(100)) {
  if (await page.evaluate(() => !!document.querySelector('textarea[aria-label="Ask in plain English"]'))) break;
}
await sleep(1500); // schema load

// ── Recording ──────────────────────────────────────────────────────────────
const now = () => Date.now() / 1000;
const frames = [];
const events = [];
const log = (type, data = {}) => events.push({ t: now(), type, ...data });
page.on("Page.screencastFrame", (f) => {
  const file = `${String(frames.length).padStart(5, "0")}.jpg`;
  writeFileSync(join(out, "frames", file), Buffer.from(f.data, "base64"));
  frames.push({ t: f.metadata.timestamp, file });
  lastFrame = Date.now();
  void page.send("Page.screencastFrameAck", { sessionId: f.sessionId });
});
const startScreencast = () =>
  page.send("Page.startScreencast", {
    format: "jpeg",
    quality: 88,
    maxWidth: VIEW.width * VIEW.scale,
    maxHeight: VIEW.height * VIEW.scale,
    everyNthFrame: 2,
  });
let lastFrame = Date.now();
let restarts = 0;
await startScreencast();
// The screencast sometimes stops sending frames for good; restart it when that happens.
const watchdog = setInterval(async () => {
  if (Date.now() - lastFrame < 400) return;
  lastFrame = Date.now();
  restarts++;
  await page.send("Page.stopScreencast");
  await startScreencast();
}, 100);

const progress = setInterval(() => process.env.DEBUG && console.log(`${events.at(-1)?.type ?? "-"} frames=${frames.length} restarts=${restarts}`), 2000);

// ── Page helpers ───────────────────────────────────────────────────────────
/** The rect of the element matching `sel` (and whose text is `text`, if given), or null. */
const rect = (sel, text = null) =>
  page.evaluate(
    (sel, text) => {
      const els = [...document.querySelectorAll(sel)].filter((e) => {
        const r = e.getBoundingClientRect();
        return r.width > 0 && r.height > 0 && (text === null || e.textContent.trim() === text || e.getAttribute("aria-label") === text);
      });
      const r = els[0]?.getBoundingClientRect();
      return r ? { x: r.x, y: r.y, w: r.width, h: r.height } : null;
    },
    sel,
    text,
  );

/** Waits until `sel` (with `text`) is on screen and returns its rect. */
async function waitFor(sel, text = null, timeout = 8000) {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(50)) {
    const r = await rect(sel, text);
    if (r) return r;
  }
  throw new Error(`Timed out waiting for ${sel} ${text ?? ""}`);
}

const waitForText = async (text, timeout = 8000) => {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(50))
    if (await page.evaluate((t) => document.body.innerText.includes(t), text)) return;
  const shown = await page.evaluate(() => document.body.innerText.slice(-600));
  throw new Error(`Timed out waiting for "${text}". The page ends with:\n${shown}`);
};

const union = (...rs) => {
  const x = Math.min(...rs.map((r) => r.x));
  const y = Math.min(...rs.map((r) => r.y));
  return { x, y, w: Math.max(...rs.map((r) => r.x + r.w)) - x, h: Math.max(...rs.map((r) => r.y + r.h)) - y };
};

let mouse = { x: VIEW.width * 0.62, y: VIEW.height * 0.7 };

/** Glides the (drawn) cursor to the middle of `r`, then clicks there for real. */
async function click(r, { modifiers = 0, glide = 650 } = {}) {
  const to = { x: r.x + r.w / 2, y: r.y + r.h / 2 };
  log("cursor", { x: to.x, y: to.y, dur: glide / 1000 });
  await sleep(glide + 120);
  mouse = to;
  await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...to, modifiers });
  log("click", { x: to.x, y: to.y });
  await page.send("Input.dispatchMouseEvent", { type: "mousePressed", ...to, button: "left", clickCount: 1, modifiers });
  await sleep(70);
  await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...to, button: "left", clickCount: 1, modifiers });
  await sleep(250);
}

const KEYS = { Enter: 13, Escape: 27, Backspace: 8 };
async function press(key) {
  // A real Escape stops headless Chrome's screencast for good, so it goes in as a DOM event.
  if (key === "Escape")
    return page.evaluate(() => {
      const el = document.activeElement ?? document.body;
      el.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true }));
      return true;
    });
  const k = { key, code: key, windowsVirtualKeyCode: KEYS[key], nativeVirtualKeyCode: KEYS[key] };
  await page.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...k });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...k });
}

/** Types like a person: uneven gaps, a little longer after spaces. Never long enough to trigger a draft mid-word. */
async function type(text) {
  for (const ch of text) {
    await page.send("Input.insertText", { text: ch });
    await sleep(ch === " " ? 95 + Math.random() * 60 : 55 + Math.random() * 55);
  }
}

const focusAsk = () => page.evaluate(() => document.querySelector('textarea[aria-label="Ask in plain English"]').focus());
const ask = () => waitFor('textarea[aria-label="Ask in plain English"]');
const askBar = async () => {
  const r = await ask();
  return { x: r.x - 40, y: r.y - 14, w: VIEW.width - r.x, h: 70 };
};
const editor = () => waitFor(".monaco-editor");
const results = async () => {
  const head = await waitFor("*", "Results");
  return { x: head.x - 20, y: head.y - 10, w: VIEW.width - head.x + 20, h: VIEW.height - head.y - 20 };
};

/** The Agent view below the editor: from its tab to the bottom of the window. */
const agentArea = async () => {
  const tab = await waitFor('[role="tab"]', "Investigation");
  return { x: tab.x - 20, y: tab.y - 10, w: VIEW.width - tab.x + 20, h: VIEW.height - tab.y - 20 };
};

/** Scrolls the element matching `sel` whose text is `text` into view, inside its scrolling panel. */
const reveal = (sel, text) =>
  page.evaluate(
    (sel, text) => {
      const el = [...document.querySelectorAll(sel)].find((e) => e.offsetParent && e.textContent.trim().startsWith(text));
      el?.scrollIntoView({ block: "nearest" });
      return !!el;
    },
    sel,
    text,
  );

/** The rect of the innermost visible element whose text contains `text`. */
const textRect = (text) =>
  page.evaluate((text) => {
    const els = [...document.querySelectorAll("body *")].filter((e) => e.offsetParent && e.textContent.includes(text));
    const r = els.at(-1).getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  }, text);

const scene = (name) => log("scene", { name });
const cam = (r, opts = {}) => log("cam", { rect: r, ...opts });
const caption = (text) => log("caption", { text });
const hideCursor = () => log("cursor", { hidden: true });

// ── Storyboard ─────────────────────────────────────────────────────────────
// The order here is the capture's; render.mjs puts the scenes in the video's order.
// 1. Ask: type the question, SQL streams in.
scene("ask");
cam(null);
caption("Ask in plain English");
await sleep(1000);
cam(union(await askBar(), await editor()), { maxZoom: 1.55 });
await sleep(900);
await focusAsk();
await type("Top 10 customers by spend");
caption("SQL appears as you type. No waiting.");
await waitForText("Haiku 5.5 · first SQL");
await sleep(1400);

// 1b. Run it.
scene("run");
cam(null);
caption("It already knows your schema");
await press("Enter");
await waitForText("Kenji Watanabe");
await sleep(400);
cam(union(await results(), await askBar()), { maxZoom: 1.3 });
await sleep(3800);

// 2. Safety: a change needs your OK.
scene("safety");
cam(union(await askBar(), await editor()), { maxZoom: 1.55 });
caption("Changes always ask first");
await focusAsk();
await press("Escape");
await sleep(400);
await type("Mark order 1042 as shipped");
await waitForText("Haiku 5.5 · first SQL");
await sleep(700);
await press("Enter");
const dialog = await waitFor('[role="dialog"]');
await sleep(300);
cam(dialog, { maxZoom: 1.8, pad: 60 });
await sleep(2300);
await click(await rect('[role="dialog"] button', "Cancel"));
await sleep(500);
cam(null);
await sleep(600);

log("end");

// Off camera: an empty editor and a taller Agent view for the next scene.
await click(await waitFor('button[aria-label="Clear"]'), { glide: 0 });
{
  const bar = await page.evaluate(() => {
    const seps = [...document.querySelectorAll('[role="separator"]')].filter((e) => e.offsetParent);
    const r = seps.sort((a, b) => b.getBoundingClientRect().y - a.getBoundingClientRect().y)[0].getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  });
  await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...bar });
  await page.send("Input.dispatchMouseEvent", { type: "mousePressed", ...bar, button: "left", clickCount: 1 });
  for (let k = 1; k <= 10; k++) {
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: bar.x, y: bar.y - k * 22, button: "left" });
    await sleep(16);
  }
  await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: bar.x, y: bar.y - 220, button: "left", clickCount: 1 });
}
await sleep(400);

// 3. Investigate: a question that needs several steps. In ?video mode Investigate is on for
// the demo store with counts only, so it goes straight to the plan.
scene("agent");
cam(await askBar(), { maxZoom: 1.9 });
caption("Investigate, step by step");
hideCursor();
await focusAsk();
await press("Escape");
await sleep(400);
await type("/inv");
await sleep(800);
await press("Enter");
await sleep(400);
await type("why did paid orders drop last week?");
await sleep(300);
await press("Enter");
await waitForText("Change any line, then press Go");
await sleep(300);
cam(union(await askBar(), await agentArea()), { maxZoom: 1.3 });
await sleep(1600);
await reveal("button", "Go");
await sleep(200);
await click(await waitFor("button", "Go"));
await waitForText("Paid orders fell from 52 to 31", 20000);
await reveal("section", "Answer");
await sleep(300);
cam(await agentArea(), { maxZoom: 1.45 });
caption("Every answer cites the queries behind it");
await sleep(5200);
log("end");

// Off camera: Settings → AI apps, Access tab, scrolled to the connections. With a suggested note
// waiting, the gear's label counts it and it opens on the Suggestions tab, hence the prefix match.
await click(await rect('button[aria-label^="Settings"]'), { glide: 0 });
await click(await waitFor('[role="tab"]', "Access"), { glide: 0 });
await waitForText("What AI apps get back");
await page.evaluate(() => [...document.querySelectorAll("h3")].find((h) => h.textContent === "Connections").scrollIntoView({ block: "start" }));
await sleep(800);

// 4. Guardrails: what AI apps may see, what they ran, and blocking one.
scene("guard");
const section = (title) =>
  page.evaluate((title) => {
    // The heading's section: its nearest ancestor with the list or table in it.
    let el = [...document.querySelectorAll("h3")].find((h) => h.textContent === title).parentElement;
    while (!el.querySelector("ul, table")) el = el.parentElement;
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  }, title);
cam(await section("Connections"), { maxZoom: 1.7, pad: 30 });
caption("You choose what AI apps may see");
await sleep(900);
// A native select's menu doesn't show in a screencast, so the cursor points at it and the
// choices go through one by one, each with its description.
const demoSelect = await waitFor('select[aria-label="What AI apps may see on Demo store (SQLite)"]');
log("cursor", { x: demoSelect.x + demoSelect.w / 2, y: demoSelect.y + demoSelect.h / 2, dur: 0.6 });
await sleep(800);
for (const v of ["off", "masked", "all", "counts"]) {
  await page.evaluate((v) => {
    const s = document.querySelector('select[aria-label="What AI apps may see on Demo store (SQLite)"]');
    s.value = v;
    s.dispatchEvent(new Event("change", { bubbles: true }));
  }, v);
  await sleep(v === "counts" ? 1000 : 850);
}
// The tabs sit above the Connections section, which scrolled them out of view.
await reveal('[role="tab"]', "Activity");
await sleep(300);
await click(await waitFor('[role="tab"]', "Activity"));
await waitForText("never-send column email");
await sleep(400);
cam(await rect('[role="tabpanel"] table'), { maxZoom: 1.6, pad: 30 });
caption("See what each app ran, and what was refused");
await sleep(3000);
await reveal('[role="tab"]', "Access");
await sleep(200);
await click(await waitFor('[role="tab"]', "Access"));
await waitForText("What AI apps get back");
cam(await section("Apps"), { maxZoom: 1.7, pad: 30 });
caption("Don't trust an app? Block it.");
await click(await waitFor("button", "Block Cursor"));
await waitFor("button", "Allow Cursor");
await sleep(1800);
log("end");

// Off camera: close Settings, then "Teach SayQL this database" → Run audit. In ?video mode the demo
// store's knowledge sources are its code folder and its data docs.
await press("Escape");
await waitFor("span", "Teach SayQL this database");
await sleep(300);
await click(await rect("span", "Teach SayQL this database"), { glide: 0 });
await click(await waitFor('[role="dialog"] button', "Run audit"), { glide: 0 });
await waitFor('[role="dialog"] button', "Start audit");
await sleep(800);

// 5. Knowledge: the audit reads the code and docs and suggests notes; then a note whose column was
// renamed shows as doubtful, with SayQL's fix.
scene("knowledge");
cam(await waitFor('[role="dialog"]'), { maxZoom: 1.6, pad: 30 });
caption("Learns from your code and docs");
await sleep(900);
await click(await waitFor('[role="dialog"] button', "Start audit"), { glide: 550 });
hideCursor();
await sleep(150);
cam(await waitFor('[role="dialog"]'), { maxZoom: 1.7, pad: 30 });
await waitForText("Suggested a note: orders.status");
cam(await waitFor('[role="dialog"]'), { maxZoom: 1.7, pad: 30 });
await waitForText("2 notes suggested", 10000);
await sleep(200);
// The log and its result; the summary below it would push the dialog past the caption.
cam(union(await waitFor('[role="log"]'), await textRect("2 notes suggested")), { maxZoom: 1.6, pad: 30 });
await sleep(1600);
log("end");

// Off camera: close the audit; the notes reload with the doubtful one.
await click(await waitFor('[role="dialog"] button[aria-label="Close"]'), { glide: 0 });
await waitForText("may be out of date");
await sleep(600);

scene("notes");
const doubtfulNote = await waitFor("span", "products.price is in cents, not dollars");
cam(union(await textRect("may be out of date: the column"), doubtfulNote, await rect("span", "Notes")), { maxZoom: 1.7, pad: 30 });
caption("Notes stay true as the schema changes");
await sleep(1500);
await click(doubtfulNote, { glide: 600 });
await waitFor('[role="dialog"] button', "Use this text");
await sleep(200);
{
  const d = await waitFor('[role="dialog"]');
  const b = await waitFor('[role="dialog"] button', "Use this text");
  cam({ x: d.x, y: d.y, w: d.w, h: b.y + b.h - d.y + 12 }, { maxZoom: 1.8, pad: 24 });
}
await sleep(2200);
await click(await waitFor('[role="dialog"] button', "Use this text"), { glide: 550 });
// The note, fixed, and no longer doubtful.
await waitFor("span", "products.price_cents is in cents, not dollars");
await sleep(100);
cam(union(await rect("span", "Notes"), await rect("span", "products.price_cents is in cents, not dollars")), { maxZoom: 1.9, pad: 40 });
await sleep(2000);
log("end");

// ── Teams: the SayQL server's admin page ────────────────────────────────────
// SAYQL_ADMIN_LINK is a throwaway server's one-time admin link (see team-server.sh), and
// SAYQL_TEAM_DB the SQLite file it serves as "Shop".
const adminLink = process.env.SAYQL_ADMIN_LINK;
const teamDb = process.env.SAYQL_TEAM_DB;
if (!adminLink || !teamDb) throw new Error("Set SAYQL_ADMIN_LINK and SAYQL_TEAM_DB: see scripts/video/team-server.sh.");
const server = new globalThis.URL(adminLink).origin;
await page.goto(adminLink);
await waitFor("#view-app");
await keepRepainting();
const adminToken = await page.evaluate(() => sessionStorage.getItem("sayql_admin_token"));
const api = async (method, path, body, token = adminToken) => {
  const r = await fetch(server + path, {
    method,
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
    body: body && JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${await r.text()}`);
  return r.json();
};
// The database, with its private columns, and a person's question for the audit log.
const shop = await api("POST", "/api/v1/admin/connections", {
  name: "Shop", engine: "sqlite", path: teamDb, agent_access: "counts", never_send: ["customers.email", "payments.card_last4"], notes: "",
});
const maya = await api("POST", "/api/v1/admin/users", { email: "maya@example.com", name: "Maya", role: "member" });
const mayaToken = (await api("POST", "/api/v1/auth/redeem", { code: maya.invite_code, device: "SayQL app" }, "")).token;
await api("POST", `/api/v1/connections/${shop.id}/query`, {
  sql: "SELECT COUNT(*) AS open_tickets FROM tickets WHERE status = 'open'", source: "ask", question: "How many tickets are open?",
}, mayaToken);
await page.evaluate(() => (location.hash = "agents"));
await waitFor("#agent-add");
await sleep(800);

// 6. Teams: an agent with its own key.
scene("teams");
cam(null);
caption("On a team server, each agent gets its own key");
await sleep(700);
await click(await waitFor("#agent-add"));
const agentDialog = await waitFor("#agent-dialog");
cam(agentDialog, { maxZoom: 1.6, pad: 30 });
await click(await waitFor('#agent-form input[name="name"]'), { glide: 450 });
await type("Support bot");
await sleep(300);
await click(await waitFor("#agent-conns label"), { glide: 500 });
await sleep(400);
await click(await waitFor('#agent-form button[type="submit"]'), { glide: 500 });
await waitFor("#agent-result");
await sleep(300);
{
  // The dialog down to the Claude Code command.
  const d = await waitFor("#agent-dialog");
  const c = await waitFor("#agent-claude");
  cam({ x: d.x, y: d.y, w: d.w, h: c.y + c.h - d.y + 16 }, { maxZoom: 1.7, pad: 30 });
}
caption("Its key, shown once, ready for Claude Code");
await sleep(3200);
const agentKey = await page.evaluate(() => document.getElementById("agent-key").value);
await click(await waitFor("#agent-done"));
await sleep(500);

// 7. Its scope: two tables, and only tenant 42's rows of orders.
scene("scope");
caption("Only the tables and rows you choose");
const scopeButton = await page.evaluate(() => {
  const row = [...document.querySelectorAll("#agent-rows tr")].find((tr) => tr.textContent.startsWith("Support bot"));
  const r = [...row.querySelectorAll("button")].find((b) => b.textContent === "Scope").getBoundingClientRect();
  return { x: r.x, y: r.y, w: r.width, h: r.height };
});
await click(scopeButton);
await waitFor('#scope-conns input[name="all"]');
await sleep(300);
cam(await waitFor("#scope-dialog"), { maxZoom: 1.6, pad: 30 });
const box = (sel, text) =>
  page.evaluate(
    (sel, text) => {
      const el = [...document.querySelectorAll(sel)].find((e) => e.offsetParent && (text === null || e.textContent.trim() === text));
      const r = el.getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height };
    },
    sel,
    text,
  );
await click(await box("#scope-conns label.check", "All tables, including ones added later"), { glide: 500 });
await sleep(300);
await click(await box("#scope-conns .scope-tables label", "customers"), { glide: 450 });
await click(await box("#scope-conns .scope-tables label", "orders"), { glide: 450 });
await sleep(200);
await click(await box("#scope-conns button.scope-add", null), { glide: 500 });
await sleep(300);
cam(await waitFor("#scope-dialog"), { maxZoom: 1.6, pad: 30 });
// The pickers are native selects: the cursor points at each and the value is set.
const pick = async (name, value) => {
  const r = await waitFor(`#scope-conns [name="${name}"]`);
  log("cursor", { x: r.x + r.w / 2, y: r.y + r.h / 2, dur: 0.45 });
  await sleep(550);
  log("click", { x: r.x + r.w / 2, y: r.y + r.h / 2 });
  await page.evaluate(
    (name, value) => {
      const s = document.querySelector(`#scope-conns [name="${name}"]`);
      s.value = value;
      s.dispatchEvent(new Event("change", { bubbles: true }));
    },
    name,
    value,
  );
  await sleep(400);
};
await pick("f-table", "orders");
await pick("f-column", "tenant_id");
await click(await waitFor('#scope-conns [name="f-value"]'), { glide: 450 });
await type("42");
await sleep(900);
await click(await waitFor('#scope-form button[type="submit"]'), { glide: 500 });
await waitForText("rows filtered");
await sleep(300);
cam(await waitFor("#tab-agents table"), { maxZoom: 1.6, pad: 40 });
await sleep(1600);

// Off camera: the agent at work over MCP, as Claude Code would call it.
let rpc = 0;
const mcp = (method, params) =>
  fetch(server + "/mcp", {
    method: "POST",
    headers: { Authorization: `Bearer ${agentKey}`, "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: ++rpc, method, params }),
  }).then((r) => r.text());
const call = (sql) => mcp("tools/call", { name: "run_query", arguments: { connection_id: shop.id, sql } });
await mcp("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "claude-code", version: "2.0" } });
await call("SELECT COUNT(*) AS refunded, ROUND(AVG(total), 2) AS avg_total FROM orders WHERE status = 'refunded'");
await call("SELECT c.email FROM customers c JOIN orders o ON o.customer_id = c.id WHERE o.status = 'refunded'");
await call("SELECT COUNT(*) AS customers FROM customers WHERE plan = 'pro'");

// 8. The audit log: people and agents, refusals too.
scene("audit");
caption("Every query, from people and agents, on record");
await click(await box(".tab", "Audit log"));
await waitForText("agent: Support bot");
await sleep(400);
cam(await waitFor("#tab-audit table"), { maxZoom: 1.5, pad: 30 });
await sleep(3800);
log("end");

await sleep(300);
clearInterval(progress);
clearInterval(watchdog);
await page.send("Page.stopScreencast");
await sleep(500);
writeFileSync(join(out, "timeline.json"), JSON.stringify({ theme, view: VIEW, frames, events }, null, 1));
console.log(`${theme}: ${frames.length} frames, ${(frames.at(-1).t - frames[0].t).toFixed(1)} s`);
await page.close();
