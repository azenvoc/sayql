// Turns a capture (capture.mjs) into the videos: the app as a window on a brand background,
// eased zoom and pan onto each camera target, a drawn cursor, captions and title/end cards.
//
//   node scripts/video/render.mjs [dark|light] [full|short|poster|stills] [audio|times]
//
// Frames are drawn by compositor.html in headless Chrome and piped to ffmpeg (H.264, 30 fps).
// The music is the track in music.json (downloaded to its `file`), else music.py's original
// one; MUSIC_FILE=path picks another, MUSIC=0 none. The full cut also carries the lines in
// voiceover.json, from media/voiceover/<id>.wav (recorded takes, or Kokoro's from voiceover.py), or
// VO_DIR, or a macOS voice with VOICE set. "audio" redoes only the soundtrack. Cut points and
// the voiceover script go to media/*-cuts.md.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { launch } from "./cdp.mjs";

const theme = process.argv[2] === "light" ? "light" : "dark";
const cut = process.argv[3] ?? "full";
const FFMPEG = process.env.FFMPEG ?? "/opt/homebrew/bin/ffmpeg";
const FPS = 30;
const XFADE = 0.45;
/** The sites on the end card: SITE large, SITE_NOW under it (null hides it). */
const SITE = "sayql.dev";
const SITE_NOW = null;
/** Where the app window sits in the 1920×1080 frame at zoom 1. */
const WIN = { x: 160, y: 46, w: 1600, h: 900 };

const root = join(import.meta.dirname, "..", "..");
const capture = join(import.meta.dirname, "out", theme);
const { view, frames, events } = JSON.parse(readFileSync(join(capture, "timeline.json"), "utf8"));
const S = WIN.w / view.width; // CSS px → stage px

// ── The edit ───────────────────────────────────────────────────────────────
// Clips name capture scenes; `until` ends a clip at a later scene's start (or its "end" mark),
// `trim` drops seconds from its start, `max` caps its length, `speed` > 1 plays it faster.
// Cards are drawn by compositor.html; a card with `join` follows the previous one with no fade
// (the terminal scenes), and `speed` plays its animation faster.
const EDITS = {
  full: [
    { card: "hook", dur: 5.8 },
    { card: "title", dur: 4.8 },
    { card: "agentCounts", dur: 10.2 },
    { card: "agentRefused", dur: 7.5, join: true },
    { scene: "guard", speed: 1.12 },
    { scene: "ask", until: "safety", trim: 1.6, max: 8.9 },
    { scene: "safety", trim: 0.3, max: 6.0 },
    // Investigate: the question and its plan, then the cited answer.
    { scene: "agent", trim: 3.0, max: 5.2 },
    { scene: "agent", trim: 13.1 },
    // Knowledge: the audit suggests notes from the code and docs, then a doubtful note and its fix.
    { scene: "knowledge", trim: 0.3, speed: 1.4 },
    { scene: "notes", trim: 0.3, speed: 1.4 },
    { scene: "teams", until: "scope", speed: 1.5 },
    { scene: "scope", until: "audit", speed: 1.5 },
    { scene: "audit" },
    { card: "end", dur: 5.4 },
  ],
  short: [
    { card: "hook", dur: 3.0 },
    { card: "agentCounts", dur: 5.6, speed: 1.75 },
    { card: "agentRefused", dur: 4.0, speed: 1.75, join: true },
    { card: "endShort", dur: 2.9 },
  ],
};
EDITS.poster = EDITS.stills = EDITS.full;

const sceneStart = (name) => events.find((e) => e.type === "scene" && e.name === name).t;
/** The first scene or "end" mark after `t`. */
const nextMark = (t) => events.find((e) => (e.type === "scene" || e.type === "end") && e.t > t + 1e-6).t;

const items = EDITS[cut].map((it) => {
  if (it.card) return { speed: 1, ...it, kind: "card" };
  const speed = it.speed ?? 1;
  const from = sceneStart(it.scene) + (it.trim ?? 0);
  let to = (it.until ? sceneStart(it.until) : nextMark(sceneStart(it.scene))) - (it.trimEnd ?? 0);
  if (it.max) to = Math.min(to, from + it.max * speed);
  return { ...it, kind: "clip", from, to, speed, dur: (to - from) / speed };
});
// Clips that continue where the previous one stopped play straight on; others cross-fade.
let t = 0;
items.forEach((it, i) => {
  const prev = items[i - 1];
  it.contiguous = (prev?.kind === "clip" && it.kind === "clip" && Math.abs(prev.to - it.from) < 0.05) || (it.join && prev?.kind === "card");
  it.start = i === 0 || it.contiguous ? t : t - XFADE;
  t = it.start + it.dur;
});
const total = t;

// ── State at a source time ─────────────────────────────────────────────────
const ease = (x) => (x <= 0 ? 0 : x >= 1 ? 1 : x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2);
const FULL = { z: 1, cx: 960, cy: 540 };

/** Zoom and centre that frame `rect` (CSS px) above the caption, clamped inside the frame. */
function framing(e) {
  if (!e.rect) return FULL;
  const pad = (e.pad ?? 24) * S;
  const r = { x: WIN.x + e.rect.x * S - pad, y: WIN.y + e.rect.y * S - pad, w: e.rect.w * S + 2 * pad, h: e.rect.h * S + 2 * pad };
  const z = Math.max(1, Math.min(e.maxZoom ?? 2, (1920 * 0.96) / r.w, 900 / r.h));
  // Keep the view inside the window where it's big enough, else centred on it.
  const clamp = (v, half, lo, size) => (size <= 2 * half ? lo + size / 2 : Math.min(Math.max(v, lo + half), lo + size - half));
  // Centre the target in the 0–930 px band above the caption.
  const cx = clamp(r.x + r.w / 2, 960 / z, WIN.x, WIN.w);
  const cy = clamp(r.y + r.h / 2 + (540 - 465) / z, 540 / z, WIN.y, WIN.h);
  return { z, cx, cy };
}

/** Camera at source time `ts`; the clip's first camera move snaps if the clip was cut into. */
function cameraAt(ts, clip) {
  const CAM_S = 0.95;
  let move = { from: FULL, to: FULL, t0: -Infinity };
  const at = (m, t) => {
    const k = ease((t - m.t0) / CAM_S);
    const lz = Math.log(m.from.z) + (Math.log(m.to.z) - Math.log(m.from.z)) * k;
    return { z: Math.exp(lz), cx: m.from.cx + (m.to.cx - m.from.cx) * k, cy: m.from.cy + (m.to.cy - m.from.cy) * k };
  };
  for (const e of events) {
    if (e.t > ts) break;
    if (e.type !== "cam") continue;
    const snap = !clip.contiguous && e.t <= clip.from + 0.15;
    move = snap ? { from: framing(e), to: framing(e), t0: e.t } : { from: at(move, e.t), to: framing(e), t0: e.t };
  }
  return at(move, ts);
}

function cursorAt(ts) {
  let pos = { x: view.width * 0.62, y: view.height * 0.7 };
  let glide = null;
  let visible = false;
  let ripple = null;
  for (const e of events) {
    if (e.t > ts) break;
    if (e.type === "cursor" && e.hidden) visible = false;
    else if (e.type === "cursor") {
      const from = glide ? glideAt(glide, e.t) : pos;
      glide = { from, to: { x: e.x, y: e.y }, t0: e.t, dur: e.dur || 0.001 };
      pos = glide.to;
      visible = true;
    } else if (e.type === "click" && ts - e.t < 0.4) ripple = { x: e.x, y: e.y, p: (ts - e.t) / 0.4 };
  }
  const p = glide ? glideAt(glide, ts) : pos;
  const st = (v) => ({ x: WIN.x + v.x * S, y: WIN.y + v.y * S });
  return { cursor: { ...st(p), visible }, ripple: ripple && { ...st(ripple), p: ripple.p } };
}
function glideAt(g, t) {
  const k = ease((t - g.t0) / g.dur);
  return { x: g.from.x + (g.to.x - g.from.x) * k, y: g.from.y + (g.to.y - g.from.y) * k };
}

/** The caption showing at source time `ts` within `clip`, with fades at its edges. */
function captionAt(ts, clip) {
  const caps = events.filter((e) => e.type === "caption" && e.t >= clip.from - 0.2 && e.t < clip.to);
  const i = caps.findLastIndex((e) => e.t <= ts);
  // A clip trimmed past its scene's first caption still shows it.
  const cur = i >= 0 ? caps[i] : events.findLast((e) => e.type === "caption" && e.t <= ts);
  if (!cur?.text) return null;
  const until = i + 1 < caps.length ? caps[i + 1].t : null;
  const fadeIn = Math.min(1, (ts - Math.max(cur.t, clip.from)) / clip.speed / 0.3);
  const fadeOut = until === null ? 1 : Math.min(1, (until - ts) / clip.speed / 0.2);
  return { text: cur.text, opacity: Math.max(0, Math.min(fadeIn, fadeOut)) };
}

const frameAt = (ts) => {
  let lo = 0;
  let hi = frames.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (frames[mid].t <= ts) lo = mid;
    else hi = mid - 1;
  }
  return pathToFileURL(join(capture, "frames", frames[lo].file)).href;
};

/** Everything compositor.html needs for output time `t`. */
function stateAt(t) {
  const s = { layers: [], cards: [], caption: null };
  for (const it of items) {
    if (t < it.start || t >= it.start + it.dur) continue;
    const local = t - it.start;
    const next = items[items.indexOf(it) + 1];
    const fadesOut = next && !next.contiguous;
    // Text cards fade in over the second half of a cross-fade and out over the first, so two
    // cards' text never overlaps; the app window cross-fades over the whole of it.
    const prev = items[items.indexOf(it) - 1];
    const fadeIn = (x) => (it.kind === "card" && prev?.kind === "card" ? 2 * x - 1 : x);
    const fadeOut = (x) => (it.kind === "card" && next?.kind === "card" ? 2 * x - 1 : x);
    let opacity = 1;
    if (!it.contiguous && it.start > 0) opacity = Math.min(opacity, fadeIn(local / XFADE));
    if (fadesOut) opacity = Math.min(opacity, fadeOut((it.dur - local) / XFADE));
    opacity = ease(Math.max(0, Math.min(1, opacity)));
    if (it.kind === "card") {
      s.cards.push({ kind: it.card, opacity, rise: it.contiguous ? 0 : (1 - ease(Math.min(1, local / 0.8))) * 16, local: local * it.speed });
      continue;
    }
    const ts = it.from + local * it.speed;
    const cap = captionAt(ts, it);
    if (cap) s.caption = { ...cap, opacity: cap.opacity * opacity };
    s.layers.push({ src: frameAt(ts), opacity, cam: cameraAt(ts, it), ...cursorAt(ts) });
  }
  // The incoming layer draws on top during a cross-fade, so the outgoing one stays opaque.
  if (s.layers.length === 2) s.layers[0].opacity = 1;
  return s;
}

// ── Render ─────────────────────────────────────────────────────────────────
const page = await launch({ width: 1920, height: 1080, scale: 1 });
await page.goto(pathToFileURL(join(import.meta.dirname, "compositor.html")).href);
const wordmark = pathToFileURL(join(root, ".github", "assets", `wordmark-${theme}.svg`)).href;
const interLoaded = await page.evaluate(`setup(${JSON.stringify({ theme, win: WIN, wordmark, site: SITE, siteNow: SITE_NOW })})`);
if (!interLoaded) console.warn("Inter didn't load (offline?); captions fall back to the system font.");

const shot = async (t, format = "jpeg") => {
  await page.evaluate(`frame(${JSON.stringify(stateAt(t))})`);
  const r = await page.send("Page.captureScreenshot", { format, quality: format === "jpeg" ? 95 : undefined });
  return Buffer.from(r.data, "base64");
};

const suffix = theme === "light" ? "-light" : "";
const fmt = (s) => `${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, "0")}`;
const run = (cmd, args) => {
  const r = spawnSync(cmd, args, { stdio: ["ignore", "pipe", "inherit"], encoding: "utf8" });
  if (r.status !== 0) throw new Error(`${cmd} failed`);
  return r.stdout;
};
const duration = (file) => Number(run(FFMPEG.replace(/ffmpeg$/, "ffprobe"), ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", file]));

/** Output time of `at` seconds into a card or a capture scene, or null if the edit leaves it out. */
function outputTime(line) {
  if (line.card) {
    const it = items.find((i) => i.card === line.card);
    return it ? it.start + line.at / it.speed : null;
  }
  const t = sceneStart(line.scene) + line.at;
  const it = items.find((i) => i.kind === "clip" && t >= i.from && t < i.to);
  return it ? it.start + (t - it.from) / it.speed : null;
}

/** The voiceover lines in this edit, with their time, the room before the next one, and audio if any. */
function voiceLines(work) {
  const lines = JSON.parse(readFileSync(join(import.meta.dirname, "voiceover.json"), "utf8"))
    .map((l) => ({ ...l, t: outputTime(l) }))
    .filter((l) => l.t !== null);
  lines.forEach((l, i) => (l.room = (lines[i + 1]?.t ?? total) - l.t));
  for (const l of lines) {
    const recorded = join(process.env.VO_DIR ?? join(root, "media", "voiceover"), `${l.id}.wav`);
    if (process.env.VOICE) {
      mkdirSync(join(work, "vo"), { recursive: true });
      l.file = join(work, "vo", `${l.id}.aiff`);
      // `speak` spells a line the way a macOS voice should say it ("Say Q L" for SayQL).
      run("say", ["-v", process.env.VOICE, "-r", process.env.RATE ?? "178", "-o", l.file, l.speak ?? l.text]);
    } else if (existsSync(recorded)) l.file = recorded;
    if (l.file) {
      l.length = duration(l.file);
      if (l.length > l.room) console.warn(`Voiceover "${l.id}" runs ${(l.length - l.room).toFixed(1)} s into the next line.`);
    }
  }
  return lines;
}

/** Muxes `video` with the music (ducked under the voice, if any) into `out`. */
function soundtrack(video, music, voice, out) {
  const args = ["-y", "-loglevel", "error", "-i", video];
  args.push(...(music ? ["-i", music] : ["-f", "lavfi", "-t", String(total), "-i", "anullsrc=r=48000:cl=stereo"]));
  voice.forEach((l) => args.push("-i", l.file));
  // A downloaded track is cut to length with a fade; music.py's already ends.
  const fade = music && !music.endsWith("music.wav") ? `,afade=t=out:st=${(total - 2.5).toFixed(2)}:d=2.5` : "";
  const f = [`[1:a]aresample=48000,volume=${voice.length ? 0.4 : 1}${fade}[m]`];
  if (voice.length) {
    voice.forEach((l, i) => f.push(`[${i + 2}:a]aresample=48000,pan=stereo|c0=c0|c1=c0,adelay=${Math.round(l.t * 1000)}:all=1[v${i}]`));
    f.push(`${voice.map((_, i) => `[v${i}]`).join("")}amix=inputs=${voice.length}:normalize=0:duration=longest,asplit[vk][vm]`);
    f.push(`[m][vk]sidechaincompress=threshold=0.02:ratio=8:attack=15:release=350[md]`);
    f.push(`[md][vm]amix=inputs=2:normalize=0:duration=first,loudnorm=I=-16:TP=-1.5[a]`);
  } else f.push(music ? `[m]loudnorm=I=-21:TP=-3[a]` : `[m]anull[a]`);
  args.push("-filter_complex", f.join(";"), "-map", "0:v", "-map", "[a]", "-t", total.toFixed(3));
  args.push("-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-movflags", "+faststart", out);
  run(FFMPEG, args);
}

if (cut === "stills") {
  // Frames at the given output times, for checking an edit without rendering it:
  //   node scripts/video/render.mjs dark stills 3,12.5,40   → out/dark/stills/<t>.jpg
  mkdirSync(join(capture, "stills"), { recursive: true });
  for (const t of (process.argv[4] ?? "").split(",").filter(Boolean).map(Number))
    writeFileSync(join(capture, "stills", `${t.toFixed(1)}.jpg`), await shot(t));
  for (const it of items) console.log(`${it.start.toFixed(1)}  ${it.card ?? it.scene} (${it.dur.toFixed(1)} s)`);
  console.log(`${total.toFixed(1)} s; stills in ${join(capture, "stills")}`);
  await page.close();
} else if (cut === "poster") {
  // Claude Code's question answered with counts, not rows, with its caption.
  const t = items.find((it) => it.card === "agentCounts").start + 9.2;
  writeFileSync(join(root, "media", `explainer${suffix}-poster.png`), await shot(t, "png"));
  await page.close();
} else {
  const name = cut === "short" ? `explainer${suffix}-15s` : `explainer${suffix}`;
  const work = join(capture, name);
  mkdirSync(work, { recursive: true });
  const video = join(work, "video.mp4");
  if (process.argv[4] !== "audio" || !existsSync(video)) {
    const ff = spawn(
      FFMPEG,
      [
        "-y", "-loglevel", "error",
        "-f", "image2pipe", "-framerate", String(FPS), "-c:v", "mjpeg", "-i", "-",
        "-c:v", "libx264", "-preset", "slow", "-crf", process.env.CRF ?? "22", "-tune", "animation",
        "-pix_fmt", "yuv420p", "-profile:v", "high", "-r", String(FPS), "-g", String(FPS * 2),
        video,
      ],
      { stdio: ["pipe", "inherit", "inherit"] },
    );
    const n = Math.round(total * FPS);
    for (let i = 0; i < n; i++) {
      const buf = await shot(i / FPS);
      if (!ff.stdin.write(buf)) await new Promise((r) => ff.stdin.once("drain", r));
      if (i % 150 === 0) process.stdout.write(`\r${name}: ${((i / n) * 100).toFixed(0)}%`);
    }
    ff.stdin.end();
    await new Promise((r) => ff.on("close", r));
  }
  await page.close();

  // music.py's groove starts with the first scene after the hook and title cards.
  const firstClip = items.find((it) => it.kind === "clip" || !["hook", "title"].includes(it.card));
  const endCard = items.find((it) => it.card === "end" || it.card === "endShort");
  let music = null;
  const picked = JSON.parse(readFileSync(join(import.meta.dirname, "music.json"), "utf8"));
  let credit = "scripts/video/music.py's original track (no samples or licensed music)";
  if (process.env.MUSIC_FILE) {
    music = process.env.MUSIC_FILE;
    credit = music;
  } else if (process.env.MUSIC !== "0" && existsSync(join(root, picked.file))) {
    music = join(root, picked.file);
    credit = `${picked.title} (${picked.page}), ${picked.license}`;
  } else if (process.env.MUSIC !== "0") {
    console.warn(`${picked.file} is missing, so music.py's track is used. Download it from:\n  ${picked.download}`);
    music = join(work, "music.wav");
    run("python3", [join(import.meta.dirname, "music.py"), music, total.toFixed(3), "--intro", firstClip.start.toFixed(2), "--outro", String(total - endCard.start)]);
  }
  // The full cut carries the voiceover when there is one; the short cut is music only.
  const lines = cut === "full" ? voiceLines(work) : [];
  const voiced = lines.filter((l) => l.file);
  soundtrack(video, music, voiced, join(root, "media", `${name}.mp4`));
  console.log(`\r${name}: ${total.toFixed(1)} s${voiced.length ? ", with the voiceover" : ""}`);

  // Where each part starts, for editing music or a voiceover in later, and the voiceover script.
  const rows = items.map((it) => {
    const caps = it.kind === "clip" ? events.filter((e) => e.type === "caption" && e.text && e.t >= it.from - 0.2 && e.t < it.to) : [];
    // A clip cut into after its scene's caption still shows it (see captionAt).
    const carried = it.kind === "clip" && !it.contiguous && !(caps[0]?.t <= it.from) && events.findLast((e) => e.type === "caption" && e.t < it.from - 0.2);
    if (carried?.text) caps.unshift(carried);
    const capLines = caps.map((c) => `    - ${fmt(it.start + Math.max(0, c.t - it.from) / it.speed)} caption: “${c.text}”`);
    return [`- ${fmt(it.start)} ${it.card ? `${it.card} card` : it.scene}${it.contiguous ? " (continues, no cut)" : " (cut)"}`, ...capLines].join("\n");
  });
  const script = lines.length
    ? `\n## Voiceover script\n\nEach line starts at the time shown and should end within its room. To use your own\nrecordings, save them as media/voiceover/<id>.wav and re-run \`node scripts/video/render.mjs ${theme} full audio\`.\n\n| Starts | Room | id | Line |\n|---|---|---|---|\n${lines.map((l) => `| ${fmt(l.t)} | ${l.room.toFixed(1)} s | ${l.id} | ${l.text} |`).join("\n")}\n`
    : "";
  writeFileSync(
    join(root, "media", `${name}-cuts.md`),
    `# ${name}.mp4 — cut points\n\n${total.toFixed(1)} s, ${FPS} fps, 1920×1080. Cross-fades are ${XFADE} s and start at the time shown.\n` +
      `Music: ${music ? credit : "none"}.\n\n${rows.join("\n")}\n${script}`,
  );
}
