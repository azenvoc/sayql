import { useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { QueryResult } from "../../api";
import { cx } from "../../lib/cx";
import { columnWidths, formatCell } from "./format";
import styles from "./DataGrid.module.css";

const ROW_HEIGHT = 24;
const INDEX_WIDTH = 52;

/** Virtualized read-only grid; renders only the visible rows, so thousands of rows stay smooth. */
export function DataGrid({ result }: { result: QueryResult }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const widths = useMemo(() => columnWidths(result), [result]);
  const template = `${INDEX_WIDTH}px ${widths.map((w) => `${w}px`).join(" ")}`;
  const totalWidth = INDEX_WIDTH + widths.reduce((a, b) => a + b, 0);

  const virtualizer = useVirtualizer({
    count: result.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  return (
    <div className={styles.scroll} ref={scrollRef} role="grid" aria-rowcount={result.rows.length}>
      <div style={{ width: totalWidth, minWidth: "100%" }}>
        <div className={styles.head} style={{ gridTemplateColumns: template }} role="row">
          <div className={cx(styles.cell, styles.index)} />
          {result.columns.map((name, i) => (
            <div key={i} className={styles.cell} role="columnheader" title={name}>
              {name}
            </div>
          ))}
        </div>
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((item) => (
            <div
              key={item.key}
              className={cx(styles.row, item.index % 2 === 1 && styles.alt)}
              role="row"
              style={{ gridTemplateColumns: template, transform: `translateY(${item.start}px)` }}
            >
              <div className={cx(styles.cell, styles.index)}>{item.index + 1}</div>
              {result.rows[item.index].map((value, i) => {
                const text = formatCell(value);
                return (
                  <div
                    key={i}
                    role="gridcell"
                    title={text}
                    className={cx(
                      styles.cell,
                      value === null && styles.null,
                      typeof value === "number" && styles.number,
                    )}
                  >
                    {text}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
