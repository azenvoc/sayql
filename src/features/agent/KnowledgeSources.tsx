import { useEffect, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, FilePlus, FileText, FolderOpen, FolderPlus, GitBranch, Globe, Loader2, Plus, RefreshCw, Trash2, type LucideIcon } from "lucide-react";
import { MAX_SOURCES_FREE, SOURCE_CAP_MESSAGE, commands, type KnowledgeSource } from "../../api";
import { Alert, Button, Checkbox, Field, IconButton, Spinner, TextInput } from "../../components/ui";
import { errorText } from "../../lib/errors";
import { TEAMS_URL } from "../team/teamText";
import styles from "./KnowledgeSources.module.css";

/** Hosts whose token user name SayQL knows (src-tauri/src/repos.rs `token_user`). */
export function knownTokenHost(url: string): boolean {
  const m = /^(?:https:\/\/|ssh:\/\/|git@)(?:[^@/]*@)?([^/:]+)/i.exec(url.trim());
  const host = m?.[1].toLowerCase() ?? "";
  return host === "github.com" || host.endsWith(".github.com") || host === "gitlab.com" || host.startsWith("gitlab.") || host === "bitbucket.org";
}

type Kind = KnowledgeSource["kind"]["type"];

const ICONS: Record<Kind, LucideIcon> = { local: FolderOpen, git: GitBranch, files: FileText, url: Globe };
const KIND_NAMES: Record<Kind, string> = { local: "Folder", git: "Git repository", files: "Files", url: "Web page" };
const REFRESH_TITLES: Record<Kind, string> = {
  local: "Check this folder is still there",
  git: "Check git can reach this repository",
  files: "Read these files again",
  url: "Read this page again",
};

/** "1a2b3c4", "fetched 2 hours ago": what a source's status line says. */
export function sourceStatus(s: KnowledgeSource, now = Date.now()): string {
  const git = s.kind.type === "git";
  if (!s.lastFetch) return git ? "Fetched when the audit runs" : s.kind.type === "local" ? "Read where it is" : "Read when the audit runs";
  const mins = Math.floor(Math.max(0, now - s.lastFetch) / 60_000);
  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
  const ago =
    mins < 1 ? "just now" : mins < 60 ? rtf.format(-mins, "minute") : mins < 48 * 60 ? rtf.format(-Math.floor(mins / 60), "hour") : rtf.format(-Math.floor(mins / 1440), "day");
  const commit = s.lastCommit ? ` · ${s.lastCommit.slice(0, 7)}` : "";
  return `${git ? "Fetched" : "Read"} ${ago}${commit}`;
}

function where(s: KnowledgeSource): string {
  if (s.kind.type === "local") return s.kind.path;
  if (s.kind.type === "files") return s.kind.paths.map((p) => p.split(/[/\\]/).pop()).join(", ");
  if (s.kind.type === "url") return s.kind.crawl ? `${s.kind.url} · and the pages in its folder` : s.kind.url;
  const branch = s.kind.branch || "default branch";
  return `${s.kind.url} · ${branch}${s.kind.subdir ? ` · ${s.kind.subdir}` : ""}`;
}

export interface KnowledgeSourcesProps {
  connectionId: string;
  /** Told the list each time it loads or changes. */
  onChange?: (sources: KnowledgeSource[]) => void;
}

/** What an audit reads for one connection: folders on this computer, git repositories, files and web pages. */
export function KnowledgeSources({ connectionId, onChange }: KnowledgeSourcesProps) {
  const [sources, setSources] = useState<KnowledgeSource[] | null>(null);
  const [adding, setAdding] = useState<"repo" | "url" | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const show = (list: KnowledgeSource[]) => {
    setSources(list);
    onChange?.(list);
  };

  useEffect(() => {
    let live = true;
    setSources(null);
    setAdding(null);
    setError(null);
    if (!connectionId) return;
    Promise.resolve()
      .then(() => commands.knowledgeSources(connectionId))
      .then(
        (list) => {
          if (!live) return;
          setSources(list);
          onChange?.(list);
        },
        () => live && setSources([]),
      );
    return () => {
      live = false;
    };
    // onChange is a callback from the parent: only the connection reloads the list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId]);

  const list = sources ?? [];
  const replace = (s: KnowledgeSource) => show(list.map((x) => (x.id === s.id ? s : x)));

  const atCap = list.length >= MAX_SOURCES_FREE;
  const added = (s: KnowledgeSource) => {
    show([...list, s]);
    setAdding(null);
  };

  const addFolder = () => {
    setError(null);
    commands.addKnowledgeFolder(connectionId).then(
      (s) => s && show([...list, s]),
      (e) => setError(errorText(e)),
    );
  };

  const addFiles = () => {
    setError(null);
    setBusy("files");
    commands
      .addKnowledgeFiles(connectionId)
      .then(
        (s) => s && show([...list, s]),
        (e) => setError(errorText(e)),
      )
      .finally(() => setBusy(null));
  };

  const remove = (s: KnowledgeSource) => {
    setError(null);
    commands.removeKnowledgeSource(connectionId, s.id).then(
      () => show(list.filter((x) => x.id !== s.id)),
      (e) => setError(errorText(e)),
    );
  };

  const check = (s: KnowledgeSource) => {
    setError(null);
    setBusy(s.id);
    commands
      .refreshKnowledgeSource(connectionId, s.id)
      .then(replace, (e) => setError(errorText(e)))
      .finally(() => setBusy(null));
  };

  return (
    <div className={styles.sources}>
      {sources === null ? (
        <p className={styles.muted}>
          <Spinner /> Loading…
        </p>
      ) : list.length === 0 ? (
        <p className={styles.muted}>Nothing yet: add the folder or repository with this database&apos;s code, or documents about it.</p>
      ) : (
        <ul className={styles.list} aria-label="Knowledge sources">
          {list.map((s) => (
            <li key={s.id} className={styles.item}>
              <SourceIcon kind={s.kind.type} />
              <div className={styles.text}>
                <span className={styles.name}>
                  <strong>{s.label}</strong>
                  <code className={styles.where} title={where(s)}>
                    {where(s)}
                  </code>
                </span>
                {s.error ? (
                  <span className={styles.error}>
                    <AlertTriangle size={12} /> {s.error}
                  </span>
                ) : (
                  <span className={styles.muted}>
                    {sourceStatus(s)}
                    {s.kind.type === "git" && s.kind.hasToken && " · token saved"}
                  </span>
                )}
              </div>
              <IconButton
                icon={busy === s.id ? Loader2 : RefreshCw}
                size="sm"
                label={`Refresh ${s.label}`}
                title={REFRESH_TITLES[s.kind.type]}
                disabled={busy === s.id}
                onClick={() => check(s)}
              />
              <IconButton icon={Trash2} size="sm" label={`Remove ${s.label}`} title="Remove this source" onClick={() => remove(s)} />
            </li>
          ))}
        </ul>
      )}
      {adding === "repo" ? (
        <RepoForm connectionId={connectionId} onAdded={added} onCancel={() => setAdding(null)} />
      ) : adding === "url" ? (
        <UrlForm connectionId={connectionId} onAdded={added} onCancel={() => setAdding(null)} />
      ) : atCap ? (
        <TeamsNote>{MAX_SOURCES_FREE} sources is the most for one connection in the free app.</TeamsNote>
      ) : (
        <div className={styles.add}>
          <Button size="sm" icon={FolderPlus} disabled={!connectionId} onClick={addFolder}>
            Add folder…
          </Button>
          <Button size="sm" icon={Plus} disabled={!connectionId} onClick={() => setAdding("repo")}>
            Add repository…
          </Button>
          <Button size="sm" icon={FilePlus} disabled={!connectionId || busy === "files"} onClick={addFiles}>
            {busy === "files" ? "Reading files…" : "Add files…"}
          </Button>
          <Button size="sm" icon={Globe} disabled={!connectionId} onClick={() => setAdding("url")}>
            Add web page…
          </Button>
        </div>
      )}
      {error === SOURCE_CAP_MESSAGE ? !atCap && <TeamsNote /> : error && <Alert tone="danger">{error}</Alert>}
    </div>
  );
}

/** The source's kind as an icon, named for screen readers. */
function SourceIcon({ kind }: { kind: Kind }) {
  const Icon = ICONS[kind];
  return <Icon size={14} role="img" aria-label={KIND_NAMES[kind]} />;
}

/** The free app's limit, and where unlimited sources are: the first pointer to SayQL for teams. */
function TeamsNote({ children }: { children?: ReactNode }) {
  return (
    <p className={styles.muted}>
      <span>
        {children} {SOURCE_CAP_MESSAGE}{" "}
        <button type="button" className={styles.link} onClick={() => void openUrl(TEAMS_URL)}>
          SayQL for teams
        </button>
      </span>
    </p>
  );
}

/** A web page, and whether to read the pages in its folder too. */
function UrlForm({ connectionId, onAdded, onCancel }: { connectionId: string; onAdded: (s: KnowledgeSource) => void; onCancel: () => void }) {
  const [url, setUrl] = useState("");
  const [crawl, setCrawl] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const add = () => {
    setBusy(true);
    setError(null);
    commands
      .addKnowledgeUrl(connectionId, url.trim(), crawl)
      .then(onAdded, (e) => setError(errorText(e)))
      .finally(() => setBusy(false));
  };

  return (
    <form
      className={styles.form}
      aria-label="Add a web page"
      onSubmit={(e) => {
        e.preventDefault();
        if (url.trim() && !busy) add();
      }}
    >
      <Field label="Page address" hint="A public page about this database: docs, a wiki page, a data dictionary.">
        {(id) => <TextInput id={id} mono autoFocus value={url} placeholder="https://docs.acme.com/data/" onChange={(e) => setUrl(e.target.value)} />}
      </Field>
      <Checkbox label="Also read the pages in its folder on the same site (up to 50)" checked={crawl} onChange={setCrawl} />
      {error === SOURCE_CAP_MESSAGE ? <TeamsNote /> : error && <Alert tone="danger">{error}</Alert>}
      <div className={styles.add}>
        <Button size="sm" type="submit" variant="primary" disabled={!url.trim() || busy}>
          {busy ? "Reading…" : "Add web page"}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      <p className={styles.muted}>SayQL reads it from this computer, without cookies, and keeps its text here. It reads it again each time the audit runs.</p>
    </form>
  );
}

/** A git repository: its address, branch, a folder in it, and a token if git can't read it as you. */
function RepoForm({ connectionId, onAdded, onCancel }: { connectionId: string; onAdded: (s: KnowledgeSource) => void; onCancel: () => void }) {
  const [url, setUrl] = useState("");
  const [branch, setBranch] = useState("");
  const [subdir, setSubdir] = useState("");
  const [token, setToken] = useState("");
  const [tokenUser, setTokenUser] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const askUser = token.trim() !== "" && url.trim() !== "" && !knownTokenHost(url);

  const add = () => {
    setBusy(true);
    setError(null);
    commands
      .addKnowledgeRepo(connectionId, {
        url: url.trim(),
        branch: branch.trim(),
        subdir: subdir.trim(),
        token: token.trim() || null,
        tokenUser: askUser ? tokenUser.trim() || null : null,
      })
      .then(onAdded, (e) => setError(errorText(e)))
      .finally(() => setBusy(false));
  };

  return (
    <form
      className={styles.form}
      aria-label="Add a repository"
      onSubmit={(e) => {
        e.preventDefault();
        if (url.trim() && !busy) add();
      }}
    >
      <Field label="Repository address" hint="Its https:// or SSH address, as its Clone button shows it. GitHub, GitLab, Bitbucket or any git server.">
        {(id) => <TextInput id={id} mono autoFocus value={url} placeholder="https://github.com/acme/shop.git" onChange={(e) => setUrl(e.target.value)} />}
      </Field>
      <div className={styles.pair}>
        <Field label="Branch" optional>
          {(id) => <TextInput id={id} mono value={branch} placeholder="default branch" onChange={(e) => setBranch(e.target.value)} />}
        </Field>
        <Field label="Folder in the repository" optional>
          {(id) => <TextInput id={id} mono value={subdir} placeholder="services/api" onChange={(e) => setSubdir(e.target.value)} />}
        </Field>
      </div>
      <Field
        label="Read-only token"
        optional
        hint="Only if git can't read it with your own SSH key or credential helper. Kept in your keychain and sent only to this repository's server."
      >
        {(id) => <TextInput id={id} type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />}
      </Field>
      {askUser && (
        <Field label="User name for the token" hint="The user name this server expects with a token.">
          {(id) => <TextInput id={id} value={tokenUser} onChange={(e) => setTokenUser(e.target.value)} />}
        </Field>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
      <div className={styles.add}>
        <Button size="sm" type="submit" variant="primary" disabled={!url.trim() || busy || (askUser && !tokenUser.trim())}>
          {busy ? "Checking access…" : "Add repository"}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      <p className={styles.muted}>SayQL copies it with your own git, on this computer only, and fetches it each time the audit runs.</p>
    </form>
  );
}
