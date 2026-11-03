import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { Schema } from "../../api";
import { VoiceButton, VoicePanel, VoiceStrip } from "./VoiceControls";
import type { VoiceModel } from "./voiceModel";
import {
  NOTHING_HEARD,
  voicePrompt,
  voiceReducer,
  voiceTakesEscape,
  withTranscript,
  type VoicePhase,
} from "./voicePhase";

const idle: VoicePhase = { kind: "idle" };
const model: VoiceModel = {
  known: true,
  ready: false,
  downloading: false,
  received: 0,
  total: 0,
  bytes: 81_781_811,
  error: null,
};

describe("voiceReducer", () => {
  it("offers the download the first time", () => {
    expect(voiceReducer(idle, { type: "press", ready: false })).toEqual({ kind: "offer" });
  });

  it("records, then transcribes on the second press", () => {
    let p = voiceReducer(idle, { type: "press", ready: true });
    expect(p.kind).toBe("starting");
    p = voiceReducer(p, { type: "started" });
    expect(p).toEqual({ kind: "recording", level: 0 });
    p = voiceReducer(p, { type: "level", level: 0.2 });
    expect(p).toEqual({ kind: "recording", level: 0.2 });
    p = voiceReducer(p, { type: "press", ready: true });
    expect(p.kind).toBe("transcribing");
    // Pressing again while transcribing does nothing.
    expect(voiceReducer(p, { type: "press", ready: true })).toBe(p);
    expect(voiceReducer(p, { type: "transcribed" })).toEqual(idle);
  });

  it("esc cancels a recording, and voice only takes esc while it shows something", () => {
    const recording = voiceReducer(voiceReducer(idle, { type: "press", ready: true }), {
      type: "started",
    });
    expect(voiceTakesEscape(recording)).toBe(true);
    expect(voiceReducer(recording, { type: "cancel" })).toEqual(idle);
    expect(voiceTakesEscape(idle)).toBe(false);
  });

  it("a canceled start stays canceled", () => {
    const canceled = voiceReducer(voiceReducer(idle, { type: "press", ready: true }), {
      type: "cancel",
    });
    expect(voiceReducer(canceled, { type: "started" })).toEqual(idle);
    expect(voiceReducer(idle, { type: "level", level: 0.5 })).toEqual(idle);
  });

  it("shows errors as a message, and says when the download finished", () => {
    expect(
      voiceReducer({ kind: "starting" }, { type: "failed", message: "No microphone found." }),
    ).toEqual({
      kind: "message",
      text: "No microphone found.",
      tone: "error",
    });
    expect(voiceReducer({ kind: "offer" }, { type: "downloaded" })).toMatchObject({
      kind: "message",
      tone: "info",
    });
    expect(voiceReducer(idle, { type: "downloaded" })).toEqual(idle);
  });
});

describe("withTranscript", () => {
  it("fills an empty question", () => {
    expect(withTranscript("", " top ten customers ")).toBe("top ten customers");
  });
  it("adds after typed text", () => {
    expect(withTranscript("orders ", "from last week")).toBe("orders from last week");
  });
  it("keeps the question when nothing was heard", () => {
    expect(withTranscript("orders", "  ")).toBe("orders");
  });
});

describe("voicePrompt", () => {
  const col = (name: string) => ({
    name,
    data_type: "text",
    nullable: true,
    primary_key: false,
    enum_values: null,
  });
  const table = (name: string, cols: string[]) => ({
    schema: null,
    name,
    kind: "table" as const,
    columns: cols.map(col),
    foreign_keys: [],
    row_estimate: 1,
  });
  const schema = {
    dialect: "SQLite",
    tables: [table("customers", ["id", "email"]), table("order_items", ["id", "unit_price"])],
  } as Schema;

  it("lists tables, then columns, without repeats", () => {
    expect(voicePrompt(schema, [])).toBe("customers, order_items, id, email, unit_price");
  });
  it("puts picked tables' columns first", () => {
    expect(voicePrompt(schema, ["order_items"])).toBe(
      "customers, order_items, id, unit_price, email",
    );
  });
  it("stays under the length limit", () => {
    expect(voicePrompt(schema, [], 20)).toBe("customers");
    expect(voicePrompt(null, [])).toBe("");
  });
});

describe("Ask box voice states", () => {
  const html = (el: React.ReactElement) => renderToStaticMarkup(el);

  it("the mic says how to start, and turns into stop while recording", () => {
    expect(html(<VoiceButton phase={idle} onToggle={() => {}} />)).toContain(
      'aria-label="Say your question (⌘⇧M)"',
    );
    const on = html(<VoiceButton phase={{ kind: "recording", level: 0.1 }} onToggle={() => {}} />);
    expect(on).toContain('aria-label="Stop (⌘⇧M)"');
    expect(on).toContain('aria-pressed="true"');
    expect(html(<VoiceButton phase={{ kind: "transcribing" }} onToggle={() => {}} />)).toContain(
      "disabled",
    );
  });

  it("recording shows Listening, a meter and esc to cancel", () => {
    const strip = html(<VoiceStrip phase={{ kind: "recording", level: 0.1 }} />);
    expect(strip).toContain("Listening…");
    expect(strip).toContain("cancel");
    expect(strip.match(/scaleY/g)).toHaveLength(5);
    expect(html(<VoiceStrip phase={{ kind: "transcribing" }} />)).toContain("Transcribing…");
    expect(html(<VoiceStrip phase={idle} />)).toBe("");
  });

  const panel = (phase: VoicePhase, m: Partial<VoiceModel> = {}) =>
    html(
      <VoicePanel
        phase={phase}
        model={{ ...model, ...m }}
        onDownload={() => {}}
        onCancelDownload={() => {}}
        onClose={() => {}}
      />,
    );

  it("first use offers the one-time download", () => {
    const offer = panel({ kind: "offer" });
    expect(offer).toContain("Voice needs a one-time 82 MB download. It runs on this computer.");
    expect(offer).toContain("Download");
    expect(offer).toContain("Cancel");
  });

  it("shows download progress", () => {
    const p = panel(
      { kind: "offer" },
      { downloading: true, received: 41_000_000, total: 81_781_811 },
    );
    expect(p).toContain("41 MB of 82 MB");
    expect(p).toContain('aria-valuenow="50"');
  });

  it("a failed download can be retried", () => {
    const p = panel(
      { kind: "offer" },
      { error: "The download failed. Check your connection and try again." },
    );
    expect(p).toContain("The download failed.");
    expect(p).toContain("Try again");
  });

  it("errors show as a short message", () => {
    expect(panel({ kind: "message", text: NOTHING_HEARD, tone: "error" })).toContain(
      'role="alert"',
    );
    expect(panel(idle)).toBe("");
  });
});
