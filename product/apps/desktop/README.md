# AG1 desktop

AG1 uses a React/TypeScript interface with a native Rust agent runtime. The default desktop executes through the Rust platform host; the retained TypeScript bridge is a legacy compatibility path, not the default agent loop. General chats and attached project chats have separate histories.

## Interface

Chats live in the left sidebar. The composer contains model selection, attachments, plan mode and tool approval choices. Model changes apply to the next message. Responses stream as they arrive; thinking and tool activity are expandable. Earlier messages load as you scroll upward without discarding the conversation already loaded.

Settings provides Discord-inspired dark, Tokyo, light and system appearance, independent UI/chat/code/status text sizes and fonts, layout widths, line spacing, colors, motion and chat preferences. Preferences stay local to the desktop. Task details and retained artifacts are available from the chat menu.

Tool approvals offers Ask before changes, Allow routine changes and Bypass approvals. The native runtime persists the selected policy. Planning restrictions still apply in plan mode. Changing policy does not approve a pending request or alter an executing tool batch.

## GitHub

Install the official GitHub CLI (`gh`) on PATH. Settings → GitHub → Connect starts GitHub CLI's browser/device authorization, or connects its existing signed-in account after the user's explicit action. Credentials remain under GitHub CLI's management and are not exposed to the renderer or agent prompt. Disconnect disconnects AG1 without logging other CLI applications out.

The integration includes repository browsing and cloning; issue and pull-request creation, metadata, comments, reviews and exact-head merge; shared paged file/check review; Actions workflows/runs/jobs/logs/artifacts; release creation, editing and asset transfers; Discussions and Projects forms; and account/repository notification inboxes. The advanced workbench discovers pinned REST schemas and supports bounded JSON REST/GraphQL requests. UI and model actions share native account/scope-bound one-use confirmations. Discuss with AG1 inserts an editable draft without sending it.

Local Git provides granted-folder status/diff, staging, commits, branches, exact-SHA fetch, fast-forward pull, leased push, clone and recoverable checkout. Standard linked worktrees require explicit access to their exact metadata roots. New work uses aggregate native storage admission/runtime budgets; existing recovery records are preserved. Read/status remains available at capacity, and interrupted transitions require explicit finish or restore. Repositories requiring alternate object stores, arbitrary separate Git directories or unsupported linked metadata are refused for writes.

GitHub.com is supported; Enterprise is unsupported. Binary transfer uses dedicated bounded native operations (64 MiB per file), and secret values use a private native dialog. General token/credential administration, arbitrary binary endpoints and app/JWT authentication flows are unsupported. Advanced JSON requests remain consequential; account permissions and unsupported schema details are checked by GitHub. Uncertain writes are never automatically retried. See [the capability ledger](../../../implementation/evidence/implementation-records/github/P14-GITHUB-CAPABILITIES.md) for exact coverage, verification and limits.

## Build and verification

From the repository root:

```sh
npm --prefix product/apps/desktop ci
npm --prefix product/apps/desktop run build
npm --prefix product/apps/desktop test -- --run
cargo test --manifest-path product/Cargo.toml -p ag1-desktop --tests --target-dir .local/p14-target
python tools/release_package.py
```

The release packager builds the native production feature set, verifies the dependency/license closure and records the source digest, manifest and archive hashes. Keep the frozen P13 `product/target/release/ag1-process-host` untouched; use the separate target directory above. Linux requires GTK3/WebKitGTK 4.1. Models and verified llama.cpp runtimes remain external and retain the original tuning. Windows/macOS packaging assets exist, but this UI release was validated on Linux.

Native visual tests use `--features visual-test` and isolated fake providers on a private Xvfb display. The diagnostic broker is absent from production builds. Do not use the user's active display for tests.

## Runtime and recovery

The host serializes commands and workspace ownership. Durable approvals bind to the exact operation. Stop cancels the active response; when stopping an owned local generation requires terminating its server, the next response reloads it. An unowned remote generation cannot be declared safely stopped without proof of quiescence.

Opening a chat validates the captured configuration and launch plans without reading model weights. The runtime verifies only the requested model’s material bytes before inference, retaining identity checks for warm reuse. A cold first request can still spend time verifying/loading that model and processing its prompt. No new long-duration performance acceptance is claimed.

The first send from the New chat screen creates a fresh session when saved history exists. New/resume/fork navigation waits for its durable completion before later commands are sent; delayed background views cannot overwrite that transition. A failed or unconfirmed model selection retains the draft and prevents prompt submission.

See `implementation/evidence/P14/CURRENT.md` for the current handoff and `implementation/evidence/implementation-records/desktop/DESKTOP-UI-ROLLBACK.md` for reversible source and systemwide release instructions.

The settings-delivery UI replaces theme cards with an importable data-only theme library; see [THEME-PLUGINS.md](THEME-PLUGINS.md). Appearance lives inside Settings. Chat deletion uses a restorable Trash and retains durable journals/shared artifacts. Plugins have separate browsing, installation, creation and advanced views. Plan mode has a configurable subtle composer outline.

The model picker obtains reasoning controls through `model_reasoning`. A provider integration can declare `thinking.modes` and `thinking.default` alongside its model; native adapters remain responsible for mapping those modes into their API. A loaded, unauthenticated llama endpoint is inspected using read-only `/v1/models` and `/props`, with exact single-model identity, no redirects/proxies, bounded responses and short timeouts. Discovery does not load models or perform inference. An unavailable or unenumerated capability stays unknown instead of synthesizing effort levels from a boolean or model name. Explicit changes are checked again against the selected model before the scoped native command is admitted. Discovered native effort mappings are journaled with the model identity so resume retains their request semantics.


Native chat navigation retains independent session drivers. Switching the visible chat does not cancel its work; Stop addresses that chat's operation and approval policy remains scoped to its owner. Local requests share endpoint admission and queue behind a busy model; API requests use the existing remote scheduler capacity. The desktop retains at most 64 resident owners, evicting only idle nonselected conversation drivers. Persisted history remains available. Project repository controls wait for all running conversations in that project. Legacy TypeScript workspace switching still requires settlement.

Tool progress commentary stays visible above grouped execution detail. Results are paired by recorded tool-call identity. Expand a call to read its output; raw text is literal source with a bounded preview, optional wrapping, Copy, and Show full output. Expanding activity stops automatic following so the clicked row remains readable. Long tool results are hydrated only when opened.

Scrolling throughout the application is measured. Scroll rendering uses the browser animation clock, with no fixed frame timer. Bounded local diagnostics retain up to 20 scroll-time animation-frame samples in `rendering-samples.jsonl` inside desktop state. Samples contain timing/count/viewport data only. These measure callback cadence, not presented compositor frames; physical high-refresh performance and Windows/macOS behavior need platform measurements. On Wayland, AG1 uses GTK’s frame clock when the driver rejects DRM refresh waits. The fallback is limited to the AG1 executable, activates only during rendering, and can be disabled with `AG1_DISABLE_DISPLAY_CLOCK_FALLBACK=1`. NVIDIA defaults to shared-buffer WebKit compositing to avoid DMA-BUF import failures. Explicit graphics environment overrides are respected.


Routine-change mode recognizes native editPreview as read-only and editApply/append as file mutations, with the same workspace/approval restrictions as edit. Planning hides mutation aliases. Failed preview/apply/append results carry explicit error flags; an aborted apply reports zero published edits, and ambiguous targets are rejected. Native exact-edit failures may include bounded candidate read ranges and LF-normalized candidate strings, without relaxing matching or authorizing a replacement. A zero-usage cancellation no longer erases an earlier measurement from that turn.

Request-local working state is appended as transient tail context for each provider call. Earlier user messages and accumulated tool rounds remain unchanged as inspection/task freshness changes. The suffix is neither journaled nor used as new user intent. This removes one demonstrated source of prefix cache invalidation; it does not guarantee server cache hits or eliminate model loading/prefill.
