import { useState, type MouseEvent, type ReactNode } from "react";
import { cx } from "../../lib/cx";
import { usePersistentState } from "../../hooks/usePersistentState";
import styles from "./SplitPane.module.css";

export interface SplitPaneProps {
  /** "horizontal" puts the panes side by side; "vertical" stacks them. */
  direction: "horizontal" | "vertical";
  /** Size in px of the first pane. */
  initialSize: number;
  minSize?: number;
  maxSize?: number;
  /** Remembers the size across launches under this key. */
  persistKey: string;
  first: ReactNode;
  second: ReactNode;
}

/** Two panes with a draggable divider. */
export function SplitPane({
  direction,
  initialSize,
  minSize = 120,
  maxSize = 900,
  persistKey,
  first,
  second,
}: SplitPaneProps) {
  const [size, setSize] = usePersistentState(`split:${persistKey}`, initialSize);
  const [dragging, setDragging] = useState(false);
  const horizontal = direction === "horizontal";

  const startDrag = (e: MouseEvent) => {
    e.preventDefault();
    const start = horizontal ? e.clientX : e.clientY;
    const startSize = size;
    setDragging(true);
    const move = (ev: globalThis.MouseEvent) => {
      const delta = (horizontal ? ev.clientX : ev.clientY) - start;
      setSize(Math.max(minSize, Math.min(maxSize, startSize + delta)));
    };
    const up = () => {
      setDragging(false);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div className={cx(styles.split, horizontal ? styles.horizontal : styles.vertical)}>
      <div className={styles.first} style={horizontal ? { width: size } : { height: size }}>
        {first}
      </div>
      <div
        className={cx(styles.divider, dragging && styles.dragging)}
        role="separator"
        aria-orientation={horizontal ? "vertical" : "horizontal"}
        onMouseDown={startDrag}
      />
      <div className={styles.second}>{second}</div>
    </div>
  );
}
