import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import "./styles/base.css";

async function start() {
  // In a plain browser during development, fake the Rust backend so the UI can be previewed.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    (await import("./dev/mockBackend")).installMockBackend();
  }
  // Release builds hide the webview's own menu (Reload, Inspect Element) so right-click
  // feels native. Text fields and the SQL editor keep theirs (cut, copy, paste).
  if (!import.meta.env.DEV) {
    document.addEventListener("contextmenu", (e) => {
      if (
        !(e.target as Element).closest("input, textarea, [contenteditable='true'], .monaco-editor")
      )
        e.preventDefault();
    });
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void start();
