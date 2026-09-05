# Dyad architecture and product flows

All paths are relative to `vendor/dyad` at the revision recorded in README. This is source-level UI/interaction inspection; a running packaged Dyad was not benchmarked or visually approved. The source checkout has no complete UI screenshot baseline. Capture those before frontend extraction is accepted.

## Shell and renderer

`package.json` identifies 1.14.0-beta.1, Electron 40.0.0 and React 19.2.4. `src/main_bootstrap.ts` leads to `src/main.ts`; Electron Forge/Vite build the main process, preload and renderer. Main owns windows, IPC registration, startup reconciliation, native theme, application lifecycle, DB, credentials, updater and subprocess resources. Product windows disable Node integration and enable context isolation; preview windows/views have further sandbox and navigation restrictions. Do not infer all windows share identical options.

`src/renderer.tsx` initializes React, query/cache infrastructure, telemetry and renderer service wiring. `src/router.ts` explicitly composes TanStack Router routes for home, chat, apps/app-details, settings/provider settings, templates, plugins, library, themes, prompts and media. `src/routes/root.tsx` renders `src/app/layout.tsx` around the outlet.

`RootLayout` wraps app runtime, screenshot, GitHub, image generation, first-prompt, plan handoff, version preview and preview iframe providers. `RootLayoutContent` subscribes to app output globally, binds chat stream effects, watches test/plan/integration events, handles shortcuts and renders ThemeProvider, sidebar, title bar, content and Sonner notifications. This is an orchestration root, not a portable shell component. Extract its composition and spacing while replacing its domain providers.

`src/pages/chat.tsx` composes `ChatPanel` and `PreviewPanel` with `react-resizable-panels`. It persists split geometry via `autoSaveId`, handles chat/preview collapse, restores prior widths and suppresses transitions while resizing. `src/components/ChatPanel.tsx` composes ChatHeader, MessagesList, ChatInput, VersionPane, loading/banners and lazy TerminalPanel. `src/components/preview_panel/PreviewPanel.tsx` composes toolbar, preview/code/problems/configuration/plan/testing/publishing/security surfaces and collapsible console. Run state/recording subscriptions outlive a single selected tab.

## Visual system

`src/styles/globals.css` defines Tailwind v4, `tw-animate-css`, typography plugin, light/dark semantic tokens and Geist/Geist Mono font faces in regular/medium/bold. Key reference values: title bar offset 2.25rem, base radius 0.625rem, light lavender background `oklch(0.985 0.0188 292.61)`, light primary `oklch(0.59 0.16 287.69)`, dark background `oklch(0.145 0 0)`, dark sidebar `oklch(0.205 0 0)`. Font URLs currently reference node_modules paths; rebundle licensed font files with application asset URLs.

`src/components/ui/button.tsx` uses CVA variants, shared `cn`, text-sm/medium, h-9 default controls, h-8 small controls, gap-2, focus rings and disabled states. `ui/sidebar.tsx`, dialog/menu/tooltip/popover primitives and `src/components/app-sidebar.tsx` supply navigation composition. The current primitive system is Base UI, not an assumption based on older Radix/shadcn versions. Lucide is the icon system. CSS `app-region` behavior in the title bar is Electron-specific.

`ThemeContext.tsx` persists theme choice but queries native theme and subscribes to native changes. It cancels an in-flight initial query before accepting a newer native event, preventing a stale bootstrap reply from overwriting current state. Preserve that ordering property in a Tauri adapter. Layout uses flex/min-width/overflow discipline, responsive utility classes, resizable panes, reduced-motion hooks and Framer Motion. Test small supported desktop window sizes, zoom, long paths/text, keyboard focus, high DPI and reduced motion; source inspection alone cannot prove cross-webview layout fidelity.

Loading/empty/error behavior comes from `PreviewLoadingScreen`, `PreviewErrorBanner`, `ChatErrorBox`, streaming indicators, setup/empty-state cards and Sonner notifications. Reuse the visual states but replace billing/quota and Dyad provider assumptions. Monaco and terminal are expensive surfaces; preserve lazy construction and disposal.

## Frontend state and protocol

TanStack Query owns many IPC-backed entity queries, with keys in `src/lib/queryKeys.ts`. Jotai atoms under `src/atoms` own selection, local view state and cached projections. More recent domain flows use state machines under `src/chat_stream`, `src/app_run`, `src/preview_iframe`, `src/version_preview`, `src/distributed_machines`, `src/state_machines` and related providers. Invocation identities, revisions, fences and cancellation tombstones guard stale async completions. They are meaningful product engineering evidence, not a requirement to port the entire framework.

`src/ipc/contracts/core.ts` defines Zod-backed invoke/event/send/stream contracts and client factories. `src/ipc/types/index.ts` assembles namespaces (`ipc.app`, `ipc.chatStream`, settings, runtime, Git, terminal, etc.). `createClient` obtains `window.electron.ipcRenderer`. `src/preload.ts` exposes an allowlisted context bridge supporting invoke envelopes, one-way sends, subscriptions and zoom. `src/ipc/preload/channels.ts` enumerates valid channels. `src/ipc/handlers/base.ts`, `safe_handle.ts`, `trusted_handle.ts` and `src/ipc/ipc_host.ts` register and validate main handlers; operation coordination and invalidation buses then govern mutations and cross-window cache refresh.

Replacing only `window.electron` would leave Dyad's data schemas, stream model, numeric app/chat IDs, user-input/subagent concepts and backend assumptions embedded in React. Build AG1 view models and query hooks over the AG1 protocol. Retain only presentation state in Jotai. Current global chat effects survive page unmount, but the future agent queue and continuation logic must survive the renderer itself.

## Project lifecycle, traced

| Step | Path and execution |
|---|---|
| Create/import | HomeChatInput/first-prompt flow and app/import hooks → `ipc.app.createApp` / import contracts → `src/ipc/handlers/app_handlers.ts` / `import_handlers.ts`. Create sanitizes display names, rejects display-name conflicts, chooses a unique folder slug, inserts app and initial chat records, creates template, initializes Git/ignores and cleans up failed creation. |
| Load | `useLoadApps`, `useLoadApp`, `useChats`, `useChatMessages` fetch DB-backed entities. Router app/chat IDs synchronize with selected atoms and per-window tab state. |
| Files | `FileTree.tsx`, `CodeView.tsx`, `FileEditor.tsx` → app file contracts → `app_handlers.ts` contained filesystem access. `fileSaveQueue.ts` coordinates saves; editor text cannot silently outrun saved state. |
| Dependencies | `src/ipc/services/app_runtime_service.ts` resolves package-manager signals and install/start commands, managed pnpm policy/rebuild warnings and alternate runtime modes. Node/pnpm is a project runtime dependency, not a reason to retain Node inside the final AG1 core. |
| Start | `useRunApp.ts` / `app_run` remote provider → app run contracts → `AppRunActorService` → `AppRuntimeService`; app resource claims serialize lifecycle actions, invocation IDs fence stale outputs. The service spawns install/dev processes and waits for readiness. |
| Preview | `src/ipc/utils/start_proxy_server.ts`, shared port helpers and runtime output bind the dev server to a preview origin. `PreviewIframe` normally renders/instruments that preview; native test/automation preview uses `PreviewWebContentsView` and main `preview_web_contents_view.ts`. |
| Logs/errors | `MainAppRuntimeOutput`, process manager and runtime transport deliver stdout/stderr/readiness/errors; preview bridge captures client console/network/errors; `preview_console`, Console and Problems present bounded projections. |
| Agent iteration | ChatInput/useStreamChat → typed chat stream contracts → `chat_stream_handlers.ts` → response processors or `src/pro/main/ipc/handlers/local_agent/local_agent_handler.ts`; changes update messages/files/version/runtime. This intelligence machinery is explicitly not the AG1 target. |
| Stop/delete | `AppRunActorService`/runtime service and `process_manager.ts` stop process trees/proxies. App operation coordination fences deletion/rename against streams, tests, recording and file/Git activity. Main quit controller drains cleanup; source retains recovery markers when external teardown fails. |
| Persistence | `src/db/schema.ts`, `src/db/index.ts`, Drizzle migrations, `src/main/settings.ts`; SQLite contains apps/chats/messages/versions/provider and product records. Settings/native secrets and project files live separately. |
| History | `src/ipc/handlers/version_handlers.ts`, `src/version_preview`, `version_preview_*` services and `git_utils.ts` map message/commit boundaries to history/diffs, refuse dirty/conflicted unsafe restores, handle detached HEAD and preserve eligible cancelled-turn changes in recovery commits. External database snapshots complicate rollback; not a general filesystem transaction. |

Local, Docker and cloud sandbox runtime branches are present. Cloud/billing/database integration details are not required for AG1 desktop parity; retain local project lifecycle behaviors and represent future runtime kinds through capabilities.

## Preview is a major migration seam

`src/components/preview_panel/PreviewIframe.tsx` imports the restricted Pro Annotator. It also integrates component selection, screenshots, recording, visual edits and postMessage routing. `worker/dyad-component-selector-client.js`, `worker/dyad-recorder-client.js`, proxy code and component tagger packages implement injected instrumentation. The iframe's surroundings can be reused after decoupling; the complete dependency tree is not approved for copying.

`src/main/preview_web_contents_view.ts` creates sandboxed views with no preload, no Node and ephemeral per-test partitions; navigation is restricted. `preview_cdp_broker.ts` attaches Chromium-specific debugging/automation. Native overlays require special z-order/focus/bounds handling (`PreviewNativeOverlayGuard`, `previewViewBounds`). Tauri webviews do not offer a drop-in Electron/CDP contract. Prefer a capability-oriented preview API with isolated web preview, optional external automation browser, and native app launch/observe adapters. Browser automation parity must be proved separately from preview display parity.

## Native/Electron replacement map

| Current subsystem | Target role |
|---|---|
| BrowserWindow/main/preload/contextBridge/IPC | Small Tauri host and typed command/channel adapter |
| WebContentsView/CDP/screenshot internals | Preview/automation service with platform capability tests; not mechanical translation |
| Node filesystem/Git/spawn/PTY | Rust workspace/process/tool services; external Git/compiler/package manager binaries as needed |
| better-sqlite3/Drizzle | Rust persistence with explicit import/migration; do not inherit Dyad agent schema |
| node-pty/xterm | Portable terminal UI after transport decoupling; Rust PTY owner with resize/input/output/shutdown |
| dialog/shell/clipboard/notifications/theme | Narrow Tauri plugins or host APIs with scoped commands |
| safeStorage/keychain native module | OS-backed secret service and migration policy; no renderer keys |
| Electron updater/Forge/fuses/Squirrel | Tauri packaging/signing/updater lifecycle and dependency/license review |
| Renderer PostHog + Dyad quota/auth | New explicit diagnostics settings; do not copy production project keys/endpoints or billing flows |

See the raw boundary inventory for 1,039 matching import/use lines, including tests. It is an inventory, not 1,039 independent production dependencies. UI reuse decisions and license exclusions are in the companion maps.

## Tests worth preserving as behavior specifications

`FileDiffEditor.test.tsx` protects Monaco widget/model disposal order; FileEditor/FileTree/CodeView tests protect editing/navigation; PreviewPanel/PreviewWebContentsView/PreviewToolbar and native overlay tests protect lifecycle and bounds; chat list/tabs/input/Message parser tests describe interactions. `app_runtime_service.test.ts`, app-run actor tests, operation-coordination integration tests and version-preview tests expose asynchronous races. `e2e-tests/` uses packaged Electron plus Playwright; the shell driver must change for Tauri, while scenarios can survive. No Dyad tests were executed in this research environment: pinned Node requirement is `>=24 <26`, current Node is 26.7, dependencies/native Electron modules are not installed in this checkout and no headless display was provisioned.
