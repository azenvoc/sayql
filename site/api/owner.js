// The owner portal's API (sayql.dev/owner): one person, one password. Vercel function.
//
// Sign-in: the password is checked against OWNER_PASSWORD_HASH (scrypt, set by the owner; the
// password itself is never stored anywhere). Ten wrong passwords in 15 minutes lock sign-in for the
// rest of that window. A session is an HMAC-signed cookie (OWNER_SESSION_SECRET), HttpOnly, Secure,
// SameSite=Strict, scoped to /api/owner, for 12 hours. Every POST also needs the X-Requested-With
// header, which a cross-site form can't send.
//
// Data comes from the Supabase project `sayql` (waitlist, trials, licences, install_runs) and from
// GitHub's public release download counts. SayQL's apps and servers report nothing here: free use
// isn't tracked, by design.

import crypto from "node:crypto";
import { headers, rpc, store } from "./_lib/supabase.js";

const COOKIE = "sayql_owner";
const SESSION_SECONDS = 12 * 60 * 60;
const LOCKOUT_FAILURES = 10;
const LOCKOUT_MINUTES = 15;
// The public half of the licence signing key, as in the server repo's keys/licence-public.key.
const LICENCE_PUBLIC_KEY = "t6qg1aROiCORksK7tsCHbY4sDpFk/AwQn8tO0wYxSx4=";
const RELEASES = "https://api.github.com/repos/azenvoc/sayql-releases/releases?per_page=50";

export default async function handler(req, res) {
  res.setHeader("Cache-Control", "no-store");
  const action = String(req.query.action || "");
  const db = store("OWNER");
  if (!db || !process.env.OWNER_PASSWORD_HASH || !process.env.OWNER_SESSION_SECRET) {
    return res.status(503).json({ error: "The owner portal isn't set up yet." });
  }
  if (req.method === "POST" && req.headers["x-requested-with"] !== "sayql-owner") {
    return res.status(403).json({ error: "Missing request header." });
  }
  try {
    if (action === "login" && req.method === "POST") return await login(req, res, db);
    if (action === "logout" && req.method === "POST") return logout(res);
    if (!signedIn(req)) return res.status(401).json({ error: "Sign in first." });
    if (action === "data" && req.method === "GET") return res.status(200).json(await data(db));
    if (action === "licence" && req.method === "POST") return await recordLicence(req, res, db);
    if (action === "ai-account" && req.method === "POST") return await createAiAccount(req, res, db);
    if (action === "ai-topup" && req.method === "POST") return await topUpAi(req, res, db);
    if (action === "ai-active" && req.method === "POST") return await setAiActive(req, res, db);
    if (action === "ai-rekey" && req.method === "POST") return await rekeyAi(req, res, db);
    return res.status(404).json({ error: "No such action." });
  } catch (e) {
    console.error("owner:", action, e.message);
    return res.status(502).json({ error: "Something went wrong. Try again in a minute." });
  }
}

// ---- sign-in ----

async function login(req, res, db) {
  const since = new Date(Date.now() - LOCKOUT_MINUTES * 60 * 1000).toISOString();
  const failures = await count(db, `owner_login_attempts?ok=is.false&at=gte.${encodeURIComponent(since)}`);
  if (failures >= LOCKOUT_FAILURES) {
    return res.status(429).json({ error: `Too many wrong passwords. Try again in ${LOCKOUT_MINUTES} minutes.` });
  }
  const password = String(body(req).password || "");
  const ok = password.length > 0 && checkPassword(password, process.env.OWNER_PASSWORD_HASH);
  await insert(db, "owner_login_attempts", { ok });
  if (!ok) return res.status(401).json({ error: "That password isn't right." });
  const exp = Math.floor(Date.now() / 1000) + SESSION_SECONDS;
  const value = `v1.${exp}`;
  res.setHeader("Set-Cookie", cookie(`${value}.${sign(value)}`, SESSION_SECONDS));
  return res.status(200).json({ ok: true });
}

function logout(res) {
  res.setHeader("Set-Cookie", cookie("", 0));
  return res.status(200).json({ ok: true });
}

function cookie(value, maxAge) {
  return `${COOKIE}=${value}; Path=/api/owner; Max-Age=${maxAge}; HttpOnly; Secure; SameSite=Strict`;
}

function sign(value) {
  return crypto.createHmac("sha256", process.env.OWNER_SESSION_SECRET).update(value).digest("base64url");
}

function signedIn(req) {
  const raw = String(req.headers.cookie || "")
    .split(/;\s*/)
    .find((c) => c.startsWith(`${COOKIE}=`));
  if (!raw) return false;
  const token = raw.slice(COOKIE.length + 1);
  const cut = token.lastIndexOf(".");
  if (cut < 0) return false;
  const value = token.slice(0, cut);
  const mac = Buffer.from(token.slice(cut + 1));
  const want = Buffer.from(sign(value));
  if (mac.length !== want.length || !crypto.timingSafeEqual(mac, want)) return false;
  const [v, exp] = value.split(".");
  return v === "v1" && Number(exp) > Date.now() / 1000;
}

/** `scrypt$N$r$p$<salt base64>$<hash base64>`, as made by the owner's set-password script. */
function checkPassword(password, stored) {
  const [scheme, n, r, p, salt, hash] = String(stored).split("$");
  if (scheme !== "scrypt" || !salt || !hash) return false;
  const want = Buffer.from(hash, "base64");
  const got = crypto.scryptSync(password, Buffer.from(salt, "base64"), want.length, {
    N: Number(n),
    r: Number(r),
    p: Number(p),
    maxmem: 256 * 1024 * 1024,
  });
  return got.length === want.length && crypto.timingSafeEqual(got, want);
}

// ---- data ----

async function data(db) {
  const since = new Date(Date.now() - 90 * 24 * 60 * 60 * 1000).toISOString().slice(0, 10);
  const month = new Date().toISOString().slice(0, 7) + "-01T00:00:00Z";
  const [waitlist, trials, licences, installs, downloads, aiAccounts, aiMonth] = await Promise.all([
    select(db, "waitlist?select=email,interest,size,created_at&order=created_at.desc"),
    select(db, "trials?select=company,email,size,server_version,created_at&order=created_at.desc"),
    select(
      db,
      "licences?select=licensee,contact_email,plan,users,issued,expires,price_usd_month,notes,created_at&order=expires.asc",
    ),
    select(db, `install_runs?select=day,runs&day=gte.${since}&order=day.asc`),
    downloadCounts(),
    select(db, "ai_accounts?select=id,name,contact_email,balance_micro,active,created_at&order=created_at.desc"),
    select(
      db,
      `ai_ledger?select=account_id,kind,amount_micro,cost_micro,input_tokens,cache_write_tokens,cache_read_tokens,output_tokens,at&at=gte.${encodeURIComponent(month)}&order=at.desc&limit=5000`,
    ),
  ]);
  // Per account this month: requests, tokens, what customers paid and what Anthropic cost us.
  const byAccount = {};
  for (const r of aiMonth) {
    const a = (byAccount[r.account_id] ||= { requests: 0, tokens: 0, charged_micro: 0, cost_micro: 0, topups_micro: 0, last_used: null });
    if (r.kind === "usage") {
      a.requests += 1;
      a.tokens += (r.input_tokens || 0) + (r.cache_write_tokens || 0) + (r.cache_read_tokens || 0) + (r.output_tokens || 0);
      a.charged_micro += -r.amount_micro;
      a.cost_micro += r.cost_micro || 0;
      a.last_used ||= r.at;
    } else if (r.kind === "topup") a.topups_micro += r.amount_micro;
  }
  const ai = aiAccounts.map((a) => ({ ...a, month: byAccount[a.id] || null }));
  return { waitlist, trials, licences, installs, downloads, ai, generated_at: new Date().toISOString() };
}

/** App downloads per release and platform, from GitHub's public counts (update checks excluded). */
async function downloadCounts() {
  try {
    const r = await fetch(RELEASES, {
      headers: { Accept: "application/vnd.github+json", "User-Agent": "sayql-owner-portal" },
      signal: AbortSignal.timeout(5000),
    });
    if (!r.ok) return { error: `GitHub answered ${r.status}` };
    const releases = await r.json();
    return releases
      .filter((rel) => !rel.draft)
      .map((rel) => {
        const by = { macOS: 0, Windows: 0, Linux: 0 };
        for (const a of rel.assets || []) {
          const platform = platformOf(a.name);
          if (platform) by[platform] += a.download_count;
        }
        return { tag: rel.tag_name, published_at: rel.published_at, ...by };
      });
  } catch (e) {
    return { error: e.message };
  }
}

function platformOf(name) {
  // Installers people download; updater archives and signatures are skipped.
  if (/\.dmg$/i.test(name)) return "macOS";
  if (/\.(msi|exe)$/i.test(name)) return "Windows";
  if (/\.(AppImage|deb|rpm)$/i.test(name)) return "Linux";
  return null;
}

// ---- licences ----

async function recordLicence(req, res, db) {
  const b = body(req);
  const key = String(b.key || "").replace(/\s+/g, "");
  const licence = verifyLicence(key);
  if (!licence) return res.status(400).json({ error: "That isn't a licence key signed with SayQL's key." });
  const email = String(b.contact_email || "").trim().toLowerCase();
  const price = b.price_usd_month === "" || b.price_usd_month == null ? null : Number(b.price_usd_month);
  if (price != null && !(price >= 0 && price < 1e6)) return res.status(400).json({ error: "The price must be a number." });
  const row = {
    licensee: licence.licensee,
    contact_email: email || null,
    plan: licence.plan,
    users: licence.users,
    issued: licence.issued,
    expires: licence.expires,
    price_usd_month: price,
    key_sha256: crypto.createHash("sha256").update(key).digest("hex"),
    notes: String(b.notes || "").trim().slice(0, 2000) || null,
  };
  const r = await fetch(`${db.url}/rest/v1/licences`, {
    method: "POST",
    headers: headers(db, { "Content-Type": "application/json", Prefer: "return=minimal" }),
    body: JSON.stringify(row),
  });
  if (r.status === 409) return res.status(409).json({ error: "That key is already recorded." });
  if (!r.ok) throw new Error(`licences answered ${r.status}`);
  return res.status(200).json({ ok: true, licence: row });
}

/** The licence payload if `key` is signed by SayQL's licence key, else null. */
function verifyLicence(key) {
  const [payload, signature] = key.split(".");
  if (!payload || !signature) return null;
  try {
    const publicKey = crypto.createPublicKey({
      key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(LICENCE_PUBLIC_KEY, "base64")]),
      format: "der",
      type: "spki",
    });
    const bytes = Buffer.from(payload, "base64url");
    if (!crypto.verify(null, bytes, publicKey, Buffer.from(signature, "base64url"))) return null;
    const l = JSON.parse(bytes.toString("utf8"));
    return l && l.v === 1 && ["team", "trial"].includes(l.plan) ? l : null;
  } catch {
    return null;
  }
}

// ---- AI credits ----

/** A new credit account. Its key is returned this once; only a SHA-256 of it is kept. */
async function createAiAccount(req, res, db) {
  const b = body(req);
  const name = String(b.name || "").trim().slice(0, 200);
  if (!name) return res.status(400).json({ error: "Name the team or company." });
  const email = String(b.contact_email || "").trim().toLowerCase().slice(0, 320) || null;
  const key = "sqlc_" + crypto.randomBytes(24).toString("base64url");
  const r = await fetch(`${db.url}/rest/v1/ai_accounts`, {
    method: "POST",
    headers: headers(db, { "Content-Type": "application/json", Prefer: "return=representation" }),
    body: JSON.stringify({ name, contact_email: email, key_sha256: crypto.createHash("sha256").update(key).digest("hex") }),
  });
  if (!r.ok) throw new Error(`ai_accounts answered ${r.status}`);
  const [account] = await r.json();
  const dollars = Number(b.initial_usd || 0);
  if (dollars > 0) await rpc(db, "ai_topup", { p_account: account.id, p_amount: Math.round(dollars * 1e6), p_kind: "topup", p_note: "first top-up" });
  return res.status(200).json({ ok: true, id: account.id, key });
}

/** A top-up in USD (or, as an adjustment, a correction that may be negative). */
async function topUpAi(req, res, db) {
  const b = body(req);
  const id = Number(b.account_id);
  const dollars = Number(b.usd);
  const kind = b.kind === "adjust" ? "adjust" : "topup";
  if (!Number.isInteger(id)) return res.status(400).json({ error: "Pick an account." });
  if (!Number.isFinite(dollars) || dollars === 0 || Math.abs(dollars) > 100_000 || (kind === "topup" && dollars < 0)) {
    return res.status(400).json({ error: "Enter an amount in USD." });
  }
  const balance = await rpc(db, "ai_topup", {
    p_account: id,
    p_amount: Math.round(dollars * 1e6),
    p_kind: kind,
    p_note: String(b.note || "").trim().slice(0, 500) || null,
  });
  return res.status(200).json({ ok: true, balance_micro: balance });
}

/**
 * A new key for an account whose key was lost or leaked: the old key stops working at once; the
 * balance, history and on/off state stay. The new key is returned this once; only its SHA-256 is kept.
 */
async function rekeyAi(req, res, db) {
  const id = Number(body(req).account_id);
  if (!Number.isInteger(id)) return res.status(400).json({ error: "Pick an account." });
  const key = "sqlc_" + crypto.randomBytes(24).toString("base64url");
  const r = await fetch(`${db.url}/rest/v1/ai_accounts?id=eq.${id}`, {
    method: "PATCH",
    headers: headers(db, { "Content-Type": "application/json", Prefer: "return=representation" }),
    body: JSON.stringify({ key_sha256: crypto.createHash("sha256").update(key).digest("hex") }),
  });
  if (!r.ok) throw new Error(`ai_accounts answered ${r.status}`);
  const rows = await r.json();
  if (!Array.isArray(rows) || !rows.length) return res.status(404).json({ error: "No such account." });
  return res.status(200).json({ ok: true, key });
}

/** Turns an account off (its key stops working at once) or back on. */
async function setAiActive(req, res, db) {
  const b = body(req);
  const id = Number(b.account_id);
  if (!Number.isInteger(id)) return res.status(400).json({ error: "Pick an account." });
  const r = await fetch(`${db.url}/rest/v1/ai_accounts?id=eq.${id}`, {
    method: "PATCH",
    headers: headers(db, { "Content-Type": "application/json", Prefer: "return=minimal" }),
    body: JSON.stringify({ active: !!b.active }),
  });
  if (!r.ok) throw new Error(`ai_accounts answered ${r.status}`);
  return res.status(200).json({ ok: true });
}


// ---- Supabase ----



async function select(db, path) {
  const r = await fetch(`${db.url}/rest/v1/${path}`, { headers: headers(db) });
  if (!r.ok) throw new Error(`${path.split("?")[0]} answered ${r.status}`);
  return r.json();
}

async function count(db, path) {
  const r = await fetch(`${db.url}/rest/v1/${path}&select=id`, {
    method: "HEAD",
    headers: headers(db, { Prefer: "count=exact" }),
  });
  if (!r.ok) throw new Error(`count answered ${r.status}`);
  return Number((r.headers.get("content-range") || "").split("/")[1] || 0);
}

async function insert(db, table, row) {
  const r = await fetch(`${db.url}/rest/v1/${table}`, {
    method: "POST",
    headers: headers(db, { "Content-Type": "application/json", Prefer: "return=minimal" }),
    body: JSON.stringify(row),
  });
  if (!r.ok) throw new Error(`${table} answered ${r.status}`);
}

function body(req) {
  if (typeof req.body === "string") {
    try {
      return JSON.parse(req.body);
    } catch {
      return {};
    }
  }
  return req.body || {};
}
