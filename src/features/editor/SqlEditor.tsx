import { useEffect, useRef } from "react";
import Editor from "@monaco-editor/react";
import type { Schema } from "../../api";
import { useLatest } from "../../hooks/useLatest";
import { useTheme } from "../theme";
import { monaco, monacoTheme, setCompletionSchema } from "./monacoSetup";
import type { Range } from "./selection";

export interface SqlEditorProps {
  value: string;
  onChange: (sql: string) => void;
  /** ⌘↵ / Ctrl+↵ inside the editor. */
  onRun: () => void;
  /** Where the selection is in `value`, as character offsets; null when nothing is selected. */
  onSelectionChange?: (selection: Range | null) => void;
  /** Selects this range when it changes, e.g. to show what ran. */
  select?: Range | null;
  schema: Schema | null;
  /** Only the visible editor's schema feeds autocomplete. */
  active: boolean;
}

export function SqlEditor({
  value,
  onChange,
  onRun,
  onSelectionChange,
  select,
  schema,
  active,
}: SqlEditorProps) {
  const { resolved } = useTheme();
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const runRef = useLatest(onRun);
  const selectionRef = useLatest(onSelectionChange);

  useEffect(() => {
    if (active) setCompletionSchema(schema);
  }, [active, schema]);

  useEffect(() => {
    const editor = editorRef.current;
    const model = editor?.getModel();
    if (!editor || !model || !select) return;
    const start = model.getPositionAt(select.start);
    const end = model.getPositionAt(select.end);
    editor.setSelection(monaco.Selection.fromPositions(start, end));
    editor.revealRangeInCenterIfOutsideViewport(monaco.Range.fromPositions(start, end));
  }, [select]);

  return (
    <Editor
      language="sql"
      theme={monacoTheme(resolved)}
      value={value}
      onChange={(v) => onChange(v ?? "")}
      onMount={(editor) => {
        editorRef.current = editor;
        // An action, not addCommand: Monaco binds commands to the last editor created, so with
        // several tabs open ⌘↵ would run another tab's query.
        editor.addAction({
          id: "sayql.run",
          label: "Run",
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
          run: () => runRef.current(),
        });
        editor.onDidChangeCursorSelection((e) => {
          const model = editor.getModel();
          if (!model || e.selection.isEmpty()) return selectionRef.current?.(null);
          selectionRef.current?.({
            start: model.getOffsetAt(e.selection.getStartPosition()),
            end: model.getOffsetAt(e.selection.getEndPosition()),
          });
        });
      }}
      options={{
        minimap: { enabled: false },
        fontSize: 13,
        fontFamily: "'JetBrains Mono', 'SF Mono', Menlo, Consolas, monospace",
        lineNumbersMinChars: 3,
        scrollBeyondLastLine: false,
        wordWrap: "on",
        automaticLayout: true,
        padding: { top: 8 },
        renderLineHighlight: "line",
        overviewRulerBorder: false,
        scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10 },
      }}
    />
  );
}
