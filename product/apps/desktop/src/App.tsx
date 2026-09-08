import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { Virtuoso } from "react-virtuoso";
import {
  ArrowUp,
  Check,
  ChevronRight,
  Code2,
  FileCode2,
  FolderOpen,
  GitCompareArrows,
  House,
  LoaderCircle,
  MessageSquare,
  Plus,
  Settings2,
  ShieldCheck,
  Square,
  Sun,
  Moon,
  X,
  AlertCircle,
  Copy,
} from "lucide-react";
import { Dialog } from "@base-ui/react/dialog";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Skeleton } from "./components/ui/skeleton";
import { api, createCommandQueue, errorMessage, type Catalog, type View } from "./bridge";
import type { BridgeSnapshot, Command, JsonValue } from "./generated/protocol";
import { uploadMessage } from "./attachments";
import { ExtensionShortcuts } from "./ExtensionSettings";
import { navigateSession } from "./sessionNavigation";
import { ExtensionUI } from "./ExtensionUI";
import { NativeConversation } from "./NativeConversation";
import { NativeDetails, Blocks } from "./NativeDetails";
import { RecoveryPanel, type Recovery } from "./RecoveryPanel";
import { previewDocument } from "./preview";
const Editor = lazy(() => import("./Editor"));
type Page = "home" | "chat" | "files" | "diff" | "settings";
type FileView = { path: string; text: string; revision: string };
export default function App() {
  const [catalog, setCatalog] = useState<Catalog>({ projects: [], theme: "system" }),
    [view, setView] = useState<View>({ snapshot: null, error: null, progress: "" }),
    [page, setPage] = useState<Page>("home");
  const [path, setPath] = useState(""),
    [draft, setDraft] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [projectDialog, setProjectDialog] = useState(false),
    [quitDialog, setQuitDialog] = useState(false),
    [answer, setAnswer] = useState("");
  const [recovery, setRecovery] = useState<Recovery | null>(null);
  const [editorReset, setEditorReset] = useState(0);
  const [attachments, setAttachments] = useState<File[]>([]);
  const [requestPage, setRequestPage] = useState<{
      id: string;
      text: string;
      next: string | null;
    } | null>(null),
    [reviewedRequest, setReviewedRequest] = useState<string | null>(null);
  const requestDecoder = useRef(new TextDecoder());
  const [files, setFiles] = useState<string[]>([]),
    [file, setFile] = useState<FileView | null>(null),
    [edit, setEdit] = useState(""),
    [diff, setDiff] = useState(""),
    [models, setModels] = useState<{ id: string; profile: Record<string, unknown> }[]>([]),
    [sessions, setSessions] = useState<string[]>([]),
    [preview, setPreview] = useState(false);
  const [history, setHistory] = useState<Record<string, BridgeSnapshot>>({}),
    [viewer, setViewer] = useState<string | null>(null);
  const active = view.snapshot,
    snapshot = viewer ? history[viewer] : active,
    dirty = !!file && edit !== file.text;
  const dark =
    catalog.theme === "dark" ||
    (catalog.theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  const themeGeneration = useRef(0);
  const editRef = useRef(edit); editRef.current = edit;
  const activeRef = useRef(active);
  activeRef.current = active;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);
  useEffect(() => {
    const themeAtStart = themeGeneration.current;
    api
      .catalog()
      .then(next => setCatalog(current => themeGeneration.current === themeAtStart ? next : { ...next, theme: current.theme }))
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      let delay = active ? (view.details ? 16 : 150) : 1000;
      try {
        const next = await api.view();
        delay = next.snapshot ? (next.details ? 16 : 150) : 1000;
        if (!cancelled)
          setView((previous) =>
            JSON.stringify(previous) === JSON.stringify(next) ? previous : next,
          );
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) timer = setTimeout(poll, delay);
      }
    };
    void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // Opening a session starts its streaming cadence immediately. Empty-home
    // observation does not need a 60 Hz wakeup loop or overlapping requests.
  }, [!!active, !!view.details]);
  useEffect(() => {
    if (active)
      setHistory((h) =>
        Object.fromEntries(Object.entries({ ...h, [active.scope.session]: active }).slice(-32)),
      );
  }, [active]);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen("close-requested", () => setQuitDialog(true)))
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => unlisten?.();
  }, []);
  const queue = useRef(createCommandQueue());
  const actions = useRef(0);
  const run = useCallback(
    (command: Command) =>
      queue.current(command, (next) => {
        activeRef.current = next.snapshot;
        setView(next);
      }),
    [],
  );
  async function action(fn: () => Promise<unknown>) {
    actions.current++;
    setBusy(true);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      actions.current--;
      setBusy(actions.current > 0);
    }
  }

  async function openProject(project: string) {
    if (dirty) throw new Error("Save or discard your file changes before switching projects");
    let next: View;
    try {
      next = await api.open(project);
      setRecovery(null);
    } catch (error) {
      setRecovery(await api.recovery(project).catch(() => null));
      setProjectDialog(false);
      throw error;
    }
    setView(next);
    setViewer(null);
    setPage("chat");
    setFile(null);
    setFiles([]);
    setProjectDialog(false);
    const nextCatalog = await api.catalog();
    setCatalog(current => ({ ...nextCatalog, theme: current.theme }));
  }
  async function loadFiles() {
    setFiles((await run({ kind: "files", action: "list", args: {} })) as string[]);
  }
  async function readFile(name: string) {
    if (dirty) throw new Error("Save or discard your changes before opening another file");
    const data = (await run({ kind: "files", action: "read", args: { path: name } })) as FileView;
    setFile(data);
    setEdit(data.text);
  }
  async function save() {
    if (!file) return;
    const submitted = edit;
    const data = (await run({
      kind: "files",
      action: "save",
      args: { path: file.path, text: edit, revision: file.revision },
    })) as FileView;
    setFile(data);
    if (editRef.current === submitted) setEdit(data.text);
  }
  useEffect(() => {
    function key(e: KeyboardEvent) {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (dirtyRef.current) void action(save);
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        setProjectDialog(true);
      }
    }
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  });
  async function navigate(next: Page) {
    setPage(next);
    if (!active) return;
    if (next === "files") await loadFiles();
    if (next === "diff") {
      const d = (await run({ kind: "files", action: "diff", args: {} })) as { view: string };
      setDiff(d.view);
    }
    if (next === "settings") {
      const data = (await run({ kind: "model", action: "status", args: {} })) as {
        models: typeof models;
      };
      setModels(data.models);
      setSessions((await run({ kind: "session", action: "list", args: {} })) as string[]);
    }
  }
  const pending = active?.pending[0];
  async function readRequest() {
    if (!pending) return;
    const continuing = requestPage?.id === pending.binding.id && requestPage.next !== null;
    const offset = continuing ? requestPage!.next! : "0";
    if (!continuing) requestDecoder.current = new TextDecoder();
    const data = (await run({
      kind: "artifact_read",
      artifact: pending.evidence,
      offset,
      limit: 32768,
    })) as { base64: string; next: string | null };
    const text = requestDecoder.current.decode(
      Uint8Array.from(atob(data.base64), (c) => c.charCodeAt(0)),
      { stream: data.next !== null },
    );
    setRequestPage({ id: pending.binding.id, text, next: data.next });
    if (data.next === null) setReviewedRequest(pending.binding.id);
  }
  const executing = ["running", "streaming", "awaiting_input"].includes(active?.status ?? "");
  return (
    <div className="shell">
      <aside className="rail">
        <button className="brand" aria-label="AG1 home" onClick={() => setPage("home")}>
          a<span>1</span>
        </button>
        <nav>
          {(
            [
              ["home", House, "Home"],
              ["chat", MessageSquare, "Chat"],
              ["files", FileCode2, "Files"],
              ["diff", GitCompareArrows, "Changes"],
            ] as const
          ).map(([key, Icon, label]) => (
            <button
              key={key}
              title={label}
              aria-label={label}
              aria-current={page === key ? "page" : undefined}
              onClick={() => void action(() => navigate(key))}
            >
              <Icon size={21} />
              <span>{label}</span>
            </button>
          ))}
        </nav>
        <button
          className={page === "settings" ? "selected" : ""}
          aria-label="Settings"
          onClick={() => void action(() => navigate("settings"))}
        >
          <Settings2 size={21} />
          <span>Settings</span>
        </button>
        <button
          aria-label="Toggle theme"
          onClick={() => {
            themeGeneration.current++;
            const theme = dark ? "light" : "dark";
            setCatalog({ ...catalog, theme });
            void api.theme(theme);
          }}
        >
          {dark ? <Sun size={19} /> : <Moon size={19} />}
        </button>
      </aside>
      <div className="main">
        <header className="titlebar">
          <div className="breadcrumb">
            <span>AG1</span>
            <ChevronRight size={13} />
            <button onClick={() => setProjectDialog(true)}>
              {active?.workspace_path.split("/").at(-1) ?? "Your workspace"}
            </button>
          </div>
          <div className="status">
            <span className={"dot " + (executing ? "working" : "")} />
            {active?.fixture_provider
              ? "Fixture provider"
              : executing
                ? "Working"
                : active
                  ? "Ready"
                  : "Local workspace"}
            <span className="key">Ctrl O</span>
          </div>
        </header>
        {(error || view.error) && (
          <div role="alert" className="error">
            <AlertCircle size={17} />
            <span>{error || view.error}</span>
            <button aria-label="Dismiss error" onClick={() => setError("")}>
              <X size={16} />
            </button>
          </div>
        )}
        {recovery && <RecoveryPanel record={recovery} onClose={() => setRecovery(null)} />}
        {view.details && <ExtensionShortcuts key={`shortcuts/${active?.scope.session}`} run={run} />}
        {view.details && <ExtensionUI activity={view.details.activity} run={run} key={`extension-ui/${active?.scope.session}`} onEditor={(text, paste) => setDraft(current => paste ? current + text : text)} />}
        {view.details && active && <NativeDetails details={view.details} status={active.status} model={active.model} run={run} onEditor={text => setDraft(current => current === draft ? text : current)} />}
        {page === "home" ? (
          <div className="home">
            <div className="home-intro">
              <div className="eyebrow">
                <ShieldCheck size={15} /> YOUR MODELS. YOUR WORKSPACE.
              </div>
              <h1>What will you work on?</h1>
              <p>Open a project and pick up where you left off.</p>
            </div>
            <button className="open-card" onClick={() => setProjectDialog(true)}>
              <div className="folder-icon">
                <FolderOpen size={27} />
              </div>
              <div>
                <strong>Open a project</strong>
                <p>Choose a folder on this computer</p>
              </div>
              <Plus size={20} />
            </button>
            <div className="section-label">
              RECENT PROJECTS <span>{catalog.projects.length}</span>
            </div>
            <div className="recent">
              {catalog.projects.length ? (
                catalog.projects.map((p) => (
                  <button key={p} onClick={() => void action(() => openProject(p))}>
                    <FolderOpen size={18} />
                    <div>
                      <strong>{p.split("/").at(-1)}</strong>
                      <span>{p}</span>
                    </div>
                    <ChevronRight size={16} />
                  </button>
                ))
              ) : (
                <div className="empty-recent">Your projects will appear here.</div>
              )}
            </div>
            <div className="home-note">
              <Code2 size={16} />
              <span>Local tools, durable conversations, and changes you can review.</span>
            </div>
          </div>
        ) : !snapshot && page !== "settings" ? (
          <div className="empty">
            <FolderOpen size={35} />
            <h2>Start with a project</h2>
            <p>Open a folder to chat, browse files, and review changes.</p>
            <Button onClick={() => setProjectDialog(true)}>Open project</Button>
          </div>
        ) : page === "chat" ? (
          <div className="chat-layout">
            <section className="chat">
              <div className="tabs">
                {Object.values(history)
                  .slice(-6)
                  .map((s) => (
                    <button
                      key={s.scope.session}
                      className={snapshot?.scope.session === s.scope.session ? "active" : ""}
                      onClick={() =>
                        setViewer(
                          s.scope.session === active?.scope.session ? null : s.scope.session,
                        )
                      }
                    >
                      <MessageSquare size={14} />
                      {s.messages.find((m) => m.role === "user")?.text.slice(0, 24) ??
                        "New conversation"}
                      {s.scope.session !== active?.scope.session && (
                        <span className="tab-tag">Saved</span>
                      )}
                    </button>
                  ))}
                <button
                  aria-label="New conversation"
                  disabled={executing || busy}
                  onClick={() =>
                    void action(async () => {
                      await run({ kind: "session", action: "new", args: {} });
                      setViewer(null);
                    })
                  }
                >
                  <Plus size={15} />
                </button>
              </div>
              {viewer && (
                <div className="viewer-banner">
                  Viewing a saved snapshot. Return to the active tab to send a message.
                </div>
              )}
              {view.details && !viewer && snapshot ? <NativeConversation key={`${snapshot.scope.workspace}/${snapshot.scope.session}/${snapshot.scope.epoch}`} details={view.details} snapshot={snapshot} run={run} /> : snapshot?.messages.length ? (
                <Virtuoso
                  className="messages"
                  data={snapshot.messages}
                  followOutput="smooth"
                  itemContent={(_i, m) => (
                    <article className={"message " + m.role}>
                      <div className="message-avatar">
                        {m.role === "user" ? (
                          "You"
                        ) : m.role === "toolResult" ? (
                          <Code2 size={16} />
                        ) : (
                          "a1"
                        )}
                      </div>
                      <div className="message-body">
                        <strong>
                          {m.role === "user"
                            ? "You"
                            : m.role === "toolResult"
                              ? "Tool result"
                              : "AG1"}
                        </strong>
                        {view.details?.messages.find(message => message.id === m.id) ? <Blocks message={view.details.messages.find(message => message.id === m.id)!} /> : <pre>{m.text}</pre>}
                        <button
                          className="evidence-button"
                          onClick={() =>
                            void action(async () => {
                              const data = (await run(view.details ? { kind: "session", action: "entry", args: { id: m.id, offset: "0" } } : {
                                kind: "artifact_read",
                                artifact: m.evidence,
                                offset: "0",
                                limit: 32768,
                              })) as { base64: string; next: string | null };
                              setDiff(
                                new TextDecoder().decode(
                                  Uint8Array.from(atob(data.base64), (c) => c.charCodeAt(0)),
                                ) +
                                  (data.next
                                    ? "\n[More evidence is available through paging.]"
                                    : ""),
                              );
                              setPage("diff");
                            })
                          }
                        >
                          View evidence
                        </button>
                      </div>
                    </article>
                  )}
                />
              ) : (
                <div className="chat-welcome">
                  <span className="welcome-mark">a1</span>
                  <h2>{executing ? "Working on your request…" : "Ready when you are."}</h2>
                  <p>{executing ? "Your request is in progress." : "Ask a question, explore the code, or describe a change."}</p>
                  <div className="suggestions">
                    {[
                      "Explain this project",
                      "Find the main entry point",
                      "Review recent changes",
                    ].map((s) => (
                      <button key={s} onClick={() => setDraft(s)}>
                        {s}
                        <ChevronRight size={14} />
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {executing && (
                <div className="streaming">
                  <LoaderCircle size={14} className="spin" />
                  <pre>{view.progress || "AG1 is working…"}</pre>
                </div>
              )}
              <div className="composer-area">
                {view.details && <label className="attachment-picker">Attach files<input aria-label="Attach files" type="file" multiple onChange={e => setAttachments(Array.from(e.target.files ?? []))} />{attachments.map(file => <span key={file.name}>{file.name} </span>)}</label>}
                <form
                  className="composer"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const text = draft;
                    void action(async () => {
                      if (viewer) throw new Error("Saved tabs are snapshot viewers");
                      if (text.startsWith("/") && attachments.length) throw new Error("Send attachments as a message before running a slash action");
                      const input = view.details ? await uploadMessage(text, attachments, run) : { kind: "inline" as const, text };
                      await run(
                        view.details && text.startsWith("/")
                          ? { kind: "runtime", action: "client_action", args: { text } }
                          : executing
                          ? { kind: "session", action: "steer", args: view.details ? { input } : { text } }
                          : { kind: "prompt", input },
                      );
                      setDraft("");
                      setAttachments([]);
                    });
                  }}
                >
                  <textarea
                    aria-label="Message AG1"
                    placeholder={
                      executing
                        ? "Add a direction while AG1 works…"
                        : "Ask AG1 to work on your project…"
                    }
                    value={draft}
                    disabled={!!viewer}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        e.currentTarget.form?.requestSubmit();
                      }
                    }}
                  />
                  <div className="composer-toolbar">
                    <button
                      type="button"
                      className="model-label"
                      onClick={() => void action(() => navigate("settings"))}
                    >
                      <span className="dot" />
                      {snapshot?.model}
                      <ChevronRight size={12} />
                    </button>
                    <div>
                      {executing && active?.active_operation && (
                        <Button
                          type="button"
                          variant="ghost"
                          aria-label="Stop generation"
                          onClick={() =>
                            void action(() =>
                              run({ kind: "cancel", operation: active.active_operation! }),
                            )
                          }
                        >
                          <Square size={15} />
                        </Button>
                      )}
                      <Button
                        type="submit"
                        size="icon"
                        aria-label={executing ? "Send steering" : "Send message"}
                        disabled={(!draft.trim() && !attachments.length) || busy || !!viewer}
                      >
                        <ArrowUp size={18} />
                      </Button>
                    </div>
                  </div>
                </form>
                <div className="composer-note">
                  Review tool requests before allowing changes.{" "}
                  <span>Enter to send · Shift Enter for a new line</span>
                </div>
              </div>
            </section>
            <aside className="context-panel">
              <div className="panel-heading">Workspace context</div>
              <FolderOpen size={25} />
              <h3>{snapshot?.workspace_path.split("/").at(-1)}</h3>
              <p className="path">{snapshot?.workspace_path}</p>
              <div className="context-item">
                <MessageSquare size={15} />
                <span>{snapshot?.message_count ?? 0} messages</span>
              </div>
              <div className="context-item">
                <ShieldCheck size={15} />
                <span>Changes require review</span>
              </div>
              <button className="context-link" onClick={() => void action(() => navigate("files"))}>
                Browse project files <ChevronRight size={14} />
              </button>
              {snapshot?.diagnostic && <div className="diagnostic">{snapshot.diagnostic}</div>}
            </aside>
          </div>
        ) : page === "files" ? (
          <div className="files-layout">
            <aside className="file-tree">
              <div className="panel-heading">
                PROJECT FILES{" "}
                <button aria-label="Refresh files" onClick={() => void action(loadFiles)}>
                  ↻
                </button>
              </div>
              {files.map((name) => (
                <button
                  className={file?.path === name ? "active" : ""}
                  key={name}
                  onClick={() => void action(() => readFile(name))}
                >
                  <FileCode2 size={14} />
                  {name}
                </button>
              ))}
            </aside>
            <section className="file-pane">
              <div className="file-toolbar">
                <span>
                  {file?.path ?? "Select a file"}
                  {dirty && (
                    <span className="dirty" aria-label="Unsaved changes">
                      ●
                    </span>
                  )}
                </span>
                <div>
                  {dirty && (
                    <Button
                      variant="ghost"
                      onClick={() => {
                        setEdit(file!.text);
                        setEditorReset((n) => n + 1);
                      }}
                    >
                      Discard
                    </Button>
                  )}
                  <Button variant="outline" disabled={!file} onClick={() => setPreview(!preview)}>
                    {preview ? "Editor" : "Static preview"}
                  </Button>
                  <Button disabled={!dirty || busy || executing} onClick={() => void action(save)}>
                    <Check size={14} /> Save
                  </Button>
                </div>
              </div>
              {file ? (
                <Suspense
                  fallback={
                    <div className="editor-loading" aria-label="Loading editor">
                      <Skeleton className="h-5 w-2/3" />
                      <Skeleton className="h-5 w-1/2" />
                      <p>Loading editor…</p>
                    </div>
                  }
                >
                  {preview ? (
                    <iframe
                      title="Isolated static preview"
                      sandbox=""
                      referrerPolicy="no-referrer"
                      srcDoc={previewDocument(edit)}
                    />
                  ) : (
                    <Editor
                      text={edit}
                      path={file.path}
                      dark={dark}
                      resetKey={file.revision + editorReset}
                      onChange={setEdit}
                    />
                  )}
                </Suspense>
              ) : (
                <div className="empty">
                  <FileCode2 size={30} />
                  <h2>Your project files</h2>
                  <p>Select a file from the sidebar.</p>
                </div>
              )}
            </section>
          </div>
        ) : page === "diff" ? (
          <section className="diff-page">
            <div className="page-heading">
              <div>
                <h2>Review changes</h2>
                <p>Current workspace changes and captured evidence.</p>
              </div>
              <Button variant="outline" onClick={() => void action(() => navigate("diff"))}>
                Refresh
              </Button>
            </div>
            <pre className="diff-content">{diff || "No changes loaded."}</pre>
          </section>
        ) : (
          <section className="settings-page">
            <div className="page-heading">
              <div>
                <h2>Settings</h2>
                <p>Choose how AG1 works in this workspace.</p>
              </div>
            </div>
            <div className="settings-section">
              <h3>Appearance</h3>
              <div className="theme-options">
                {(["light", "dark", "system"] as const).map((theme) => (
                  <button
                    aria-pressed={catalog.theme === theme}
                    className={"theme-card " + theme}
                    key={theme}
                    onClick={() => {
                      themeGeneration.current++;
                      setCatalog({ ...catalog, theme });
                      void api.theme(theme);
                    }}
                  >
                    <div className="theme-swatch">
                      <i />
                      <b />
                    </div>
                    <span>{theme}</span>
                    {catalog.theme === theme && <Check size={14} />}
                  </button>
                ))}
              </div>
            </div>
            <div className="settings-section">
              <h3>Local models</h3>
              <p>Your existing llama.cpp builds and model settings are preserved.</p>
              {!active ? (
                <Button variant="outline" onClick={() => setProjectDialog(true)}>
                  Open a project to select a model
                </Button>
              ) : (
                models.map((model) => (
                  <div className="model-card" key={model.id}>
                    <div>
                      <strong>{model.id}</strong>
                      <p>
                        {String(model.profile.contextWindow)} context ·{" "}
                        {view.details ? String(model.profile.provider) : `${String(model.profile.cacheTypeK)} KV cache`}
                      </p>
                      <code>{view.details ? JSON.stringify(model.profile.runtime ?? "Host-configured endpoint") : String(model.profile.serverPath)}</code>{!!model.profile.conflict && <p role="alert">Configuration conflict: {String(model.profile.conflict)}</p>}
                    </div>
                    <Button
                      variant={active.model === model.id ? "secondary" : "outline"}
                      disabled={executing || busy || !!model.profile.conflict}
                      onClick={() =>
                        void action(() =>
                          run({ kind: "model", action: "select", args: view.details ? { provider: String(model.profile.provider), model_id: model.id } : { id: model.id } }),
                        )
                      }
                    >
                      {active.model === model.id ? (
                        <>
                          <Check size={14} /> Selected
                        </>
                      ) : (
                        "Select"
                      )}
                    </Button>
                  </div>
                ))
              )}
              <p className="settings-hint">
                Selecting a model does not start it. Loading begins when you send a message.
                Occupied endpoints are reported as conflicts.
              </p>
            </div>
            {active && (
              <div className="settings-section">
                <h3>Conversations</h3>
                <Button
                  variant="outline"
                  disabled={executing || busy || !active.message_count}
                  onClick={() =>
                    void action(() => view.details ? navigateSession(run, "fork", { id: [...active.messages].reverse().find(m => m.role === "user")?.id ?? "", role: "user" }, false, text => setDraft(current => current === draft ? text : current)) : run({ kind: "session", action: "fork", args: {} }))
                  }
                >
                  <Copy size={14} /> Fork current conversation
                </Button>
                {sessions.map((name) => (
                  <button
                    className="session-link"
                    key={name}
                    disabled={executing || busy}
                    onClick={() =>
                      void action(() =>
                        run({ kind: "session", action: "resume", args: view.details ? { session: name } : { file: name } }),
                      )
                    }
                  >
                    {name.split("/").at(-1)}
                  </button>
                ))}
              </div>
            )}
            <div className="settings-section">
              <Button variant="outline" onClick={() => setQuitDialog(true)}>
                Quit AG1
              </Button>
            </div>
          </section>
        )}
        <footer>
          <span>
            <ShieldCheck size={12} /> Local workspace
          </span>
          <span>
            {busy ? (
              <>
                <LoaderCircle size={12} className="spin" /> Working…
              </>
            ) : dirty ? (
              "Unsaved file changes"
            ) : active?.status === "idle" ? (
              "All caught up"
            ) : (
              (active?.status?.replaceAll("_", " ") ?? "Ready")
            )}{" "}
          </span>
        </footer>
      </div>
      <Dialog.Root open={projectDialog} onOpenChange={setProjectDialog}>
        <Dialog.Portal>
          <Dialog.Backdrop className="dialog-backdrop" />
          <Dialog.Popup className="dialog">
            <Dialog.Title>Open a project</Dialog.Title>
            <Dialog.Description>Enter the folder path for your workspace.</Dialog.Description>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void action(() => openProject(path));
              }}
            >
              <Input
                aria-label="Project folder"
                placeholder="/home/you/projects/my-project"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                autoFocus
              />
              <div className="dialog-actions">
                <Button type="button" variant="ghost" onClick={() => setProjectDialog(false)}>
                  Cancel
                </Button>
                <Button type="submit" disabled={busy || !path.trim()}>
                  {busy ? <LoaderCircle className="spin" size={15} /> : <FolderOpen size={15} />}{" "}
                  Open project
                </Button>
              </div>
            </form>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>
      <Dialog.Root open={!!pending} onOpenChange={() => {}}>
        <Dialog.Portal>
          <Dialog.Backdrop className="dialog-backdrop" />
          <Dialog.Popup className="dialog approval-dialog">
            <div className="approval-icon">
              <ShieldCheck size={25} />
            </div>
            <Dialog.Title>
              {pending?.kind === "approval" ? "Review tool request" : "AG1 needs your input"}
            </Dialog.Title>
            <Dialog.Description>Only this operation will use your answer.</Dialog.Description>
            <pre>{pending?.title}</pre>
            {pending?.truncated && (
              <div>
                <p>This summary is shortened. Review the complete request before allowing it.</p>
                {requestPage?.id === pending.binding.id && <pre>{requestPage.text}</pre>}
                <Button variant="outline" disabled={busy} onClick={() => void action(readRequest)}>
                  {requestPage?.id === pending.binding.id && requestPage.next
                    ? "Next request page"
                    : "Read complete request"}
                </Button>
              </div>
            )}
            {pending?.kind === "select" ? (
              <select
                aria-label="Answer"
                value={answer}
                onChange={(e) => setAnswer(e.target.value)}
              >
                <option value="">Choose an option</option>
                {pending.options.map((option) => (
                  <option key={option} value={option}>
                    {option}
                  </option>
                ))}
              </select>
            ) : (
              pending?.kind !== "approval" && (
                <Input
                  aria-label="Answer"
                  value={answer}
                  onChange={(e) => setAnswer(e.target.value)}
                />
              )
            )}
            <div className="dialog-actions">
              <Button
                variant="outline"
                onClick={() =>
                  pending &&
                  void action(() =>
                    run({ kind: "approve", binding: pending.binding, allow: false }),
                  )
                }
              >
                Deny
              </Button>
              <Button
                disabled={
                  busy ||
                  !!(pending?.truncated && reviewedRequest !== pending.binding.id) ||
                  !!(pending?.kind === "select" && !pending.options.includes(answer))
                }
                onClick={() =>
                  pending &&
                  void action(async () => {
                    await run(
                      pending.kind === "approval"
                        ? { kind: "approve", binding: pending.binding, allow: true }
                        : {
                            kind: "session",
                            action: "answer",
                            args: { binding: pending.binding, answer } as unknown as JsonValue,
                          },
                    );
                    setAnswer("");
                  })
                }
              >
                {pending?.kind === "approval" ? "Allow this operation" : "Submit answer"}
              </Button>
            </div>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>
      <Dialog.Root open={quitDialog} onOpenChange={setQuitDialog}>
        <Dialog.Portal>
          <Dialog.Backdrop className="dialog-backdrop" />
          <Dialog.Popup className="dialog">
            <Dialog.Title>Quit AG1?</Dialog.Title>
            <Dialog.Description>
              {executing
                ? "The active session will be cancelled and its saved state retained."
                : "Your saved conversations will be available next time."}
              {dirty ? " Unsaved editor changes will be lost." : ""}
            </Dialog.Description>
            <div className="dialog-actions">
              <Button variant="ghost" onClick={() => setQuitDialog(false)}>
                Keep working
              </Button>
              <Button onClick={() => void action(api.quit)}>Quit</Button>
            </div>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
