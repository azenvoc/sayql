// Takes the README's app screenshots from the UI preview (vite + src/dev/mockBackend.ts in ?video mode).
//
//   pnpm exec vite --port 1431 --strictPort        # in another terminal
//   node scripts/screenshots/readme.mjs
//
// Writes .github/assets/{hero,feature-access,feature-notes,feature-ask,feature-agent}-{light,dark}.png
// at 2×, on a transparent background: the hero (Settings → AI apps → Activity, as on sayql.dev) and
// the notes as windows with a title bar, the others as cropped cards.
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "../video/cdp.mjs";

const URL = process.env.SAYQL_URL ?? "http://localhost:1431/?video";
const VIEW = { width: 1200, height: 780, scale: 2 };
const OUT = join(import.meta.dirname, "..", "..", ".github", "assets");

for (const theme of ["light", "dark"]) {
  const page = await launch(VIEW);
  await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });

  const rect = (sel, text = null) =>
    page.evaluate(
      (sel, text) => {
        const el = [...document.querySelectorAll(sel)].find((e) => {
          const r = e.getBoundingClientRect();
          return r.width > 0 && r.height > 0 && (text === null || e.textContent.trim() === text || e.getAttribute("aria-label") === text);
        });
        const r = el?.getBoundingClientRect();
        return r ? { x: r.x, y: r.y, w: r.width, h: r.height } : null;
      },
      sel,
      text,
    );
  const waitFor = async (sel, text = null) => {
    for (const end = Date.now() + 10000; Date.now() < end; await sleep(50)) {
      const r = await rect(sel, text);
      if (r) return r;
    }
    throw new Error(`Timed out waiting for ${sel} ${text ?? ""}`);
  };
  const waitForText = async (text, timeout = 15000) => {
    for (const end = Date.now() + timeout; Date.now() < end; await sleep(50))
      if (await page.evaluate((t) => document.body.innerText.includes(t), text)) return;
    throw new Error(`Timed out waiting for "${text}"`);
  };
  const click = async (r, modifiers = 0) => {
    const at = { x: r.x + r.w / 2, y: r.y + r.h / 2 };
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...at, modifiers });
    await page.send("Input.dispatchMouseEvent", { type: "mousePressed", ...at, button: "left", clickCount: 1, modifiers });
    await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...at, button: "left", clickCount: 1, modifiers });
    await sleep(250);
  };
  const enter = async () => {
    const k = { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 };
    await page.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...k });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...k });
  };
  const type = async (text) => {
    for (const ch of text) {
      await page.send("Input.insertText", { text: ch });
      await sleep(30);
    }
  };
  const focusAsk = () => page.evaluate(() => document.querySelector('textarea[aria-label="Ask in plain English"]').focus());
  const clearAsk = () =>
    page.evaluate(() => {
      const el = document.querySelector('textarea[aria-label="Ask in plain English"]');
      el.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true }));
      return true;
    });
  /** The page (or a CSS-px rect of it) as a 2× PNG. */
  const shot = async (clip) => {
    const r = await page.send("Page.captureScreenshot", {
      format: "png",
      ...(clip && { clip: { x: clip.x, y: clip.y, width: clip.w, height: clip.h, scale: 1 } }),
    });
    return r.data;
  };

  await page.goto(URL);
  await page.evaluate(() => {
    localStorage.clear();
    localStorage.setItem("split:editor", "300");
    return true;
  });
  await page.goto(URL);
  await page.evaluate(() => document.fonts.ready.then(() => true));
  await sleep(2500);

  // Nothing connects on its own (0.4.8): connect the demo store first.
  await click(await waitFor('[role="treeitem"] span', "Demo store (SQLite)"));
  await sleep(800);

  // Hero and the Ask box card: two tables picked, a question in Quick mode, the result.
  await click(await waitFor('[role="treeitem"] span', "customers"), 4);
  await click(await waitFor('[role="treeitem"] span', "orders"), 4);
  await focusAsk();
  await type("Top 10 customers by total spend");
  await waitForText("first SQL");
  await sleep(500);
  await enter();
  await waitForText("Kenji Watanabe");
  await sleep(800);
  const askBox = await waitFor('textarea[aria-label="Ask in plain English"]');
  const askCard = await shot({ x: 0, y: askBox.y - 17, w: VIEW.width, h: 262 });

  // The agent card: Investigate, Auto, a finished investigation, in a taller window so the
  // whole answer fits.
  await page.send("Emulation.setDeviceMetricsOverride", { width: VIEW.width, height: 1000, deviceScaleFactor: 2, mobile: false });
  await sleep(500);
  await clearAsk();
  await sleep(300);
  await click(await waitFor('[role="radio"]', "Investigate"));
  await click(await waitFor('[role="switch"]'));
  await focusAsk();
  await type("why did paid orders drop last week?");
  await enter();
  await waitForText("Paid orders fell from 52 to 31", 30000);
  await sleep(800);
  await page.evaluate(() => {
    document.querySelector("#root")?.querySelectorAll("*").forEach((e) => e.scrollTop > 0 && (e.scrollTop = 0));
    return true;
  });
  await page.evaluate(() => (document.activeElement?.blur(), true));
  await sleep(200);
  const ask2 = await waitFor('textarea[aria-label="Ask in plain English"]');
  const tab = await waitFor('[role="tab"]', "Investigation");
  const navRight = (await waitFor('[role="tablist"]')).x;
  const agentClip = { x: navRight, y: ask2.y - 14, w: VIEW.width - navRight, h: VIEW.height - (ask2.y - 14) - 28 };
  // Skip the editor between the Ask box and the Agent view: two strips stacked.
  const top = await shot({ x: agentClip.x, y: agentClip.y, w: agentClip.w, h: 64 });
  const answer = await waitFor('section[aria-label="Answer"]');
  const bottom = await shot({ x: agentClip.x, y: tab.y - 6, w: agentClip.w, h: answer.y + answer.h + 14 - (tab.y - 6) });

  const size = (height) =>
    page.send("Emulation.setDeviceMetricsOverride", { width: VIEW.width, height, deviceScaleFactor: 2, mobile: false });
  const scrollTo = (title) =>
    page.evaluate((title) => {
      [...document.querySelectorAll("h3")].find((h) => h.textContent === title).scrollIntoView({ block: "start" });
      return true;
    }, title);
  // A section of Settings: the heading's nearest ancestor with its list or table in it.
  const section = (title) =>
    page.evaluate((title) => {
      let el = [...document.querySelectorAll("h3")].find((h) => h.textContent === title).parentElement;
      while (!el.querySelector("ul, table")) el = el.parentElement;
      const r = el.getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height };
    }, title);

  // The hero, as on sayql.dev: Settings → AI apps → Activity, what each app ran and what was
  // refused, with Cursor's refused change opened.
  await size(540);
  await click(await waitFor('button[aria-label^="Settings"]'));
  await click(await waitFor('[role="tab"]', "Activity"));
  await waitForText("never-send column email");
  await click(await waitFor("td", "Cursor"));
  await waitForText("App could see");
  await sleep(500);
  const governance = await shot();

  // The access card, two strips stacked: blocking an app, then the demo store's row of the
  // connections table, what AI apps get back and the columns they never see.
  await size(1400);
  await click(await waitFor('[role="tab"]', "Access"));
  await waitForText("What AI apps get back");
  await scrollTo("Apps");
  await sleep(400);
  const apps = await section("Apps");
  const appsStrip = await shot({ x: apps.x - 16, y: apps.y - 12, w: apps.w + 32, h: apps.h + 24 });
  const demoRow = await page.evaluate(() => {
    const table = [...document.querySelectorAll("table")].find((t) => t.innerText.includes("What AI apps get back"));
    const head = table.querySelector("thead").getBoundingClientRect();
    const row = [...table.querySelectorAll("tbody tr")].find((r) => r.innerText.includes("Demo store (SQLite)")).getBoundingClientRect();
    return { x: head.x, y: head.y, w: head.width, h: row.bottom - head.y };
  });
  // Ends at the row's own bottom edge, so none of the next row shows.
  const rowStrip = await shot({ x: apps.x - 16, y: demoRow.y - 12, w: apps.w + 32, h: demoRow.h + 12 });

  // The notes card: an audit suggests notes, then a note whose column was renamed is doubtful,
  // with SayQL's fix.
  await size(VIEW.height);
  await page.evaluate(() => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true }));
    return true;
  });
  await sleep(500);
  await click(await waitFor("span", "Teach SayQL this database"));
  await sleep(400);
  await click(await waitFor('[role="dialog"] button', "Run audit"));
  // The sources load before Start audit does anything.
  await waitFor('[role="dialog"] button', "Start audit");
  await sleep(800);
  await click(await waitFor('[role="dialog"] button', "Start audit"));
  await waitForText("2 notes suggested", 30000);
  await click(await waitFor('[role="dialog"] button[aria-label="Close"]'));
  await waitForText("may be out of date");
  await sleep(400);
  await click(await waitFor("span", "products.price is in cents, not dollars"));
  await waitFor('[role="dialog"] button', "Use this text");
  await sleep(500);
  const notes = await shot();

  // Frame them on a transparent page.
  const dark = theme === "dark";
  const css = `
    body { margin: 0; background: transparent; font: 13px -apple-system, BlinkMacSystemFont, sans-serif; }
    .wrap { display: inline-block; padding: 32px 36px 44px; }
    .win, .card { border-radius: 12px; overflow: hidden; border: 1px solid ${dark ? "#3a3f4b" : "#d6d9de"};
      box-shadow: 0 18px 40px ${dark ? "rgba(23,23,46,.55)" : "rgba(0,42,42,.16)"}, 0 2px 6px rgba(0,0,0,.08); }
    .bar { height: 26px; display: flex; align-items: center; justify-content: center; position: relative;
      background: ${dark ? "#2a2d33" : "#e8eaee"}; color: ${dark ? "#c9ccd3" : "#3d4148"}; font-weight: 600; font-size: 12.5px; }
    .dots { position: absolute; left: 12px; display: flex; gap: 8px; }
    .dots i { width: 12px; height: 12px; border-radius: 50%; display: block; }
    img { display: block; }
    .gap { height: 1px; background: ${dark ? "#3a3f4b" : "#d6d9de"}; }`;
  const frame = async (name, body, width) => {
    await page.send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
    await page.evaluate((html) => {
      document.open();
      document.write(html);
      document.close();
      return true;
    }, `<style>${css}</style><div class="wrap" style="width:${width}px">${body}</div>`);
    await page.evaluate(() => Promise.all([...document.images].map((i) => i.decode())).then(() => true));
    const r = await rect(".wrap");
    writeFileSync(join(OUT, `${name}-${theme}.png`), Buffer.from(await shot(r), "base64"));
    console.log(`wrote ${name}-${theme}.png`);
  };
  const img = (data, w) => `<img src="data:image/png;base64,${data}" width="${w}">`;
  await page.send("Emulation.setDeviceMetricsOverride", { width: VIEW.width + 120, height: VIEW.height + 160, deviceScaleFactor: 2, mobile: false });
  const win = (data) =>
    `<div class="win"><div class="bar"><span class="dots"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i></span>SayQL</div>${img(data, VIEW.width)}</div>`;
  await frame("hero", win(governance), VIEW.width + 2);
  await frame("feature-access", `<div class="card">${img(appsStrip, apps.w + 32)}<div class="gap"></div>${img(rowStrip, apps.w + 32)}</div>`, apps.w + 34);
  await frame("feature-notes", win(notes), VIEW.width + 2);
  await frame("feature-ask", `<div class="card">${img(askCard, VIEW.width)}</div>`, VIEW.width + 2);
  await frame("feature-agent", `<div class="card">${img(top, agentClip.w)}<div class="gap"></div>${img(bottom, agentClip.w)}</div>`, agentClip.w + 2);
  await page.close();
}
