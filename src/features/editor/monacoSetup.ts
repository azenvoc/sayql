// Monaco is loaded from the local package (not a CDN) so the editor works offline.
import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/languages/definitions/sql/register";
// editor.api is the bare editor; each feature beyond typing is a contribution loaded on its own.
// Monaco's context menu stays out so the editor keeps the native cut/copy/paste menu.
import "monaco-editor/editor/contrib/suggest/browser/suggestController";
import "monaco-editor/editor/contrib/find/browser/findController";
import "monaco-editor/editor/contrib/comment/browser/comment";
import "monaco-editor/editor/contrib/bracketMatching/browser/bracketMatching";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { loader } from "@monaco-editor/react";
import type { Schema } from "../../api";
import type { ResolvedTheme } from "../theme";
import { qualifiedName } from "../schema";

self.MonacoEnvironment = { getWorker: () => new EditorWorker() };
loader.config({ monaco });

// Monaco needs literal colors; these mirror --color-surface and friends in styles/tokens.css.
monaco.editor.defineTheme("sayql-light", {
  base: "vs",
  inherit: true,
  rules: [
    { token: "keyword", foreground: "0b4fc4", fontStyle: "bold" },
    { token: "string", foreground: "1a7f37" },
    { token: "number", foreground: "b35900" },
    { token: "comment", foreground: "8a939d", fontStyle: "italic" },
    { token: "operator", foreground: "56606b" },
  ],
  colors: {
    "editor.background": "#ffffff",
    "editor.lineHighlightBackground": "#f6f7f9",
    "editorLineNumber.foreground": "#b9c0ca",
    "editorLineNumber.activeForeground": "#56606b",
    "editorGutter.background": "#f6f7f9",
  },
});

monaco.editor.defineTheme("sayql-dark", {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "keyword", foreground: "6aa2ff", fontStyle: "bold" },
    { token: "string", foreground: "8fd19e" },
    { token: "number", foreground: "f0a868" },
    { token: "comment", foreground: "727a85", fontStyle: "italic" },
    { token: "operator", foreground: "a3aab4" },
  ],
  colors: {
    "editor.background": "#1f2227",
    "editor.lineHighlightBackground": "#24272d",
    "editorLineNumber.foreground": "#474c55",
    "editorLineNumber.activeForeground": "#a3aab4",
    "editorGutter.background": "#24272d",
  },
});

export const monacoTheme = (theme: ResolvedTheme) =>
  theme === "dark" ? "sayql-dark" : "sayql-light";

const KEYWORDS = [
  "SELECT",
  "FROM",
  "WHERE",
  "GROUP BY",
  "ORDER BY",
  "HAVING",
  "LIMIT",
  "JOIN",
  "LEFT JOIN",
  "INNER JOIN",
  "ON",
  "AS",
  "AND",
  "OR",
  "NOT",
  "IN",
  "IS NULL",
  "IS NOT NULL",
  "COUNT",
  "SUM",
  "AVG",
  "MIN",
  "MAX",
  "DISTINCT",
  "CASE",
  "WHEN",
  "THEN",
  "ELSE",
  "END",
  "WITH",
  "UNION",
  "DESC",
  "ASC",
  "BETWEEN",
  "LIKE",
];

let completionSchema: Schema | null = null;

/** Tables and columns offered by autocomplete. One provider serves every editor instance. */
export function setCompletionSchema(schema: Schema | null) {
  completionSchema = schema;
}

monaco.languages.registerCompletionItemProvider("sql", {
  provideCompletionItems(model, position) {
    const word = model.getWordUntilPosition(position);
    const range = new monaco.Range(
      position.lineNumber,
      word.startColumn,
      position.lineNumber,
      word.endColumn,
    );
    const Kind = monaco.languages.CompletionItemKind;
    const suggestions: monaco.languages.CompletionItem[] = KEYWORDS.map((k) => ({
      label: k,
      kind: Kind.Keyword,
      insertText: k,
      range,
    }));
    for (const t of completionSchema?.tables ?? []) {
      const name = qualifiedName(t);
      suggestions.push({ label: name, kind: Kind.Struct, detail: t.kind, insertText: name, range });
      for (const c of t.columns) {
        suggestions.push({
          label: c.name,
          kind: Kind.Field,
          detail: `${t.name} · ${c.data_type}`,
          insertText: c.name,
          range,
        });
      }
    }
    return { suggestions };
  },
});

export { monaco };
