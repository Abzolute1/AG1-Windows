# Dyad UI reuse map

All paths resolve against the pinned `vendor/dyad`. Classifications concern the inspected source; **direct** still requires dependency closure and notice checks. Do not copy the entire renderer, package.json or `ipc/types` barrel. File families below are explicit bounded sets or directory surfaces, not permission to reuse every transitive import.

| UI element | Exact path(s) | Classification | Extraction contract / validation |
|---|---|---|---|
| Semantic colors, radii, spacing, typography | `src/styles/globals.css` | Direct CSS tokens; decouple host rules/assets | Preserve both themes, 2.25rem chrome offset, 0.625rem radius; replace font URLs and app-region rules; screenshot comparison |
| Buttons/badges/cards/inputs/separators/skeletons | `src/components/ui/button.tsx`, `badge.tsx`, `card.tsx`, `input.tsx`, `separator.tsx`, `skeleton.tsx` | Direct | Bring `src/lib/utils.ts`/CVA/Tailwind dependencies after audit; keyboard/disabled/error states |
| Dialog/menu/popover/tooltip/select | `src/components/ui/dialog.tsx`, `dropdown-menu.tsx`, `popover.tsx`, `tooltip.tsx`, `select.tsx` | Direct primitives | Preserve Base UI behavior, portal layering and focus return; do not substitute old Radix assumptions |
| Sidebar primitive | `src/components/ui/sidebar.tsx` | Reusable after decoupling | Local collapse/mobile/shortcut state allowed; remove product routing assumptions |
| Product sidebar | `src/components/app-sidebar.tsx` | Reusable after decoupling | AG1 project/session routes, recent items, settings; omit cloud/billing/library promotions |
| Title bar/window controls | `src/app/TitleBar.tsx` | Electron-bound | Recreate host actions with Tauri, retain visual geometry; drag regions/menus/zoom per OS |
| App root | `src/app/layout.tsx`, `src/renderer.tsx` | Visual-reference-only composition | Replace domain provider tree, telemetry, global stream side effects and window bootstrap |
| Split workspace | `src/pages/chat.tsx` | Reusable after decoupling | React resizable panes and persistence; AG1 IDs/session selectors; keyboard and resize tests |
| Home/create project | `src/pages/home.tsx`, `src/components/chat/HomeChatInput.tsx` | Reusable after decoupling / Dyad-agent-bound | Keep composition, recent projects and prompt entry; replace firstPrompt/quota/blueprint/template assumptions |
| Project list/details | `src/components/AppList.tsx`, `src/pages/apps.tsx`, `src/pages/app-details.tsx` | Reusable after decoupling | Backend-owned project identities and runtime targets; import/create/delete actions through protocol |
| Chat shell/header/tabs | `src/components/ChatPanel.tsx`, `src/components/chat/ChatHeader.tsx`, `ChatTabs.tsx` | Reusable after decoupling | Retain layout, tabs, scroll affordances; remove Dyad chat machine and version ownership |
| Message virtualization | `src/components/chat/MessagesList.tsx` | Reusable after decoupling | Keep Virtuoso/memoization/auto-scroll behavior; render AG1 stable message/block IDs |
| Message content | `src/components/chat/ChatMessage.tsx`, `DyadMarkdownParser.tsx` | Dyad-agent-bound | New typed AG1 message renderer; visual ancestor only for tool-card composition, not Dyad XML execution tags |
| Tool cards | `src/components/chat/DyadCardPrimitives.tsx`, `DyadOutput.tsx`, `DyadWebFetch.tsx`, `DyadListFiles.tsx` | Reusable after decoupling | Pure presentation accepts typed AG1 tool/outcome data; audit imports separately |
| Search/replace card | `src/components/chat/DyadSearchReplace.tsx` | Restricted dependency / do not reuse as-is | Imports `src/pro/shared/search_replace_parser.ts`; use AG1 edit/diff results and fresh parsing |
| Input and attachments | `src/components/chat/ChatInput.tsx`, `LexicalChatInput.tsx`, `AttachmentsList.tsx` | Reusable after decoupling | Preserve keyboard/edit/mention behavior; AG1 references, steering/follow-ups, artifact IDs and cancellation semantics |
| Progress/errors | `src/components/chat/StreamingLoadingAnimation.tsx`, `ChatErrorBox.tsx`, `ContextLimitBanner.tsx` | Reusable after decoupling | Distinguish generation/retry/compaction/persistence failure/settled state |
| Model selection | `src/components/ModelPicker.tsx`, `src/components/ProviderSettings.tsx` | Dyad-agent-bound, reusable visual shell | Replace Dyad provider/quota/pricing with AG1 capabilities, runtime/residency/context and model registry |
| Settings layout | `src/pages/settings.tsx`, `src/components/SettingsList.tsx`, `src/components/settings/ProviderSettingsPage.tsx` | Reusable after decoupling | Categories/layout, AG1 typed settings commands; no copying entire Dyad settings schema |
| Theme provider | `src/contexts/ThemeContext.tsx` | Electron-bound hook with portable state | Native theme adapter; preserve stale-bootstrap ordering and system fallback |
| Preview tabs/console layout | `src/components/preview_panel/PreviewPanel.tsx`, `PreviewToolbar.tsx`, `Console.tsx`, `ConsoleEntry.tsx`, `Problems.tsx` | Reusable after decoupling | Runtime capability-driven tabs, bounded log feed, diagnostics linking to files |
| Preview loading/error | `src/components/preview_panel/PreviewLoadingScreen.tsx`, `PreviewErrorBanner.tsx` | Reusable after decoupling | AG1 runtime states, retry/cancel/open logs actions |
| Web iframe preview | `src/components/preview_panel/PreviewIframe.tsx` | Restricted dependency + Dyad-agent-bound | Rebuild around clean preview adapter; exclude Pro Annotator and billing/integration code |
| Native Electron preview | `src/components/preview_panel/PreviewWebContentsView.tsx`, `PreviewNativeOverlayGuard.tsx`, `src/main/preview_web_contents_view.ts`, `preview_cdp_broker.ts` | Electron-bound | Product behavior reference only; platform spike for webview/automation isolation, overlays and focus |
| Files/tree/code | `src/components/preview_panel/FileTree.tsx`, `CodeView.tsx`, `FileEditor.tsx`, `fileSaveQueue.ts` | Reusable after decoupling | AG1 read/write CAS revision contracts, save queue, dirty buffers and conflict UI |
| Monaco setup | `src/components/chat/monaco.ts`, `src/utils/get_language.ts` | Reusable after asset/theme decoupling | Bundle workers locally; lazy load; validate CSP and OS webviews |
| Read-only diff editor | `src/components/preview_panel/FileDiffEditor.tsx` | Reusable after theme adapter | Props already isolate file path/old/new content; preserve widget-before-model disposal |
| Staged/version diff | `src/components/preview_panel/StagedDiffView.tsx`, `VersionDiffView.tsx`, `src/components/chat/VersionPane.tsx` | Reusable after decoupling | AG1 worktree/commit/session distinctions, expected-head guard, no automatic destructive restore |
| Terminal | `src/components/chat/TerminalPanel.tsx`, `src/ipc/utils/pty_session_manager.ts`, `pty_command_runner.ts` | Portable xterm UI; Electron/Node backend | Rust PTY protocol: byte stream, dimensions, exit status, process-tree lifecycle |
| Plan/questions | `src/components/preview_panel/PlanPanel.tsx`, `src/components/chat/QuestionnaireInput.tsx` | Dyad-agent-bound presentation | AG1 task/plan/approval typed state; question response correlation and expiry |
| Notifications | Sonner use in `src/app/layout.tsx`; `src/hooks/useNotificationHandler.ts` | Direct toast library / native decoupling | Session/project scoped notifications, durable errors remain inspectable after toast disappears |
| Component taggers/instrumentation | `packages/@dyad-sh/react-vite-component-tagger/`, `packages/@dyad-sh/nextjs-webpack-component-tagger/`, `worker/dyad-component-selector-client.js` | Apache candidates, optional after audit | Not required for initial AG1 parity; no assumed cross-webview automation support |
| Pro annotator, visual editing, local-agent backend | `src/pro/**` | Restricted-license / do not reuse | Inspect for understanding only; no code, assets or implementation translation into AG1 |
| Deploy/database/cloud subscription/quotas | `src/components/preview_panel/PublishPanel.tsx`, `DeploymentSection.tsx`, `DatabaseSection.tsx`, quota/subscription banners | Unnecessary for initial AG1 | Preserve extension capability seams; no speculative cloud product parity mandate |
| Dyad branding and provider assets | `assets/logo.svg`, `assets/icon/*`, `assets/ai-logos/*`, `assets/supabase/*` | Visual-reference-only / rights unclear | Use AG1 branding; separately clear third-party marks and artwork |

## Minimal AG1 additions

Keep the chat/files/diff/preview workflow. Add one compact status strip with context used/limit, run state and local model health. Clicking it opens a details drawer with tabs:

- **Task:** objective, latest instruction, phase, constraints, decisions, facts/hypotheses and verification state; edits are commands, never local authoritative mutations.
- **Context:** working-set budget versus externalized size, last compilation, retrieved ranges, source revisions and freshness/conflicts. “Current at revision …” is more useful than a generic green badge.
- **Artifacts & memory:** searchable records, byte-page rehydration, provenance, lease/retention and project memories with stale labels.
- **Activity:** tools/subagents/jobs, scheduler wait, retries, recovery and model throughput/logs. Show queues only when relevant; diagnostics download is explicit.

Native project targets use the same right pane: runtime/build output, diagnostics, launch/focus/stop and observation artifacts. URL controls appear only for web targets. Do not draw a fake embedded native window when the platform only supports launching an external process.

## Acceptance for visual extraction

Capture pinned Dyad home, chat streaming, files, dirty editor, diff, preview loading/error, settings, dialogs, toasts, collapsed panes and light/dark states on a supported runtime. Create equivalent AG1 scenario screenshots at matched dimensions and font scale. Review purposeful AG1 differences; require keyboard/focus/resize/scroll/empty/error interactions as well as images. Screenshot similarity alone cannot prove behavior or license cleanliness.
