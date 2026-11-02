import { Fragment, type ReactNode } from "react";
import styles from "./Markdown.module.css";

/**
 * A small Markdown renderer for text an AI wrote (an audit's closing summary): headings,
 * paragraphs, lists, quotes, rules, fenced code, and inline code, bold, italic and links.
 * It builds React elements and never sets HTML, so nothing in the text can run. Links open
 * only through `onLink` and only for http(s); without it they show as text.
 */
export function Markdown({
  text,
  onLink,
  className,
}: {
  text: string;
  onLink?: (url: string) => void;
  className?: string;
}) {
  return (
    <div className={[styles.md, className].filter(Boolean).join(" ")}>{blocks(text, onLink)}</div>
  );
}

type Block =
  | { kind: "h"; level: number; text: string }
  | { kind: "p"; text: string }
  | { kind: "ul" | "ol"; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "code"; text: string }
  | { kind: "hr" };

/** The blocks of `text`, in order. Exported for tests. */
export function parseBlocks(text: string): Block[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let i = 0;
  const isBullet = (l: string) => /^\s*[-*+]\s+/.test(l);
  const isNumbered = (l: string) => /^\s*\d+[.)]\s+/.test(l);
  const startsBlock = (l: string) =>
    /^\s*(#{1,6}\s|```|>|([-*_])\s*\2\s*\2[\s\-*_]*$)/.test(l) || isBullet(l) || isNumbered(l);
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const fence = line.match(/^\s*```/);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) body.push(lines[i++]);
      i++;
      out.push({ kind: "code", text: body.join("\n") });
      continue;
    }
    const h = line.match(/^\s*(#{1,6})\s+(.*?)\s*#*\s*$/);
    if (h) {
      out.push({ kind: "h", level: h[1].length, text: h[2] });
      i++;
      continue;
    }
    if (/^\s*([-*_])\s*\1\s*\1[\s\-*_]*$/.test(line)) {
      out.push({ kind: "hr" });
      i++;
      continue;
    }
    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i]))
        body.push(lines[i++].replace(/^\s*>\s?/, ""));
      out.push({ kind: "quote", text: body.join(" ") });
      continue;
    }
    if (isBullet(line) || isNumbered(line)) {
      const kind = isBullet(line) ? "ul" : "ol";
      const same = kind === "ul" ? isBullet : isNumbered;
      const items: string[] = [];
      while (i < lines.length && lines[i].trim()) {
        const l = lines[i];
        if (same(l)) items.push(l.replace(/^\s*([-*+]|\d+[.)])\s+/, ""));
        else if (startsBlock(l)) break;
        else items[items.length - 1] += ` ${l.trim()}`;
        i++;
      }
      out.push({ kind, items });
      continue;
    }
    const body: string[] = [];
    while (i < lines.length && lines[i].trim() && !startsBlock(lines[i]))
      body.push(lines[i++].trim());
    out.push({ kind: "p", text: body.join(" ") });
  }
  return out;
}

function blocks(text: string, onLink?: (url: string) => void): ReactNode {
  return parseBlocks(text).map((b, k) => {
    const i = (t: string) => inline(t, onLink);
    switch (b.kind) {
      case "h": {
        const H = (["h3", "h4", "h5", "h6", "h6", "h6"] as const)[b.level - 1];
        return <H key={k}>{i(b.text)}</H>;
      }
      case "p":
        return <p key={k}>{i(b.text)}</p>;
      case "ul":
        return (
          <ul key={k}>
            {b.items.map((t, n) => (
              <li key={n}>{i(t)}</li>
            ))}
          </ul>
        );
      case "ol":
        return (
          <ol key={k}>
            {b.items.map((t, n) => (
              <li key={n}>{i(t)}</li>
            ))}
          </ol>
        );
      case "quote":
        return <blockquote key={k}>{i(b.text)}</blockquote>;
      case "code":
        return (
          <pre key={k}>
            <code>{b.text}</code>
          </pre>
        );
      case "hr":
        return <hr key={k} />;
    }
  });
}

/** Inline code, links, bold and italic. Code is taken first, so nothing inside it is styled. */
function inline(text: string, onLink?: (url: string) => void): ReactNode {
  const out: ReactNode[] = [];
  const re =
    /`([^`]+)`|\[([^\]]+)\]\(([^)\s]+)\)|\*\*(.+?)\*\*|__(.+?)__|\*([^*\s][^*]*?)\*|(?<![\w])_([^_\s][^_]*?)_(?![\w])/g;
  let last = 0;
  let k = 0;
  for (const m of text.matchAll(re)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const [, code, label, url, bold1, bold2, em1, em2] = m;
    if (code !== undefined) out.push(<code key={k++}>{code}</code>);
    else if (label !== undefined) {
      const safe = /^https?:\/\//i.test(url);
      out.push(
        safe && onLink ? (
          <a
            key={k++}
            href={url}
            title={url}
            onClick={(e) => {
              e.preventDefault();
              onLink(url);
            }}
          >
            {inline(label)}
          </a>
        ) : (
          <Fragment key={k++}>{inline(label)}</Fragment>
        ),
      );
    } else if (bold1 !== undefined || bold2 !== undefined)
      out.push(<strong key={k++}>{inline(bold1 ?? bold2, onLink)}</strong>);
    else out.push(<em key={k++}>{inline(em1 ?? em2, onLink)}</em>);
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
