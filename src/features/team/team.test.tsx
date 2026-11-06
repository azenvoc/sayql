// @vitest-environment jsdom
import { cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionConfig, Settings, TeamAccount, TeamAiStatus, TeamStatus } from "../../api";
import { ModelPicker } from "../settings/ModelPicker";
import { DatabaseNavigator } from "../navigator/DatabaseNavigator";
import { TeamSection } from "./TeamSection";
import { TEAMS_URL, licenceText, parseInviteLink } from "./teamText";
import { TeamSignInDialog } from "./TeamSignInDialog";
import { useTeamConnections } from "./useTeamConnections";

// Every command a team feature could send; a signed-out app must call none of them.
const teamStatus = vi.fn<() => Promise<TeamStatus | null>>();
const teamConnections = vi.fn<() => Promise<ConnectionConfig[]>>();
const teamSignOut = vi.fn<() => Promise<void>>();
const teamSignIn = vi.fn<(server: string, code: string) => Promise<TeamAccount>>();
const teamAi = vi.fn<() => Promise<TeamAiStatus | null>>();
const teamSsoStart = vi.fn<(server: string) => Promise<void>>();
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    teamStatus: () => teamStatus(),
    teamConnections: () => teamConnections(),
    teamSignOut: () => teamSignOut(),
    teamSignIn: (s: string, c: string) => teamSignIn(s, c),
    teamAi: () => teamAi(),
    teamSsoStart: (s: string) => teamSsoStart(s),
  },
}));
const openUrl = vi.fn();
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (u: string) => openUrl(u) }));

const account: TeamAccount = {
  server: "https://sayql.acme.test",
  email: "ana@acme.test",
  name: "Ana",
  role: "member",
};
const info = {
  name: "SayQL server",
  version: "0.1.0",
  licensed: true,
  plan: "team",
  licensee: "Acme",
  expires: "2027-04-30",
  users: 3,
  userLimit: 10,
  features: [] as string[],
};

const ai: TeamAiStatus = {
  enabled: true,
  provider: "anthropic",
  models: ["claude-sonnet-5-5", "claude-haiku-5-5"],
  defaultModel: "claude-sonnet-5-5",
  fastModel: "claude-haiku-5-5",
  month: "2026-11",
  you: { tokens: 120_000, limit: 2_000_000 },
  team: { tokens: 900_000, limit: null },
};

const conn = (id: string, name: string, team?: ConnectionConfig["team"]): ConnectionConfig => ({
  id,
  name,
  kind: "postgres",
  host: "",
  port: null,
  database: "",
  user: "",
  file: "",
  ssl: false,
  trustCert: false,
  production: false,
  team,
});

describe("SayQL Team", () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(cleanup);

  it("signed out, sends nothing anywhere: no status, no connections", async () => {
    const apply = vi.fn();
    renderHook(() => useTeamConnections(null, apply, vi.fn()));
    render(<TeamSection account={null} onChange={vi.fn()} />);
    expect(
      screen.getByText("Using SayQL with a team? Sign in to your company's SayQL server."),
    ).toBeTruthy();
    await new Promise((r) => setTimeout(r, 10));
    expect(apply).toHaveBeenCalledWith([]);
    for (const command of [
      teamStatus,
      teamConnections,
      teamSignOut,
      teamSignIn,
      teamAi,
      teamSsoStart,
    ])
      expect(command).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("SayQL for teams"));
    expect(openUrl).toHaveBeenCalledWith(TEAMS_URL);
    // Opening the dialog doesn't sign in: the person confirms first.
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    expect(screen.getByRole("dialog", { name: "Sign in to your team" })).toBeTruthy();
    expect(teamSignIn).not.toHaveBeenCalled();
  });

  it("asks again when SayQL comes to the front, applying only a list that changed", async () => {
    const prod = conn("team:c_1", "Production", { database: "app", notes: "", groups: [] });
    const billing = conn("team:c_2", "Billing", { database: "b", notes: "", groups: ["Portside"] });
    teamConnections.mockResolvedValue([prod, billing]);
    teamAi.mockResolvedValue(ai);
    const apply = vi.fn();
    const onLost = vi.fn();
    renderHook(() => useTeamConnections(account, apply, vi.fn(), vi.fn(), onLost));
    await waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
    expect(onLost).not.toHaveBeenCalled();

    // Nothing changed: the list isn't applied again, which would reload the schemas.
    window.dispatchEvent(new Event("focus"));
    await waitFor(() => expect(teamConnections).toHaveBeenCalledTimes(2));
    await new Promise((r) => setTimeout(r));
    expect(apply).toHaveBeenCalledTimes(1);

    // Access was removed on the server.
    teamConnections.mockResolvedValue([prod]);
    window.dispatchEvent(new Event("focus"));
    await waitFor(() => expect(apply).toHaveBeenCalledTimes(2));
    expect(onLost).toHaveBeenCalledWith([expect.objectContaining({ name: "Billing" })]);

    // A server out of reach says nothing when coming back to the window, and changes nothing.
    teamConnections.mockRejectedValue("Couldn't reach it.");
    window.dispatchEvent(new Event("focus"));
    await new Promise((r) => setTimeout(r));
    expect(apply).toHaveBeenCalledTimes(2);
  });

  it("signed in, loads the team's connections and shows the account and licence", async () => {
    teamConnections.mockResolvedValue([
      conn("team:c_1", "Production", { database: "app", notes: "", groups: [] }),
    ]);
    teamStatus.mockResolvedValue({ account, server: info, ai });
    teamSignOut.mockResolvedValue();
    teamAi.mockResolvedValue(ai);
    const apply = vi.fn();
    const onAi = vi.fn();
    renderHook(() => useTeamConnections(account, apply, vi.fn(), onAi));
    await waitFor(() =>
      expect(apply).toHaveBeenCalledWith([expect.objectContaining({ id: "team:c_1" })]),
    );
    await waitFor(() => expect(onAi).toHaveBeenCalledWith(ai));

    const onChange = vi.fn();
    render(<TeamSection account={account} onChange={onChange} />);
    await waitFor(() =>
      expect(
        screen.getByText("Team plan for Acme · 3 of 10 seats · until 2027-04-30"),
      ).toBeTruthy(),
    );
    expect(screen.getByText("ana@acme.test")).toBeTruthy();
    // Team AI: who provides it, and this month's usage against the limits.
    expect(screen.getByText("Provided by your team (Anthropic Claude)")).toBeTruthy();
    expect(screen.getByText("You: 120k of 2M tokens")).toBeTruthy();
    expect(screen.getByText("Your team: 900k tokens (no limit)")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(null));
  });

  it("says when the server has no licence", () => {
    expect(licenceText({ ...info, licensed: false })).toContain("No valid licence");
  });

  it("names a server's built-in trial plainly", () => {
    expect(
      licenceText({ ...info, plan: "trial", licensee: "Free trial", expires: "2026-11-26" }),
    ).toBe("Free trial · 3 of 10 seats · until 2026-11-26");
  });

  it("reads both kinds of invite link, and nothing else", () => {
    const want = { server: "https://sayql.acme.test", code: "ABCD-EFGH" };
    expect(
      parseInviteLink("https://sayql.dev/join#server=https://sayql.acme.test&code=ABCD-EFGH"),
    ).toEqual(want);
    expect(
      parseInviteLink(
        " https://sayql.dev/join/#server=https%3A%2F%2Fsayql.acme.test&code=ABCD-EFGH ",
      ),
    ).toEqual(want);
    expect(
      parseInviteLink("sayql://team-join?server=https%3A%2F%2Fsayql.acme.test&code=ABCD-EFGH"),
    ).toEqual(want);
    for (const other of [
      "https://sayql.acme.test",
      "ABCD-EFGH",
      "https://sayql.dev/join#server=https://a.test",
      "https://x.test/?server=a&code=b",
    ]) {
      expect(parseInviteLink(other)).toBeNull();
    }
  });

  it("a pasted invite link fills the server and the code, and asks to check the server", () => {
    render(<TeamSignInDialog onSignedIn={vi.fn()} onClose={vi.fn()} />);
    expect(screen.queryByText(/An invite link asks you to sign in/)).toBeNull();
    const [server, code] = screen.getAllByRole("textbox") as HTMLInputElement[];
    fireEvent.change(server, {
      target: { value: "https://sayql.dev/join#server=https://sayql.acme.test&code=ABCD-EFGH" },
    });
    expect(server.value).toBe("https://sayql.acme.test");
    expect(code.value).toBe("ABCD-EFGH");
    expect(screen.getByText(/An invite link asks you to sign in/)).toBeTruthy();
    expect(teamSignIn).not.toHaveBeenCalled();
  });

  it("signs in with the company in the browser, after the server is entered", async () => {
    teamSsoStart.mockResolvedValue();
    render(<TeamSignInDialog onSignedIn={vi.fn()} onClose={vi.fn()} />);
    const company = screen.getByRole("button", {
      name: "Sign in with your company",
    }) as HTMLButtonElement;
    expect(company.disabled).toBe(true);
    fireEvent.change(screen.getAllByRole("textbox")[0], { target: { value: "sayql.acme.test" } });
    fireEvent.click(company);
    await screen.findByText(/Finish signing in in your browser/);
    expect(teamSsoStart).toHaveBeenCalledWith("sayql.acme.test");
    expect(teamSignIn).not.toHaveBeenCalled();
  });

  it("says why when the server has no company sign-in", async () => {
    teamSsoStart.mockRejectedValue(
      "Your team's SayQL server doesn't have sign-in with your company turned on.",
    );
    render(
      <TeamSignInDialog server="https://sayql.acme.test" onSignedIn={vi.fn()} onClose={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Sign in with your company" }));
    await screen.findByText(/doesn't have sign-in with your company/);
    expect(screen.queryByText(/Finish signing in in your browser/)).toBeNull();
  });

  it("signed out, offers a quiet way to sign in that can be hidden", () => {
    localStorage.clear();
    const onTeamSignIn = vi.fn();
    const nav = (signIn?: () => void) => (
      <DatabaseNavigator
        connections={[conn("c1", "Local")]}
        activeId={null}
        databases={{}}
        databaseName={null}
        schema={null}
        loading={false}
        connected={false}
        onConnect={vi.fn()}
        onOpen={vi.fn()}
        onOpenDatabase={vi.fn()}
        onAdd={vi.fn()}
        onEdit={vi.fn()}
        onDeleteConnection={vi.fn()}
        onRefresh={vi.fn()}
        onPreviewTable={vi.fn()}
        onNewTab={vi.fn()}
        onOpenSql={vi.fn()}
        selectedTables={[]}
        onSelectTables={vi.fn()}
        scripts={[]}
        onOpenScript={vi.fn()}
        onDeleteScript={vi.fn()}
        notes={[]}
        onEditNote={vi.fn()}
        onDeleteNote={vi.fn()}
        onAgentSettings={vi.fn()}
        onOverview={vi.fn()}
        onTeamSignIn={signIn}
      />
    );
    const { rerender } = render(nav(onTeamSignIn));
    fireEvent.click(screen.getByText("Have a team invite? Sign in"));
    expect(onTeamSignIn).toHaveBeenCalledTimes(1);
    // Signed in: no hint.
    rerender(nav(undefined));
    expect(screen.queryByText("Have a team invite? Sign in")).toBeNull();
    // Hidden: stays hidden.
    rerender(nav(onTeamSignIn));
    fireEvent.click(screen.getByRole("button", { name: "Hide" }));
    expect(screen.queryByText("Have a team invite? Sign in")).toBeNull();
    expect(onTeamSignIn).toHaveBeenCalledTimes(1);
  });

  it("lists team connections in their own group, with no edit or delete", () => {
    render(
      <DatabaseNavigator
        connections={[
          conn("c1", "Local"),
          conn("team:c_1", "Production", { database: "app", notes: "Read replica", groups: [] }),
        ]}
        activeId={null}
        databases={{}}
        databaseName={null}
        schema={null}
        loading={false}
        connected={false}
        onConnect={vi.fn()}
        onOpen={vi.fn()}
        onOpenDatabase={vi.fn()}
        onAdd={vi.fn()}
        onEdit={vi.fn()}
        onDeleteConnection={vi.fn()}
        onRefresh={vi.fn()}
        onPreviewTable={vi.fn()}
        onNewTab={vi.fn()}
        onOpenSql={vi.fn()}
        selectedTables={[]}
        onSelectTables={vi.fn()}
        scripts={[]}
        onOpenScript={vi.fn()}
        onDeleteScript={vi.fn()}
        notes={[]}
        onEditNote={vi.fn()}
        onDeleteNote={vi.fn()}
        onAgentSettings={vi.fn()}
        onOverview={vi.fn()}
      />,
    );
    expect(screen.getByText("Team")).toBeTruthy();
    expect(screen.getByText("Production")).toBeTruthy();
    // One Edit button: the local connection's.
    expect(screen.getAllByRole("button", { name: "Edit connection" })).toHaveLength(1);
    fireEvent.contextMenu(screen.getByText("Production"));
    expect(screen.queryByText("Edit connection…")).toBeNull();
    expect(screen.queryByText("Delete connection…")).toBeNull();
    expect(screen.getByText("Investigate settings…")).toBeTruthy();
  });

  it("on a team connection with team AI, shows the team's models, set by the admin", () => {
    const settings = {
      aiReady: false,
      providers: [],
      models: [],
      provider: "anthropic",
      typingModel: "",
      expertModel: "",
      agentModel: "",
    } as unknown as Settings;
    render(
      <ModelPicker
        settings={settings}
        purpose="typing"
        onChange={vi.fn()}
        onError={vi.fn()}
        onOpenSettings={vi.fn()}
        team={ai}
      />,
    );
    fireEvent.click(screen.getByText(/claude-haiku-5-5 · Team/));
    expect(screen.getByText("Set by your team admin")).toBeTruthy();
    expect(screen.getByText("claude-sonnet-5-5")).toBeTruthy();
    expect(screen.queryByText("More models and providers…")).toBeNull();
  });
});
