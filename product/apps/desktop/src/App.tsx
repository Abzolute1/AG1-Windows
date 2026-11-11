import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { Dialog } from "@base-ui/react/dialog";
import { ArrowUp, Check, ChevronDown, FileCode2, FolderOpen, GitCompareArrows, Github, LoaderCircle, MessageSquare, MoreHorizontal, PanelRightOpen, PanelLeftClose, Plus, Search, Settings2, Square, X } from "lucide-react";
import { api, createCommandQueue, errorMessage, type Catalog, type Chat, type Model, type View, type ConversationStatus } from "./bridge";
import type { Command, JsonValue, Scope } from "./generated/protocol";
import { NativeConversation } from "./NativeConversation";
import { NativeDetails } from "./NativeDetails";
import { ReviewPanel, restoreReviewFocus, type ReviewTab } from "./ReviewPanel";
import { GitHubReviewContext, type GitHubReviewTarget } from "./GitHubPullReview";
import { Markdown } from "./Markdown";
import { ApprovalReview, approvalExpired } from "./ApprovalReview";
import { ChatFolders, type FolderSelection, type FolderChange } from "./ChatFolders";
import { ResourcesPane } from "./ResourcesPane";
import { ExtensionShortcuts } from "./ExtensionSettings";
import { ExtensionUI } from "./ExtensionUI";
import { DraftAttachment } from "./ImageAttachment";
import { uploadMessage, validateAttachments } from "./attachments";
import { RecoveryPanel, type Recovery } from "./RecoveryPanel";
import { applyTheme, type Theme } from "./themes";
import { previewDocument } from "./preview";
import { ModelLoadControl, modelPreparing, type ModelPreparation, type PendingModelLoad } from "./ModelLoadControl";
import { ModelPicker } from "./ModelPicker";
import { Settings } from "./Settings";
import { observeScrollTiming } from "./scrollTiming";
import { MCPManager } from "./MCPManager";
import { usePreferences, applyPreferences } from "./preferences";
import { ApprovalIcon, ApprovalChoices, approvalModes, type ApprovalMode } from "./ApprovalMenu";
const Editor = lazy(() => import("./Editor"));
type Page = "chat" | "files" | "settings" | "mcp";
type FileView = { path: string; text: string; revision: string };
const safeApproval = (mode: string | null | undefined): ApprovalMode => mode === "autonomous" ? "autonomous" : "interactive";
const savedApproval = () => safeApproval(window.localStorage?.getItem("ag1.approval"));
const normalizedPath = (path: string) => path.replace(/\\/g, "/");
const basename = (path: string) => normalizedPath(path).replace(/\/+$/, "").split("/").at(-1) || path;
const folderFromAccessError = (message: string) => message.match(/Folder (?:is read-only|access is not granted for):?\s*(.+?)(?:\. Ask the user|$)/i)?.[1]?.trim() || null;
const formatTime = (ms: number) => ms < 60000 ? `${Math.floor(ms / 1000)}s` : `${Math.floor(ms / 60000)}m ${Math.floor(ms / 1000) % 60}s`;
function mergeView(old: View, next: View): View {
  const a = old.details?.activity as any, b = next.details?.activity as any;
  if (old.snapshot && next.snapshot && next.details && ["streaming", "running", "awaiting_input"].includes(next.snapshot.status) && JSON.stringify(old.snapshot.scope) === JSON.stringify(next.snapshot.scope) && (a?.live_revision ?? 0) > (b?.live_revision ?? 0)) next = { ...next, details: { ...next.details, activity: { ...b, live_revision: a.live_revision, observations: a.observations } } };
  return JSON.stringify(old) === JSON.stringify(next) ? old : next;
}
function Modal({ open, close, title, children, className = "" }: { open: boolean; close: () => void; title: string; children: React.ReactNode; className?: string }) {
  return <Dialog.Root open={open} onOpenChange={v => { if (!v) close(); }}><Dialog.Portal><Dialog.Backdrop className="dialog-backdrop" /><Dialog.Popup className={`dialog ${className}`}><div className="dialog-heading"><Dialog.Title>{title}</Dialog.Title><button aria-label={`Close ${title}`} onClick={close}><X size={18} /></button></div>{children}</Dialog.Popup></Dialog.Portal></Dialog.Root>;
}
export default function App() {
  useEffect(() => observeScrollTiming(document), []);
  const [foldersDialog, setFoldersDialog] = useState(false);
  const [conversationStatuses, setConversationStatuses] = useState<ConversationStatus[]>([]);
  const drafts = useRef(new Map<string, { text: string; files: File[]; plan: boolean; model: string }>());
  const [catalog, setCatalog] = useState<Catalog>({ projects: [], theme: "dark", chats: [] });
  const [view, setView] = useState<View>({ snapshot: null, progress: "", error: null });
  const [page, setPage] = useState<Page>("chat"), [draft, setDraft] = useState(""), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [models, setModels] = useState<Model[]>([]), [selected, setSelected] = useState(() => window.localStorage?.getItem("ag1.model") || "");
  const [preferences, updatePreferences] = usePreferences();
  const [approvalMenu, setApprovalMenu] = useState(false), [approval, setApproval] = useState<string>(savedApproval), [defaultApproval, setDefaultApproval] = useState("interactive");
  const menus = useRef<HTMLDivElement>(null);
  const [reasoning, setReasoning] = useState<Record<string, string | undefined>>({});
  const [deleting, setDeleting] = useState<Chat | null>(null);
  const [settingsSection, setSettingsSection] = useState("appearance");
  const [search, setSearch] = useState(""), [sidebar, setSidebar] = useState(true), [archived, setArchived] = useState(false);
  const [projectDialog, setProjectDialog] = useState(false), [path, setPath] = useState(""), [quitDialog, setQuitDialog] = useState(false), [plugins, setPlugins] = useState(false), [detailsOpen, setDetailsOpen] = useState(false), [requestedFolder, setRequestedFolder] = useState<string | undefined>();
  const [addMenu, setAddMenu] = useState(false), [modelMenu, setModelMenu] = useState(false), [plan, setPlan] = useState(false), [attachments, setAttachments] = useState<File[]>([]);
  const [renaming, setRenaming] = useState<Chat | null>(null), [newTitle, setNewTitle] = useState("");
  const [recovery, setRecovery] = useState<Recovery | null>(null), [answer, setAnswer] = useState("");
  const [requestPage, setRequestPage] = useState<{ id: string; text: string; next: string | null } | null>(null), [reviewed, setReviewed] = useState<string | null>(null);
  const [files, setFiles] = useState<string[]>([]), [file, setFile] = useState<FileView | null>(null), [edit, setEdit] = useState(""), [preview, setPreview] = useState(false), [editorReset, setEditorReset] = useState(0);
  const [modelLoad, setModelLoad] = useState<PendingModelLoad | null>(null);
  const [stopping, setStopping] = useState(false);
  const [review, setReview] = useState<{ scope: string; tab: ReviewTab; open: boolean; turn?: string; artifactTurn?: string; github?: GitHubReviewTarget } | null>(null);
  const reviewOpener = useRef<HTMLElement | null>(null);
  const [navigating, setNavigating] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState<{ at: number; scope: string; model?: string; ended?: number } | null>(null), [elapsed, setElapsed] = useState(0);
  const active = view.snapshot, executing = !!active && ["streaming", "running", "awaiting_input"].includes(active.status), stoppable = !!active?.active_operation && (executing || !!view.error || active.status !== "idle"), dirty = !!file && edit !== file.text;
  const activeRef = useRef(active); activeRef.current = active;
  const catalogRef = useRef(catalog); catalogRef.current = catalog;
  const indexedWorkspaces = useRef(new Set<string>()), catalogEpoch = useRef(0);
  const editRef = useRef(edit); editRef.current = edit;
  const changing = useRef(false), actions = useRef(0), themeVersion = useRef(0), queue = useRef(createCommandQueue());
  const fileInput = useRef<HTMLInputElement>(null), folderInput = useRef<HTMLInputElement>(null), requestDecoder = useRef(new TextDecoder());
  const viewEpoch = useRef(0);
  const run = useCallback((command: Command, expectedScope?: Scope) => { viewEpoch.current++; return queue.current(command, next => { viewEpoch.current++; activeRef.current = next.snapshot; setView(old => mergeView(old, next)); }, expectedScope); }, []);
  const queryScope = active ? JSON.stringify(active.scope) : "";
  // Read-only display requests stay bound to the mounted conversation.
  const query = useCallback(async (command: Command) => { const s = activeRef.current; if (!s || JSON.stringify(s.scope) !== queryScope) throw new Error("The conversation changed before this request could start"); return (await api.command(s, command)).data; }, [queryScope]);
  const mcpRun = useCallback((args: JsonValue) => { if (!active) return Promise.reject(new Error("Open a chat first")); return run({ kind: "runtime", action: "mcp_manager", args }, active.scope); }, [run, queryScope]);
  const scope = active ? `${active.workspace_path}/${active.scope.session}` : "";
  const general = !!active && normalizedPath(active.workspace_path).endsWith("/general-chat");
  const folderSelection = (view.details?.context as any)?.folder_access as FolderSelection | undefined;
  const workingFolder = folderSelection?.primary || active?.workspace_path;
  const accessError = folderFromAccessError(error || view.error || "");
  const currentChat = catalog.chats?.find(c => c.workspace === active?.workspace_path && c.session === active?.scope.session);
  const observation = (view.details?.activity as any)?.observations;
  const usage = (view.details?.activity as any)?.latest_assistant_usage?.usage;
  const preparation = (view.details?.activity as any)?.runtime_preparation as ModelPreparation | undefined;
  const status = stopping ? "Stopping response…" : active?.pending.length ? "Waiting for you" : !executing ? "" : (view.details?.activity as any)?.inference_waiting ? "Queued · waiting for model" : observation?.phase === "thinking" ? "Thinking" : observation?.phase === "generating" ? "Generating" : observation?.phase === "tool" ? "Generating operation" : observation?.phase === "executing" ? "Running tools" : preparation?.phase === "verifying" ? `Verifying ${preparation.model} files` : preparation?.phase === "loading" ? `Loading ${preparation.model}` : observation?.phase === "preparing" ? "Preparing request" : observation?.phase === "waiting" ? "Waiting for model" : observation?.phase === "text" ? "Generating" : "Processing prompt";
  useEffect(() => { const generation = themeVersion.current; void api.catalog().then(next => setCatalog(old => ({ ...next, theme: generation === themeVersion.current ? next.theme : old.theme }))).catch(e => setError(errorMessage(e))); void api.preferences().then(p => { if (p?.approval_mode) setDefaultApproval(p.approval_mode); }).catch(() => {}); void api.models().then(next => { if (Array.isArray(next)) setModels(next); }).catch(e => setError(errorMessage(e))); }, []);
  useEffect(() => { applyTheme(catalog.theme, preferences.chatSize, catalog.themes); applyPreferences(preferences); const media = matchMedia("(prefers-color-scheme: dark)"); const update = () => { applyTheme(catalog.theme, preferences.chatSize, catalog.themes); applyPreferences(preferences); }; media.addEventListener?.("change", update); return () => media.removeEventListener?.("change", update); }, [catalog.theme, catalog.themes, preferences]);
  useEffect(() => {
    let stopped = false; let timer: ReturnType<typeof setTimeout>; let viewFrame = 0;
    const poll = async () => { let delay = 1000; try { if (!changing.current) { const epoch = viewEpoch.current; const next = await api.view(); delay = document.hidden ? 3000 : modelPreparing((next.details?.activity as any)?.runtime_preparation) ? 400 : next.snapshot && ["streaming", "running", "awaiting_input"].includes(next.snapshot.status) ? 400 : next.snapshot ? 1500 : 1000; if (!stopped && !changing.current && epoch === viewEpoch.current) { cancelAnimationFrame(viewFrame); viewFrame = requestAnimationFrame(() => { if (!stopped && !changing.current && epoch === viewEpoch.current) setView(old => mergeView(old, next)); }); } } } catch (e) { if (!stopped) setError(errorMessage(e)); } finally { if (!stopped) timer = setTimeout(poll, delay); } };
    void poll(); return () => { stopped = true; clearTimeout(timer); cancelAnimationFrame(viewFrame); };
  }, []);
  useEffect(() => {
    if (!executing || !view.details) return;
    let stopped = false, revision = 0, frame = 0, releaseFrame: (() => void) | undefined;
    const consume = async () => {
      while (!stopped) {
        try {
          const live = await api.live(revision);
          if (stopped) break;
          if (!live?.scope || typeof live.revision !== "number") break;
          revision = live.revision;
          await new Promise<void>(resolve => { releaseFrame = resolve; frame = requestAnimationFrame(() => {
            if (!stopped) setView(old => old.snapshot && old.details && JSON.stringify(old.snapshot.scope) === JSON.stringify(live.scope) ? { ...old, details: { ...old.details, activity: { ...(old.details.activity as Record<string, JsonValue>), observations: live.observations, live_revision: live.revision } } } : old);
            resolve();
          }); });
        } catch { break; }
      }
    };
    void consume(); return () => { stopped = true; cancelAnimationFrame(frame); releaseFrame?.(); };
  }, [scope, executing, !!view.details]);
  useEffect(() => {
    if (!view.details) return;
    let closed = false, timer: ReturnType<typeof setTimeout>;
    const poll = async () => { try { const rows = await api.conversationStatuses(); if (!closed && Array.isArray(rows)) setConversationStatuses(old => JSON.stringify(old) === JSON.stringify(rows) ? old : rows); } catch { /* Existing chat remains available during a transient status error. */ } finally { if (!closed) timer = setTimeout(poll, document.hidden ? 3000 : 1000); } };
    void poll(); return () => { closed = true; clearTimeout(timer); };
  }, [!!view.details]);
  useEffect(() => { let unlisten: (() => void) | undefined; let stopped = false; void import("@tauri-apps/api/event").then(({ listen }) => listen("close-requested", () => setQuitDialog(true))).then(fn => { if (stopped) fn(); else unlisten = fn; }).catch(() => {}); return () => { stopped = true; unlisten?.(); }; }, []);
  useEffect(() => {
    if (!active || !view.details || changing.current) return;
    let closed = false;
    const workspace = active.workspace_path, epoch = catalogEpoch.current;
    void (async () => {
      let offset: number | null = 0;
      do {
        const value = await query({ kind: "session", action: "catalog", args: { offset, current: indexedWorkspaces.current.has(workspace) } }) as { chats: Chat[]; next: number | null };
        if (closed || !Array.isArray(value?.chats)) return;
        const chats = [...(catalogRef.current.chats ?? [])];
        for (const item of value.chats) {
          const found = chats.findIndex(c => c.workspace === item.workspace && c.session === item.session), previous = chats[found];
          if (item.empty && !previous) continue;
          const chat = { ...item, general: normalizedPath(item.workspace).endsWith("/general-chat"), archived: previous?.archived ?? false, deleted: previous?.deleted ?? false, renamed: previous?.renamed ?? false, title: previous?.renamed ? previous.title : item.title };
          if (JSON.stringify(previous) !== JSON.stringify(chat)) await api.indexChat(chat);
          if (closed || epoch !== catalogEpoch.current) return;
          if (found < 0) chats.push(chat); else chats[found] = chat;
        }
        catalogRef.current = { ...catalogRef.current, chats }; setCatalog(catalogRef.current);
        offset = value.next;
      } while (offset !== null);
      indexedWorkspaces.current.add(workspace);
    })().catch(e => { if (!closed) setError(errorMessage(e)); });
    return () => { closed = true; };
  }, [scope, active?.revision, executing, query]);
  useEffect(() => {
    if (submitted?.ended) return;
    const start = submitted?.scope === scope ? submitted.at : executing ? observation?.started_ms : null;
    if (!start) return;
    const update = () => setElapsed(Math.max(0, Date.now() - start)); update();
    if (!executing && !busy) { if (submitted) setSubmitted(s => s ? { ...s, ended: Date.now() } : s); return; }
    const timer = setInterval(update, 250); return () => clearInterval(timer);
  }, [scope, submitted, executing, busy, observation?.started_ms]);
  useEffect(() => { if (folderInput.current) folderInput.current.setAttribute("webkitdirectory", ""); }, []);
  function browseProject() {
    if (actions.current) return;
    setProjectDialog(true);
    void action(async () => {
      const selected = await api.chooseChatFolder();
      if (selected) setPath(selected);
    });
  }
  async function action(fn: () => Promise<unknown>) { actions.current++; setBusy(true); setError(""); try { await fn(); } catch (e) { setError(errorMessage(e)); } finally { actions.current--; setBusy(actions.current > 0); } }
  async function changeFolder(request: FolderChange) {
    if (executing || dirty || busy) throw new Error("Finish the response and save or discard editor changes first.");
    const expectedScope = scope;
    actions.current++; setBusy(true);
    try {
      if (!active) throw new Error("Open a chat first.");
      const result = await run({ kind: "runtime", action: "folder_access", args: request }, active.scope) as FolderSelection;
      if (`${activeRef.current?.workspace_path}/${activeRef.current?.scope.session}` !== expectedScope) throw new Error("The conversation changed. Reopen Chat folders to review its permissions.");
      setFile(null); setFiles([]); setEdit(""); setReview(null); setEditorReset(v => v + 1);
      return result;
    } finally { actions.current--; setBusy(actions.current > 0); }
  }
  useEffect(() => { setStopping(false); }, [active?.active_operation]);
  async function stopResponse() { if (!active?.active_operation || stopping) return; setStopping(true); try { await query({ kind: "cancel", operation: active.active_operation }); } catch (e) { setStopping(false); throw e; } }
  async function openWorkspace(project?: string, preserveApproval = false) {
    if (changing.current) return;

    if (dirty) throw new Error("Save or discard your file changes before switching chats");
    viewEpoch.current++; changing.current = true;
    try { const next = project ? await api.open(project) : await api.general(); activeRef.current = next.snapshot; setView(next); if (!preserveApproval) { setApproval(savedApproval()); setConfirmBypass(false); } setFile(null); setFiles([]); setDetailsOpen(false); setPage("chat"); setProjectDialog(false); setRecovery(null); const c = await api.catalog(); setCatalog(old => ({ ...c, theme: old.theme })); return next; }
    catch (e) { if (project) setRecovery(await api.recovery(project).catch(() => null)); throw e; }
    finally { viewEpoch.current++; changing.current = false; }
  }
  async function chooseModel(model: Model) { setSelected(`${model.provider}/${model.id}`); window.localStorage?.setItem("ag1.model", `${model.provider}/${model.id}`); }
  const [confirmBypass, setConfirmBypass] = useState(false);
  function chooseApproval(mode: ApprovalMode) {
    setApprovalMenu(false);
    if (mode === "disabled" && approvalValue !== "disabled") { setConfirmBypass(true); return; }
    applyApproval(mode);
  }
  function applyApproval(mode: ApprovalMode) { setApproval(mode); if (mode !== "disabled") localStorage?.setItem("ag1.approval", mode); setApprovalMenu(false); }
  const approvalValue = approval || safeApproval((view.details?.context as any)?.approval_policy?.mode || defaultApproval);
  async function selectConversation(session?: string, native = !!view.details) {
    if (!native) { await run({ kind: "session", action: session ? "resume" : "new", args: session ? { session } : {} }); return; }
    viewEpoch.current++; changing.current = true;
    try { const next = await api.selectConversation(session); if (!next.snapshot) throw new Error("The conversation did not open."); activeRef.current = next.snapshot; setView(next); return next; }
    finally { viewEpoch.current++; changing.current = false; }
  }
  function preserveDraft() { if (scope) drafts.current.set(scope, { text: draft, files: attachments, plan, model: selected }); }
  async function ensureChat(pick = chosenModel, effort = pick ? reasoning[`${pick.provider}/${pick.id}`] : undefined) {
    // Capture the visible choice before any asynchronous workspace opening.
    if (!pick || pick.conflict) throw new Error("The selected model is unavailable. Choose an available model before sending.");
    if (!activeRef.current) {
      setNavigating("New chat");
      try {
        const opened = await openWorkspace(undefined, true);
        // Home says New chat. Opening storage must not resume its last session.
        if (opened?.snapshot?.message_count) await selectConversation(undefined, !!opened.details);
      } finally { setNavigating(null); }
    }
    if (!activeRef.current) throw new Error("The chat did not open. Your message has been kept; try opening it again.");
    if (!executing) {
      const current = view.details?.context as any;
      if (activeRef.current.model !== pick.id || current?.selected_model?.provider !== pick.provider) await run({ kind: "model", action: "select", args: { provider: pick.provider, model_id: pick.id } });
      if (activeRef.current?.model !== pick.id) throw new Error("The model selection was not confirmed. Your message has not been sent.");
      const controls = pick.reasoningControls;
      const sameModel = current?.selected_model?.provider === pick.provider && current?.selected_model?.id === pick.id;
      const confirmedMode = sameModel ? controls?.modes.find(m => m.level === current?.thinking)?.id : undefined;
      const requestedEffort = effort ?? confirmedMode ?? controls?.default;
      if (requestedEffort) await run({ kind: "model", action: "reasoning", args: { provider: pick.provider, model_id: pick.id, mode: requestedEffort } });
      await run({ kind: "runtime", action: "approval_policy", args: { mode: approvalValue } });
    }
  }
  async function loadModel() {
    const pick = chosenModel;
    if (!pick || pick.provider !== "llamacpp") return;
    setModelLoad({ model: pick.id, started_at: Date.now() });
    try {
      await ensureChat(pick);
      await run({ kind: "runtime", action: "load_model", args: { provider: pick.provider, model_id: pick.id } });
    } finally { setModelLoad(null); }
  }
  async function newChat() {
    const previousDraft = draft, previousAttachments = attachments; preserveDraft(); setNavigating("New chat");
    try { const opened = !activeRef.current ? await openWorkspace() : undefined; if (activeRef.current?.message_count || executing) await selectConversation(undefined, !!(opened?.details ?? view.details)); setApproval(savedApproval()); setConfirmBypass(false); setSubmitted(null); setDraft(current => current === previousDraft ? "" : current); setAttachments(current => current === previousAttachments ? [] : current); }
    finally { setNavigating(null); }
  }
  async function openChat(chat: Chat) {

    if (dirty) throw new Error("Save or discard your file changes before switching chats");
    if (chat.workspace === active?.workspace_path && chat.session === active?.scope.session) { setPage("chat"); return; }
    preserveDraft(); setNavigating(chat.title);
    try {
      const opened = active?.workspace_path !== chat.workspace ? await openWorkspace(chat.general ? undefined : chat.workspace) : undefined;
      const next = await selectConversation(chat.session, !!(opened?.details ?? view.details));
      const saved = drafts.current.get(`${chat.workspace}/${chat.session}`);
      setPage("chat"); setApproval(savedApproval()); setConfirmBypass(false); setSubmitted(null); setDraft(saved?.text ?? ""); setAttachments(saved?.files ?? []); setPlan(saved?.plan ?? false);
      const provider = (next?.details?.context as any)?.selected_model?.provider;
      if (saved?.model) setSelected(saved.model); else if (provider && next?.snapshot?.model) setSelected(`${provider}/${next.snapshot.model}`);
    }
    finally { setNavigating(null); }
  }
  async function saveChat(chat: Chat) { catalogEpoch.current++; await api.saveChat(chat); const next = { ...catalogRef.current, chats: (catalogRef.current.chats ?? []).map(c => c.session === chat.session && c.workspace === chat.workspace ? chat : c) }; catalogRef.current = next; setCatalog(next); }
  function addAttachments(files: File[]) {
    try { const next = [...attachments, ...files]; validateAttachments(next); setAttachments(next); } catch (e) { setError(errorMessage(e)); }
  }
  async function send() {
    const text = draft, submittedFiles = attachments;
    if (!text.trim() && !submittedFiles.length) return;
    const pick = chosenModel;
    if (submittedFiles.some(f => f.type.startsWith("image/")) && Array.isArray(pick?.input) && !pick.input.includes("image")) throw new Error(`${pick.id} is configured for text only. Choose a vision-enabled model, or configure its matching vision projector. Your message and images are still here.`);
    const began = Date.now(); setElapsed(0); setSubmitted({ at: began, scope, model: pick?.id });
    await ensureChat(pick); const s = activeRef.current!; setSubmitted({ at: began, scope: `${s.workspace_path}/${s.scope.session}`, model: pick?.id });
    const input = await uploadMessage(text, submittedFiles, run);
    await run(plan ? { kind: "plan", action: "draft", args: { input, previous: null, feedback: null } } : text.startsWith("/") && !submittedFiles.length ? { kind: "runtime", action: "client_action", args: { text } } : executing ? { kind: "session", action: "steer", args: { input } } : { kind: "prompt", input });
    setDraft(old => old === text ? "" : old); setAttachments(old => old === submittedFiles ? [] : old);
  }
  const fileRefresh = useRef(0);
  const refreshFiles = useCallback(async () => {
    const serial = ++fileRefresh.current, workspace = activeRef.current?.workspace_path;
    if (!workspace) return;
    const data = await query({ kind: "files", action: "list", args: {} });
    if (serial === fileRefresh.current && workspace === activeRef.current?.workspace_path && Array.isArray(data)) setFiles(data as string[]);
  }, [query]);
  useEffect(() => {
    if (page !== "files") return;
    let closed = false, timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try { await refreshFiles(); } catch (e) { if (!closed) setError(errorMessage(e)); }
      if (!closed) timer = setTimeout(refresh, document.hidden ? 15000 : 4000);
    };
    void refresh();
    return () => { closed = true; clearTimeout(timer); fileRefresh.current++; };
  }, [page, refreshFiles]);
  async function navigate(next: Page) { setPage(next); }
  function openReview(tab: ReviewTab) { reviewOpener.current = document.activeElement as HTMLElement; setDetailsOpen(false); setReview({ scope, tab, open: true }); }
  const openGitHubReview = useCallback((github: GitHubReviewTarget) => {
    if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(github.repo) || !Number.isSafeInteger(github.number) || github.number < 1) return;
    reviewOpener.current = document.activeElement as HTMLElement; setDetailsOpen(false); setReview({ scope, tab: "github", open: true, github });
  }, [scope]);
  const clearGitHubReview = useCallback(() => setReview(old => old?.github ? null : old), []);
  const reviewChanges = useCallback((turn: string) => { reviewOpener.current = document.activeElement as HTMLElement; setDetailsOpen(false); setReview({ scope, tab: "changes", open: true, turn }); }, [scope]);
  const reviewOutputs = useCallback((turn: string) => { reviewOpener.current = document.activeElement as HTMLElement; setDetailsOpen(false); setReview({ scope, tab: "artifacts", open: true, artifactTurn: turn }); }, [scope]);
  const reviewEffect = useCallback(async (command: Command) => {
    const current = activeRef.current;
    if (!current || `${current.workspace_path}/${current.scope.session}` !== scope) throw new Error("The conversation changed. Reopen review before changing files or Git state.");
    if (dirty || ["streaming", "running", "awaiting_input"].includes(current.status)) throw new Error("Finish the response and save or discard unsaved editor changes before changing files or Git state.");
    actions.current++; setBusy(true);
    try { return await run(command, current.scope); }
    finally { actions.current--; setBusy(actions.current > 0); }
  }, [scope, dirty, run]);
  function closeReview() { setReview(old => old ? { ...old, open: false } : null); requestAnimationFrame(() => restoreReviewFocus(reviewOpener.current, review?.tab ?? "changes")); }
  async function readFile(name: string) { if (dirty) throw new Error("Save or discard your changes before opening another file"); const expected = queryScope, buffer = editRef.current; const data = await query({ kind: "files", action: "read", args: { path: name } }) as FileView; if (JSON.stringify(activeRef.current?.scope) !== expected || editRef.current !== buffer) return; setFile(data); setEdit(data.text); setEditorReset(v => v + 1); }
  async function save() { if (!file) return; const submitted = edit; const data = await run({ kind: "files", action: "save", args: { path: file.path, text: submitted, revision: file.revision } }) as FileView; setFile(data); if (editRef.current === submitted) setEdit(data.text); }
  useEffect(() => { const key = (e: KeyboardEvent) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") { e.preventDefault(); setProjectDialog(true); } if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") { e.preventDefault(); if (dirty) void action(save); } if ((e.ctrlKey || e.metaKey) && e.key === ",") { e.preventDefault(); setPage(page === "settings" ? "chat" : "settings"); } if (e.key === "Escape") { document.querySelectorAll<HTMLDetailsElement>(".chat-options[open]").forEach(d => { d.open = false; }); setAddMenu(false); setModelMenu(false); setApprovalMenu(false); setDetailsOpen(false); } }; document.addEventListener("keydown", key); return () => document.removeEventListener("keydown", key); });
  useEffect(() => { const outside = (e: PointerEvent) => { document.querySelectorAll<HTMLDetailsElement>(".chat-options[open]").forEach(d => { if (!d.contains(e.target as Node)) d.open = false; }); if (menus.current && !menus.current.contains(e.target as Node)) { setAddMenu(false); setModelMenu(false); setApprovalMenu(false); } }; document.addEventListener("pointerdown", outside); return () => document.removeEventListener("pointerdown", outside); }, []);
  useEffect(() => { setRequestPage(null); setReviewed(null); setAnswer(""); }, [scope]);
  const pending = active?.pending[0];
  async function acceptPending() {
    // Timers can be throttled while hidden: recheck the wall-clock deadline at
    // the action boundary, independently of the last rendered button state.
    if (!pending) return;
    if (approvalExpired(pending.binding.expires_ms)) throw new Error("This request expired. Ask AG1 to propose the operation again.");
    await query(pending.kind === "approval" ? { kind: "approve", binding: pending.binding, allow: true } : { kind: "session", action: "answer", args: { binding: pending.binding, answer } });
    setAnswer("");
  }
  async function readRequest() { if (!pending) return; const offset = requestPage?.id === pending.binding.id && requestPage.next ? requestPage.next : "0"; if (offset === "0") requestDecoder.current = new TextDecoder(); const data = await query({ kind: "artifact_read", artifact: pending.evidence, offset, limit: 32768 }) as { base64: string; next: string | null }; const text = requestDecoder.current.decode(Uint8Array.from(atob(data.base64), c => c.charCodeAt(0)), { stream: data.next !== null }); setRequestPage({ id: pending.binding.id, text, next: data.next }); if (data.next === null) setReviewed(pending.binding.id); }
  function theme(theme: Theme) { themeVersion.current++; setCatalog(old => ({ ...old, theme })); void api.theme(theme).catch(e => setError(errorMessage(e))); }
  const available = models.length ? models : (view.details?.models ?? []) as unknown as Model[];
  const chosenModel = selected ? available.find(m => `${m.provider}/${m.id}` === selected) : available.find(m => m.id === active?.model) ?? available[0];
  const modelName = chosenModel?.id || (selected ? "Unavailable model" : "Choose model");
  const requestModel = executing ? active?.model : busy && submitted ? submitted.model : undefined;
  const chats = (catalog.chats ?? []).filter(c => !c.empty && !c.deleted && !!c.archived === archived && c.title.toLowerCase().includes(search.toLowerCase())).sort((a, b) => (b.updated ?? "").localeCompare(a.updated ?? ""));
  function chatRow(chat: Chat) { const state = conversationStatuses.find(s => s.workspace === chat.workspace && s.session === chat.session); const working = state && ["streaming", "running", "queued"].includes(state.status); const label = state?.pending ? "Needs approval" : state?.error || state?.status === "failed" ? "Failed" : working ? state?.status === "queued" ? "Queued" : "Working" : state?.status === "completed" ? "Finished" : ""; return <div className={`chat-row ${chat.session === active?.scope.session && chat.workspace === active?.workspace_path ? "selected" : ""}`} key={`${chat.workspace}/${chat.session}`}><button disabled={busy} title={chat.title} onClick={() => void action(() => openChat(chat))}><MessageSquare size={14} /><span>{chat.title}</span>{label && <span className={`chat-run-state ${working ? "working" : ""}`} title={label} aria-label={label}>{working ? <LoaderCircle size={12} className="spin" /> : state?.pending ? "!" : state?.error || state?.status === "failed" ? "!" : <Check size={12} />}</span>}</button><details className="chat-options"><summary aria-label={`Options for ${chat.title}`}><MoreHorizontal size={16} /></summary><div className="menu"><button onClick={() => { setRenaming(chat); setNewTitle(chat.title); }}>Rename</button><button onClick={() => void action(() => saveChat({ ...chat, archived: !chat.archived }))}>{chat.archived ? "Restore chat" : "Archive chat"}</button><button className="danger-text" disabled={busy || !!working || !!state?.pending} onClick={() => setDeleting(chat)}>Delete chat</button></div></details></div>; }
  return <GitHubReviewContext.Provider value={{ open: openGitHubReview, clear: clearGitHubReview }}><div className="shell redesigned">
    {sidebar && page !== "settings" && <aside className="chat-sidebar"><div className="sidebar-brand"><span>AG1</span><button aria-label="Hide sidebar" onClick={() => setSidebar(false)}><PanelLeftClose size={17} /></button></div><button className="new-chat" disabled={busy} onClick={() => void action(newChat)}><Plus size={17} />New chat</button><label className="chat-search"><Search size={15} /><input aria-label="Search chats" placeholder="Search chats" value={search} onChange={e => setSearch(e.target.value)} /></label><div className="sidebar-scroll"><div className="sidebar-section-title">Projects<button aria-label="Add project" disabled={busy} onClick={browseProject}><Plus size={15} /></button></div>{catalog.projects.map(p => <details className="project-chats" key={p} open><summary><FolderOpen size={14} /><span>{basename(p)}</span></summary><button className="project-open" title={p} disabled={busy} onClick={() => void action(() => openWorkspace(p))}>Open project</button>{chats.filter(c => c.workspace.replace(/\/+$/, "") === p.replace(/\/+$/, "")).map(chatRow)}</details>)}{!catalog.projects.length && <p className="sidebar-empty">Attach a project when you need one.</p>}<div className="sidebar-section-title">{archived ? "Archived chats" : "Chats"}</div>{chats.filter(c => c.general || !catalog.projects.some(p => p.replace(/\/+$/, "") === c.workspace.replace(/\/+$/, ""))).map(chatRow)}{!chats.length && <p className="sidebar-empty">Your conversations will appear here.</p>}</div><div className="sidebar-bottom"><button onClick={() => setArchived(!archived)}>{archived ? "Back to chats" : "Archived chats"}</button><button onClick={() => { setSettingsSection("github"); setPage("settings"); }}><Github size={16} />GitHub</button><button disabled={busy} onClick={() => void action(async () => { if (!activeRef.current) await openWorkspace(undefined, true); setPage("mcp"); })}>MCP</button><button aria-label="Settings" onClick={() => { setSettingsSection("appearance"); setPage("settings"); }}><Settings2 size={16} />Settings</button></div></aside>}
    <main className="main"><header className="chat-header">{!sidebar && <button aria-label="Show sidebar" onClick={() => setSidebar(true)}><PanelLeftClose size={18} /></button>}<button className="chat-heading" onClick={() => setPage("chat")}>{page === "settings" ? "Settings" : page === "mcp" ? "MCP" : navigating || currentChat?.title || "New chat"}</button><div className="header-actions"><button className="workspace-pill" aria-label={folderSelection ? "Chat folders" : undefined} title={folderSelection ? workingFolder : undefined} disabled={busy} onClick={() => folderSelection ? setFoldersDialog(true) : setProjectDialog(true)}><FolderOpen size={15} />{folderSelection ? normalizedPath(workingFolder || "").endsWith("/general-chat") ? "Chat folders" : basename(workingFolder || "") : active && !general ? basename(active.workspace_path) : "Attach project"}</button>{active && <><button aria-label="Files" title="Files" onClick={() => void action(() => navigate("files"))}><FileCode2 size={17} /></button><button aria-label="Changes" title="Review changes" aria-expanded={review?.scope === scope && review.open && review.tab === "changes"} onClick={() => openReview("changes")}><GitCompareArrows size={17} /></button><button aria-label="Artifacts" title="Artifacts" aria-expanded={review?.scope === scope && review.open && review.tab === "artifacts"} onClick={() => openReview("artifacts")}><PanelRightOpen size={17} /></button><button aria-label="Task and activity" title="Task and activity" onClick={() => { setReview(old => old ? { ...old, open: false } : null); setDetailsOpen(!detailsOpen); }}><MoreHorizontal size={18} /></button></>}</div></header>
      {((stoppable && page !== "chat") || busy && !executing && !modelLoad) && <div className="operation-banner" role="status"><span><LoaderCircle size={14} className="spin" />{executing ? `${status} · ${formatTime(elapsed)}` : "Recovering the stopped response…"}{executing && active?.model && <small> · {active.model}</small>}</span>{stoppable && <button aria-label="Stop current response" disabled={stopping} onClick={() => void action(stopResponse)}><Square size={13} />Stop</button>}</div>}
      {(error || view.error) && <div className="error" role="alert"><span>{error || view.error}</span>{accessError && folderSelection && <button className="outline-button" onClick={() => { setRequestedFolder(accessError); setFoldersDialog(true); }}>Make this folder writable…</button>}<button aria-label="Dismiss error" onClick={() => { setError(""); setView(v => ({ ...v, error: null })); }}><X size={16} /></button></div>}
      {recovery && <RecoveryPanel record={recovery} onClose={() => setRecovery(null)} />}
      {view.details && <><ExtensionShortcuts key={scope} run={query} /><ExtensionUI key={`ui/${scope}`} activity={view.details.activity} run={run} onEditor={(text, paste) => setDraft(old => paste ? old + text : text)} /></>}
      {detailsOpen && active && view.details && <NativeDetails key={scope} onArtifacts={() => openReview("artifacts")} details={view.details} status={active.status} model={active.model} run={run} onEditor={setDraft} onClose={() => setDetailsOpen(false)} />}
      <section className="chat" hidden={page !== "chat"}><div className="conversation-wrap">{navigating ? <div className="chat-welcome"><h1>{navigating}</h1><p>Opening conversation…</p></div> : active && view.details ? <NativeConversation key={scope} snapshot={active} details={view.details} run={query} compactTools={preferences.compactTools} workStatus={status} workTime={preferences.showTime && (executing || submitted?.scope === scope) ? formatTime(elapsed) : undefined} reviewChanges={reviewChanges} reviewOutputs={reviewOutputs} /> : active?.messages.length ? <div className="legacy-messages">{active.messages.map(m => <article className={m.role === "user" ? "user-message" : "assistant-message"} key={m.id}><Markdown text={m.text} /></article>)}{executing && <Markdown text={view.progress} />}</div> : <div className="chat-welcome"><div className="welcome-eyebrow">AG1</div><h1>What’s on your mind?</h1><p>A conversation, a plan, or your next project.</p></div>}</div>
      <div className="composer-area"><div className="turn-status" role="status"><span style={{ visibility: preferences.showTime && !(active && view.details) ? "visible" : "hidden" }}>{(executing || busy && submitted) && <LoaderCircle size={13} className="spin" />}{(executing || busy && submitted) ? `${executing ? status : "Preparing request"} · ${formatTime(elapsed)}` : submitted?.scope === scope ? `Worked for ${formatTime(elapsed)}` : ""}</span>{usage && preferences.showTokens && <span className="token-usage" title="Provider-reported tokens for the latest completed model request">Tokens · {Number(usage.input ?? 0).toLocaleString()} in · {Number(usage.output ?? 0).toLocaleString()} out · {Number(usage.cacheRead ?? 0).toLocaleString()} cached</span>}</div>
      <div className="composer-shell" ref={menus}>{addMenu && <div className="composer-menu menu" aria-label="Add to chat"><div className="menu-heading">Add</div><button onClick={() => { fileInput.current?.click(); setAddMenu(false); }}><Plus size={16} />Files</button><button onClick={() => { folderInput.current?.click(); setAddMenu(false); }}><FolderOpen size={16} />Folder contents</button><button aria-pressed={plan} onClick={() => { setPlan(!plan); setAddMenu(false); }}><MessageSquare size={16} />Plan mode <small>{plan ? "On" : "Plan before making changes"}</small>{plan && <Check size={14} />}</button><div className="menu-heading">Plugins</div><button onClick={() => { setPlugins(true); setAddMenu(false); }}>Browse plugins and skills</button><button onClick={() => { folderSelection ? setFoldersDialog(true) : setProjectDialog(true); setAddMenu(false); }}><FolderOpen size={16} />{folderSelection ? "Chat folders" : "Attach project"}</button></div>}
      <input hidden type="file" multiple ref={fileInput} aria-label="Attach files" onChange={e => { addAttachments(Array.from(e.target.files ?? [])); e.target.value = ""; }} /><input hidden type="file" multiple ref={folderInput} aria-label="Attach folder contents" onChange={e => { addAttachments(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      <form className={`composer${plan ? " is-planning" : ""}`} onSubmit={e => { e.preventDefault(); if (!actions.current && (!executing || !plan)) void action(send); }}>{(plan || attachments.length > 0) && <div className="composer-chips">{plan && <button type="button" onClick={() => setPlan(false)}>Plan mode <X size={12} /></button>}{attachments.map((f, i) => <DraftAttachment key={`${f.name}/${f.lastModified}/${i}`} file={f} remove={() => setAttachments(old => old.filter((_, n) => n !== i))} />)}</div>}<textarea onPaste={e => { const files = Array.from(e.clipboardData.files).filter(f => f.type.startsWith("image/")); if (!files.length) for (const item of Array.from(e.clipboardData.items)) { if (item.kind === "file" && item.type.startsWith("image/")) { const file = item.getAsFile(); if (file) files.push(file); } } if (!files.length) return; e.preventDefault(); addAttachments(files); const text = e.clipboardData.getData("text/plain"); if (text) { const { selectionStart: start, selectionEnd: end } = e.currentTarget; setDraft(old => old.slice(0, start) + text + old.slice(end)); } }} aria-label="Message AG1" placeholder={executing ? "Add a direction…" : plan ? "What would you like to plan?" : "Ask anything"} value={draft} onChange={e => setDraft(e.target.value)} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && (preferences.sendKey === "enter" || e.ctrlKey || e.metaKey)) { e.preventDefault(); e.currentTarget.form?.requestSubmit(); } }} />
      <ModelLoadControl model={chosenModel} progress={preparation} pending={modelLoad} disabled={busy || executing} load={() => { if (!actions.current) void action(loadModel); }} />
      <div className="composer-toolbar"><div className="composer-left"><button type="button" aria-label="Add to chat" aria-expanded={addMenu} onClick={() => { setAddMenu(!addMenu); setModelMenu(false); setApprovalMenu(false); }}><Plus size={19} /></button><div className="approval-anchor"><button type="button" className="permission-label" aria-label="Tool approvals" aria-expanded={approvalMenu} title={`Next response: ${approvalModes.find(m => m.id === approvalValue)?.label || "Tool approvals"}`} onClick={() => { setApprovalMenu(!approvalMenu); setAddMenu(false); setModelMenu(false); }}><ApprovalIcon value={approvalValue} /><span>{approvalModes.find(m => m.id === approvalValue)?.label || "Tool approvals"}{(view.details?.context as any)?.approval_policy?.mode !== approvalValue ? " · next response" : ""}</span></button>{approvalMenu && <div className="approval-menu menu"><div className="menu-heading">Tool approvals</div><ApprovalChoices value={approvalValue} onChange={chooseApproval} /><p>{plan ? "Plan mode keeps its read-only restrictions." : `Selected for your next response. Current policy: ${(view.details?.context as any)?.approval_policy?.mode || "not yet confirmed"}.`}</p></div>}</div></div><div className="composer-right"><div className="model-anchor"><button type="button" className="model-picker" aria-label="Choose model" aria-expanded={modelMenu} onClick={() => { setModelMenu(!modelMenu); setAddMenu(false); setApprovalMenu(false); }}><span>{modelName}</span><ChevronDown size={13} /></button>{modelMenu && <ModelPicker key={chosenModel ? `${chosenModel.provider}/${chosenModel.id}` : "none"} models={available} selected={chosenModel} choose={m => void chooseModel(m)} mode={chosenModel ? reasoning[`${chosenModel.provider}/${chosenModel.id}`] : undefined} setMode={mode => { if (chosenModel) setReasoning(old => ({...old, [`${chosenModel.provider}/${chosenModel.id}`]:mode})); }} activeLevel={(view.details?.context as any)?.selected_model?.provider === chosenModel?.provider && (view.details?.context as any)?.selected_model?.id === chosenModel?.id ? (view.details?.context as any)?.thinking : undefined} />}</div>{stoppable ? <button type="button" className="send-button" aria-label="Stop generation" disabled={stopping} onClick={() => void action(stopResponse)}><Square size={16} /></button> : <button type="submit" className="send-button" aria-label="Send message" disabled={busy || !draft.trim() && !attachments.length}><ArrowUp size={18} /></button>}{executing && !!draft.trim() && <button type="submit" aria-label="Send steering" disabled={busy || plan}><ArrowUp size={18} /></button>}</div></div></form></div><div className="composer-note">{requestModel && requestModel !== modelName ? `This request uses ${requestModel}. ${modelName} is selected for your next message.` : busy && !executing && !modelLoad ? "Preparing your request…" : (preferences.sendKey === "enter" ? "Enter to send · Shift Enter for a new line" : "Ctrl / ⌘ Enter to send · Enter for a new line")}</div></div></section>{page === "settings" ? <Settings gitRun={query} gitEffect={reviewEffect} changes={() => { setPage("chat"); openReview("changes"); }} key={`${settingsSection}/${scope}/${active?.scope.epoch ?? 0}`} initialSection={settingsSection} discuss={text => { setDraft(old => old ? `${old}\n\n${text}` : text); setPage("chat"); }} theme={catalog.theme} setTheme={theme} plugins={catalog.themes ?? []} refreshThemes={async () => { const c = await api.catalog(); setCatalog(old => ({ ...old, themes: c.themes ?? [] })); }} preferences={preferences} update={updatePreferences} models={available} modelName={modelName} chooseModel={m => void chooseModel(m)} runningModel={requestModel} approval={approvalValue} setApproval={chooseApproval} back={() => setPage("chat")} quit={() => setQuitDialog(true)} chats={catalog.chats ?? []} restore={c => void action(() => saveChat({ ...c, archived: false, deleted: false }))} workspace={workingFolder && !normalizedPath(workingFolder).endsWith("/general-chat") ? workingFolder : undefined} attach={path => void action(() => openWorkspace(path))} /> : page === "mcp" ? <MCPManager key={`${scope}/${active?.scope.epoch ?? 0}`} scope={queryScope} run={mcpRun} back={() => setPage("chat")} /> : page === "files" ? <div className="files-layout"><aside className="file-tree"><button onClick={() => void action(refreshFiles)}>Refresh files</button>{files.map(name => <button key={name} onClick={() => void action(() => readFile(name))}>{name}</button>)}</aside><section className="editor-panel file-pane"><header className="file-toolbar"><span><span className="file-name" title={file?.path}>{file?.path || "Project files"}</span>{dirty && <span className="dirty" aria-label="Unsaved changes">●</span>}</span><div><button disabled={!dirty || busy} onClick={() => void action(save)}>Save</button><button disabled={!file} onClick={() => { if (file) { setEdit(file.text); setEditorReset(v => v + 1); } }}>Discard</button><button disabled={!file} onClick={() => setPreview(!preview)}>Preview</button></div></header>{file ? preview ? <iframe title="Isolated static preview" sandbox="" referrerPolicy="no-referrer" srcDoc={previewDocument(edit)} /> : <Suspense fallback={<p>Loading editor…</p>}><Editor path={file.path} text={edit} theme={catalog.theme} fontSize={preferences.codeSize} fontFamily={preferences.codeFont} wordWrap={preferences.codeWrap} resetKey={`${file.path}/${editorReset}`} onChange={setEdit} /></Suspense> : <div className="empty"><h2>Your project files</h2><p>Select a file from the sidebar.</p></div>}</section></div> : null}
    </main>
    {(active || review?.github) && review?.scope === scope && <ReviewPanel key={`${scope}/${workingFolder}`} open={review.open && (page !== "settings" || review.tab === "github")} tab={review.tab} turn={review.turn} artifactTurn={review.artifactTurn} github={review.github} localAvailable={!!active} onTab={tab => { if (tab !== "github") setPage("chat"); setReview(old => old ? { ...old, tab } : null); }} run={query} effect={reviewEffect} onClose={closeReview} />}
    <Modal open={foldersDialog} close={() => { setFoldersDialog(false); setRequestedFolder(undefined); }} title="Chat folders"><ChatFolders key={`${scope}/${requestedFolder || ""}`} selection={folderSelection} requestedPath={requestedFolder} blocked={executing ? "Finish or stop the current response to change folders." : dirty ? "Save or discard your editor changes before changing folders." : undefined} change={changeFolder} /></Modal>
    <Modal open={projectDialog} close={() => setProjectDialog(false)} title="Open a project"><p>Browse to a folder or create one in the folder picker, then open it as your project.</p>{error && <p className="inline-error" role="alert">{error}</p>}<form onSubmit={e => { e.preventDefault(); if (!actions.current && path.trim()) void action(() => openWorkspace(path.trim())); }}><button type="button" className="outline-button" disabled={busy} onClick={browseProject}><FolderOpen size={16} />Browse folders…</button><input aria-label="Project folder" placeholder="/home/you/projects/my-project" value={path} onChange={e => setPath(e.target.value)} autoFocus /><div className="project-recents">{catalog.projects.map(p => <button type="button" key={p} onClick={() => void action(() => openWorkspace(p))}>{p}</button>)}</div><div className="dialog-actions"><button type="button" onClick={() => setProjectDialog(false)}>Cancel</button><button className="primary-button" disabled={busy || !path.trim()}>{busy && <LoaderCircle size={14} className="spin" />}Open project</button></div></form></Modal>
    <Modal open={plugins} close={() => setPlugins(false)} title="Plugins and skills">{active ? <ResourcesPane run={run} appearance={() => { setPlugins(false); setSettingsSection("appearance"); setPage("settings"); }} /> : <><p>Start a general chat to browse your configured plugins and skills.</p><button className="primary-button" disabled={busy} onClick={() => void action(() => ensureChat())}>Open plugin catalog</button></>}</Modal>
    <Modal open={!!renaming} close={() => setRenaming(null)} title="Rename chat"><form onSubmit={e => { e.preventDefault(); if (renaming && newTitle.trim()) void action(async () => { await saveChat({ ...renaming, title: newTitle.trim().slice(0, 120), renamed: true }); setRenaming(null); }); }}><input aria-label="Chat title" value={newTitle} onChange={e => setNewTitle(e.target.value)} autoFocus /><button className="primary-button" disabled={!newTitle.trim()}>Save name</button></form></Modal>
    <Modal className="approval-dialog" open={!!pending} close={() => { if (pending) void action(() => query({ kind: "approve", binding: pending.binding, allow: false })); }} title={pending?.kind === "approval" ? "Review tool request" : "AG1 needs your input"}>
      <ApprovalReview key={pending?.binding.id} title={pending?.title || ""} expires={pending?.binding.expires_ms} actions={(expired, checkExpired) =>
        <div className="dialog-actions">
          <button autoFocus onClick={() => pending && void action(() => query({ kind: "approve", binding: pending.binding, allow: false }))}>Deny</button>
          <button className="primary-button" onKeyDown={e => { if (e.key === "Enter") e.preventDefault(); }} disabled={expired || !!(pending?.truncated && reviewed !== pending.binding.id) || !!(pending?.kind === "select" && !pending.options.includes(answer))} onClick={() => { if (!checkExpired()) void action(acceptPending); }}>{pending?.kind === "approval" ? "Allow this operation" : "Send answer"}</button>
        </div>
      }>{() => <>
        {pending?.truncated && <><p>Review the complete request before allowing it.</p>{requestPage?.id === pending.binding.id && <pre tabIndex={0} aria-label="Complete request page">{requestPage.text}</pre>}<button onClick={() => void action(readRequest)}>{requestPage?.id === pending.binding.id && requestPage.next ? "Next request page" : "Read complete request"}</button></>}
        {pending?.kind === "select" ? <select aria-label="Answer" value={answer} onChange={e => setAnswer(e.target.value)}><option value="">Choose an option</option>{pending.options.map(o => <option key={o}>{o}</option>)}</select> : pending?.kind !== "approval" && <input aria-label="Answer" value={answer} onChange={e => setAnswer(e.target.value)} />}
      </>}</ApprovalReview>
    </Modal>
    <Modal open={!!deleting} close={() => setDeleting(null)} title="Delete chat?"><p>Move “{deleting?.title}” to Trash? You can restore it in Settings. Its journal and shared artifacts will be retained.</p><div className="dialog-actions"><button onClick={() => setDeleting(null)}>Cancel</button><button className="primary-button" disabled={busy || !!(deleting && conversationStatuses.some(s => s.workspace === deleting.workspace && s.session === deleting.session && (["running", "streaming", "queued", "awaiting_input"].includes(s.status) || s.pending > 0)))} onClick={() => void action(async () => { if (!deleting) return; const chat = deleting; if (activeRef.current?.scope.session === chat.session && activeRef.current.workspace_path === chat.workspace) await newChat(); await saveChat({ ...chat, deleted: true }); setDeleting(null); })}>Move to Trash</button></div></Modal>
    <Modal open={confirmBypass} close={() => setConfirmBypass(false)} title="Bypass tool approvals?"><p>AG1 will run tools, edit files, execute commands, and perform connected GitHub operations without asking you to approve each operation. This applies only to this chat while AG1 stays open. Starting or switching chats, or restarting AG1, requires confirmation again.</p><p>Plan mode and the host’s workspace restrictions still apply.</p><div className="dialog-actions"><button autoFocus onClick={() => setConfirmBypass(false)}>Cancel</button><button className="primary-button" onClick={() => { applyApproval("disabled"); setConfirmBypass(false); }}>Enable bypass</button></div></Modal>
    <Modal open={quitDialog} close={() => setQuitDialog(false)} title="Quit AG1?"><p>AG1 will stop its work and close cleanly.</p><div className="dialog-actions"><button onClick={() => setQuitDialog(false)}>Cancel</button><button className="primary-button" onClick={() => void action(api.quit)}>Quit</button></div></Modal>
  </div></GitHubReviewContext.Provider>;
}
