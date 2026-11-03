// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { QUERY_CANCELLED } from "../../api";
import { ResultsPanel } from "./ResultsPanel";

describe("ResultsPanel", () => {
  afterEach(cleanup);

  it("shows a cancelled query as cancelled, not as a database error", () => {
    const outcomes = [
      { sql: "SELECT pg_sleep(30)", result: null, error: QUERY_CANCELLED, cancelled: false },
    ];
    render(<ResultsPanel outcomes={outcomes} total={1} running={false} />);
    expect(screen.getByText("Query cancelled")).toBeTruthy();
    expect(screen.queryByText(/returned an error/)).toBeNull();
  });

  it("still shows real database errors as errors", () => {
    const outcomes = [
      {
        sql: "SELECT nme FROM t",
        result: null,
        error: 'column "nme" does not exist',
        cancelled: false,
      },
    ];
    render(<ResultsPanel outcomes={outcomes} total={1} running={false} />);
    expect(screen.getByText("The database returned an error")).toBeTruthy();
  });

  it("shows a team server's refusal as a refusal, with no AI fix", () => {
    const fix = <button type="button">Fix with AI</button>;
    const refusal = {
      sql: "DELETE FROM t",
      result: null,
      error: "Your team's SayQL server runs only read-only queries, so this statement was not run.",
      cancelled: false,
      refused: true,
    };
    render(<ResultsPanel outcomes={[refusal]} total={1} running={false} errorAction={fix} />);
    expect(screen.getByText("Your team's SayQL server refused this")).toBeTruthy();
    expect(screen.queryByText("Fix with AI")).toBeNull();
    cleanup();
    // A database error from the team server is still a database error.
    render(
      <ResultsPanel
        outcomes={[{ ...refusal, error: "no such column: x", refused: false }]}
        total={1}
        running={false}
        errorAction={fix}
      />,
    );
    expect(screen.getByText("The database returned an error")).toBeTruthy();
    expect(screen.getByText("Fix with AI")).toBeTruthy();
  });
});
