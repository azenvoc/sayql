// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SOURCE_CAP_MESSAGE, type KnowledgeSource } from "../../api";
import { TEAMS_URL } from "../team/teamText";
import { KnowledgeSources, knownTokenHost, sourceStatus } from "./KnowledgeSources";

const folder: KnowledgeSource = {
  id: "src_web",
  label: "web",
  kind: { type: "local", path: "/Users/me/code/web" },
  lastFetch: null,
  lastCommit: null,
  error: null,
};
const repo: KnowledgeSource = {
  id: "src_api",
  label: "api",
  kind: {
    type: "git",
    url: "https://github.com/acme/api.git",
    branch: "main",
    subdir: null,
    hasToken: true,
    tokenUser: null,
  },
  lastFetch: Date.now() - 2 * 3_600_000,
  lastCommit: "1a2b3c4d5e6f",
  error: null,
};

const docs: KnowledgeSource = {
  id: "src_docs",
  label: "docs.acme.com",
  kind: { type: "url", url: "https://docs.acme.com/data/", crawl: true },
  lastFetch: Date.now() - 5 * 60_000,
  lastCommit: "ab12cd34ef56",
  error: null,
};

let sources: KnowledgeSource[] = [];
type Repo = {
  url: string;
  branch: string;
  subdir: string;
  token: string | null;
  tokenUser: string | null;
};
const addKnowledgeRepo = vi.fn((_id: string, r: Repo) =>
  Promise.resolve<KnowledgeSource>({
    id: "src_new",
    label: "shop",
    kind: {
      type: "git",
      url: r.url,
      branch: r.branch,
      subdir: r.subdir || null,
      hasToken: !!r.token,
      tokenUser: r.tokenUser,
    },
    lastFetch: null,
    lastCommit: null,
    error: null,
  }),
);
const removeKnowledgeSource = vi.fn((_id: string, _source: string) => Promise.resolve());
const refreshKnowledgeSource = vi.fn((_id: string, source: string) =>
  Promise.resolve({
    ...repo,
    id: source,
    error: "github.com didn't accept the sign-in. Add a read-only token for this repository.",
  }),
);
const addKnowledgeFolder = vi.fn((_id: string) => Promise.resolve<KnowledgeSource | null>(null));
const addKnowledgeFiles = vi.fn((_id: string) =>
  Promise.resolve<KnowledgeSource | null>({
    id: "src_files",
    label: "docs",
    kind: { type: "files", paths: ["/Users/me/specs/billing.pdf", "/Users/me/specs/codes.md"] },
    lastFetch: Date.now(),
    lastCommit: "4e5f6a7b8c9d",
    error: "codes.md isn't there any more.",
  }),
);
const addKnowledgeUrl = vi.fn((_id: string, url: string, crawl: boolean) =>
  Promise.resolve<KnowledgeSource>({ ...docs, id: "src_web2", kind: { type: "url", url, crawl } }),
);
const openUrl = vi.fn((_url: string) => Promise.resolve());

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (u: string) => openUrl(u) }));

vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    knowledgeSources: () => Promise.resolve(sources),
    addKnowledgeRepo: (id: string, r: Repo) => addKnowledgeRepo(id, r),
    removeKnowledgeSource: (id: string, s: string) => removeKnowledgeSource(id, s),
    refreshKnowledgeSource: (id: string, s: string) => refreshKnowledgeSource(id, s),
    addKnowledgeFolder: (id: string) => addKnowledgeFolder(id),
    addKnowledgeFiles: (id: string) => addKnowledgeFiles(id),
    addKnowledgeUrl: (id: string, url: string, crawl: boolean) => addKnowledgeUrl(id, url, crawl),
  },
}));

beforeEach(() => {
  sources = [folder, repo];
  addKnowledgeRepo.mockClear();
  removeKnowledgeSource.mockClear();
  refreshKnowledgeSource.mockClear();
  addKnowledgeUrl.mockClear();
  openUrl.mockClear();
});
afterEach(cleanup);

it("lists each source with where it is and how its last fetch went", async () => {
  const onChange = vi.fn();
  render(<KnowledgeSources connectionId="c1" onChange={onChange} />);
  const list = await screen.findByRole("list", { name: "Knowledge sources" });
  const items = within(list).getAllByRole("listitem");
  expect(items).toHaveLength(2);
  expect(items[0].textContent).toContain("/Users/me/code/web");
  expect(items[0].textContent).toContain("Read where it is");
  expect(items[1].textContent).toContain("https://github.com/acme/api.git · main");
  expect(items[1].textContent).toContain("Fetched 2 hours ago · 1a2b3c4 · token saved");
  expect(within(items[0]).getByRole("img", { name: "Folder" })).toBeTruthy();
  expect(within(items[1]).getByRole("img", { name: "Git repository" })).toBeTruthy();
  expect(onChange).toHaveBeenLastCalledWith([folder, repo]);
});

it("removes a source and checks access, showing the error", async () => {
  const onChange = vi.fn();
  render(<KnowledgeSources connectionId="c1" onChange={onChange} />);
  fireEvent.click(await screen.findByRole("button", { name: "Refresh api" }));
  expect(await screen.findByText(/didn't accept the sign-in/)).toBeTruthy();
  expect(refreshKnowledgeSource).toHaveBeenCalledWith("c1", "src_api");
  fireEvent.click(screen.getByRole("button", { name: "Remove web" }));
  await waitFor(() => expect(removeKnowledgeSource).toHaveBeenCalledWith("c1", "src_web"));
  await waitFor(() => expect(screen.queryByText("/Users/me/code/web")).toBeNull());
  expect(onChange.mock.lastCall?.[0].map((s: KnowledgeSource) => s.id)).toEqual(["src_api"]);
});

it("adds a repository with a branch, a folder and a token", async () => {
  sources = [];
  render(<KnowledgeSources connectionId="c1" />);
  expect(await screen.findByText(/Nothing yet/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Add repository…" }));
  const form = screen.getByRole("form", { name: "Add a repository" });
  const add = within(form).getByRole("button", { name: "Add repository" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  fireEvent.change(within(form).getByLabelText("Repository address"), {
    target: { value: " https://github.com/acme/shop.git " },
  });
  fireEvent.change(within(form).getByLabelText(/Branch/), { target: { value: "release" } });
  fireEvent.change(within(form).getByLabelText(/Folder in the repository/), {
    target: { value: "services/api" },
  });
  const token = within(form).getByLabelText(/Read-only token/) as HTMLInputElement;
  expect(token.type).toBe("password");
  fireEvent.change(token, { target: { value: "ghp_x" } });
  // GitHub's token user name is known: nothing more to ask.
  expect(within(form).queryByLabelText("User name for the token")).toBeNull();
  fireEvent.click(add);
  await waitFor(() =>
    expect(addKnowledgeRepo).toHaveBeenCalledWith("c1", {
      url: "https://github.com/acme/shop.git",
      branch: "release",
      subdir: "services/api",
      token: "ghp_x",
      tokenUser: null,
    }),
  );
  expect(
    await screen.findByText("https://github.com/acme/shop.git · release · services/api"),
  ).toBeTruthy();
  expect(screen.queryByRole("form", { name: "Add a repository" })).toBeNull();
});

it("asks for the token's user name on a server it doesn't know, and shows why adding failed", async () => {
  sources = [];
  addKnowledgeRepo.mockImplementationOnce(() =>
    Promise.reject("git.acme.dev didn't accept the sign-in."),
  );
  render(<KnowledgeSources connectionId="c1" />);
  fireEvent.click(await screen.findByRole("button", { name: "Add repository…" }));
  const form = screen.getByRole("form", { name: "Add a repository" });
  fireEvent.change(within(form).getByLabelText("Repository address"), {
    target: { value: "https://git.acme.dev/shop.git" },
  });
  fireEvent.change(within(form).getByLabelText(/Read-only token/), { target: { value: "tok" } });
  const add = within(form).getByRole("button", { name: "Add repository" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  fireEvent.change(within(form).getByLabelText("User name for the token"), {
    target: { value: "deploy" },
  });
  fireEvent.click(add);
  expect(await within(form).findByText("git.acme.dev didn't accept the sign-in.")).toBeTruthy();
  expect(addKnowledgeRepo.mock.lastCall?.[1].tokenUser).toBe("deploy");
  fireEvent.click(within(form).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("form", { name: "Add a repository" })).toBeNull();
});

it("knows the token user names of GitHub, GitLab and Bitbucket", () => {
  expect(knownTokenHost("https://github.com/a/b")).toBe(true);
  expect(knownTokenHost("https://gitlab.acme.dev/a/b")).toBe(true);
  expect(knownTokenHost("https://bitbucket.org/a/b")).toBe(true);
  expect(knownTokenHost("https://git.acme.dev/a/b")).toBe(false);
  expect(sourceStatus({ ...repo, lastFetch: null })).toBe("Fetched when the audit runs");
  expect(
    sourceStatus({ ...folder, lastFetch: 1000, lastCommit: "abcdef123" }, 1000 + 3 * 86_400_000),
  ).toBe("Read 3 days ago · abcdef1");
  expect(sourceStatus({ ...docs, lastFetch: null })).toBe("Read when the audit runs");
});

it("adds files, shown with their kind and what couldn't be read", async () => {
  sources = [];
  render(<KnowledgeSources connectionId="c1" />);
  fireEvent.click(await screen.findByRole("button", { name: "Add files…" }));
  const item = await screen.findByRole("listitem");
  expect(addKnowledgeFiles).toHaveBeenCalledWith("c1");
  expect(within(item).getByRole("img", { name: "Files" })).toBeTruthy();
  expect(item.textContent).toContain("billing.pdf, codes.md");
  expect(item.textContent).toContain("codes.md isn't there any more.");
  expect(within(item).getByRole("button", { name: "Refresh docs" }).getAttribute("title")).toBe(
    "Read these files again",
  );
});

it("adds a web page, with or without the pages in its folder", async () => {
  sources = [];
  render(<KnowledgeSources connectionId="c1" />);
  fireEvent.click(await screen.findByRole("button", { name: "Add web page…" }));
  const form = screen.getByRole("form", { name: "Add a web page" });
  fireEvent.change(within(form).getByLabelText("Page address"), {
    target: { value: " https://docs.acme.com/data/ " },
  });
  fireEvent.click(within(form).getByLabelText(/Also read the pages in its folder/));
  fireEvent.click(within(form).getByRole("button", { name: "Add web page" }));
  await waitFor(() =>
    expect(addKnowledgeUrl).toHaveBeenCalledWith("c1", "https://docs.acme.com/data/", true),
  );
  const item = await screen.findByRole("listitem");
  expect(within(item).getByRole("img", { name: "Web page" })).toBeTruthy();
  expect(item.textContent).toContain("https://docs.acme.com/data/ · and the pages in its folder");
  expect(item.textContent).toContain("Read 5 minutes ago · ab12cd3");
  expect(screen.queryByRole("form", { name: "Add a web page" })).toBeNull();
});

it("points to SayQL for teams at the free app's 3 sources", async () => {
  sources = [folder, repo, docs];
  render(<KnowledgeSources connectionId="c1" />);
  expect(
    await screen.findByText(/3 sources is the most for one connection in the free app/),
  ).toBeTruthy();
  expect(screen.getByText(new RegExp(SOURCE_CAP_MESSAGE))).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Add files…" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "SayQL for teams" }));
  expect(openUrl).toHaveBeenCalledWith(TEAMS_URL);
  // Removing one brings the buttons back.
  fireEvent.click(screen.getByRole("button", { name: "Remove docs.acme.com" }));
  expect(await screen.findByRole("button", { name: "Add files…" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "SayQL for teams" })).toBeNull();
});

it("shows the cap as a pointer, not an error, when the app refuses a 4th source", async () => {
  sources = [];
  addKnowledgeUrl.mockImplementationOnce(() => Promise.reject(SOURCE_CAP_MESSAGE));
  render(<KnowledgeSources connectionId="c1" />);
  fireEvent.click(await screen.findByRole("button", { name: "Add web page…" }));
  const form = screen.getByRole("form", { name: "Add a web page" });
  fireEvent.change(within(form).getByLabelText("Page address"), {
    target: { value: "https://docs.acme.com" },
  });
  fireEvent.click(within(form).getByRole("button", { name: "Add web page" }));
  expect(await within(form).findByRole("button", { name: "SayQL for teams" })).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});
