# Proposed AG1 Desktop architecture

This is a research recommendation based on the two repository maps. It is not implemented and does not freeze a roadmap phase count. Rust owns the long-term runtime; Tauri 2 is a thin desktop host; React/TypeScript renders AG1 projections using approved Dyad visual components. The original AG1 remains the executable reference until parity and intentional corrections are proved.

## Process and ownership decision

Prefer an **embedded Rust core in the Tauri main process**, implemented as a GUI-independent library with command/event ports. Expensive blocking filesystem/Git/SQLite work runs on bounded worker executors, not Tauri's UI thread. Model servers, PTYs, project dev/build/native processes and optional browser automation are supervised external resources. This avoids an always-on daemon and extra JSON hop for every core operation while preserving headless library use.

Tauri uses a core process plus OS webview processes rather than shipping a Chromium runtime; exact memory/process counts remain platform-specific. This is the architectural basis for a lighter shell, not a measured performance claim. [Tauri process model](https://v2.tauri.app/concept/process-model/).

Closing the last window may detach presentation while the host completes or pauses active work according to an explicit setting; explicit application quit requests graceful cancellation/checkpoint/shutdown. A future headless executable may own the same core. Do **not** promise active tasks survive Tauri main-process termination in the initial embedded design. If later product requirements demand that, add a local service host over the same versioned protocol, with authenticated local sockets and a clearly specified lifetime.

The current TypeScript migration worker is a temporary separate process, justified by existing SDK/RPC/Godot embedding and by TUI/global-state imports that cannot disappear in one phase. Use a pinned **copy** of AG1 in the new workspace. It initially owns its existing journal/CV/model/tool behavior. A Rust host owns worker lifecycle and the application protocol. Do not share ownership of the same tools/models/persistence between Rust and TS during a turn.

Because current TS scheduling/provider registration/cwd are process-global, the first compatibility mode permits one executing workspace session. Other tabs may display snapshots. Do not spawn independent TS workers all contending for one local model and call that parity. Multi-session execution arrives only with explicit per-workspace isolation and a host-owned endpoint scheduler; alternatively a single worker with fully audited cwd isolation can be an intermediate gate, not an assumption.

## Coherent Rust boundaries

Start with the following packages, merging if the implementation proves a boundary unnecessary. Do not create one crate per TypeScript coordinator.

| Package / modules | Owns | Why this boundary exists |
|---|---|---|
| `ag1-core` | domain IDs/state machines; session/task/plan; runtime/tool orchestration; context compiler/retrieval policy; provider normalization/scheduler; evidence rules; port traits | Makes semantics testable without Tauri, filesystem or a live model. Modules separate ownership; a single package avoids micro-crate overhead. |
| `ag1-storage` | transactional journal/projections, migrations/import/export, immutable blobs/cursors/leases/catalog, memory | Persistent schema/integrity has its own fault tests and single-writer rules. Implements core storage ports. |
| `ag1-platform` | explicit-root filesystem/Git/search, process/PTY supervisor, local model launch/health, HTTP/secrets and project-runtime adapters | OS effects and child ownership are isolated; implements core/platform ports, no React/Tauri types. |
| `ag1-protocol` | versioned wire DTOs, command/result/error/event schema, generated TS declarations, compatibility conversions | Stabilizes GUI/headless/worker transport; does not become a second domain state store. |
| `ag1-desktop` (`src-tauri`) | Tauri composition, scoped commands/channels, windows/dialogs/theme/update/packaging, adapter wiring | Desktop-specific glue only. Later CLI/headless host composes the same libraries. |

Core defines ports; storage/platform implement them. Protocol maps domain views explicitly; core must not import a Tauri command or renderer store. Avoid a dependency cycle where core requires a concrete storage crate that itself requires core services. JSON serialization is appropriate at transport/persistence edges; domain hot paths use owned typed structures and shared immutable buffers where measurement warrants.

## State ownership and concurrency

Each session has one mutation owner with a bounded inbox. It owns task/plan/messages/recovery epoch and decision ordering. Tools run as cancellable child tasks with immutable admitted inputs; outcomes return to the owner. Per-file mutations additionally coordinate by canonical workspace/file identity. Workspace-wide Git/rename/delete and process lifecycle operations acquire explicit resource claims with a consistent order; never hold them while waiting for model output, a user approval or UI rendering.

The scheduler owns queues keyed by actual model endpoint/residency identity. Preserve local single-generation behavior and current remote default concurrency until evidence supports a change. Add bounded admission and visible backpressure, cancelled waiter removal and fair project servicing. Do not simultaneously add model parallelism while porting semantics. Use permit RAII and structured cancellation, but reconcile ignored aborts before assigning a strict server to another request.

Separate durable state events (task changes, accepted commands, tool intent/outcome, journal commits) from high-volume transient deltas (tokens/log progress). Coalesce display deltas on a bounded timer/size threshold; retain authoritative final text/log artifacts. Disk spool large process outputs. Slow subscribers get an explicit gap/resnapshot path; they cannot stall the agent indefinitely. Background indexes/watchers are hints, source snapshots/digests remain authority.

## Core/UI protocol proposal

Use Tauri commands for bounded request/result operations and Tauri channels for scoped token/tool/process streams. Global events are only low-volume, non-sensitive invalidation hints. Tauri's documentation distinguishes events from channels and explicitly cautions against using its event system for large/high-throughput payloads. [Frontend calls](https://v2.tauri.app/develop/calling-rust/), [Rust-to-frontend channels and events](https://v2.tauri.app/develop/calling-frontend/).

This transport choice does not itself provide domain ordering, durable replay or authorization. Define them in the protocol:

| Contract | Required fields / semantics |
|---|---|
| Handshake | protocol major/minor, schema hash, server instance ID, supported commands/capabilities, source/build version; reject incompatible major, negotiate optional fields |
| Command envelope | request ID/idempotency key, workspace ID, session ID if applicable, expected epoch/revision, command discriminator + validated arguments |
| Result | same request ID; success/error; accepted operation ID and durable sequence when appropriate; accepted does not mean completed |
| Event envelope | server instance, session/project/operation IDs, epoch, monotonic sequence, typed payload; stream-local ordering explicit |
| Snapshot+subscribe | atomic snapshot and sequence watermark plus buffered subsequent events, or subscribe-first barrier; no gap between fetch and listener installation |
| Resume | last acknowledged server instance/sequence; replay durable tail or return `ResyncRequired` with snapshot token; restart invalidates stale transient sequence/epoch |
| Errors | stable code, retryability, safe message, operation/recovery context; distinguish conflict, denied, cancelled, missing, corrupt, provider failure, transport loss and persistence failure |
| Cancellation | targets operation ID; idempotent; responds requested/settled with actual outcome; cannot imply filesystem rollback |
| Approval | opaque request ID bound to operation/arguments hash/workspace/session epoch/revision, expiry and choices; duplicate/stale answers rejected |
| Artifact access | opaque artifact ID/cursor and bounded byte range; metadata first; authenticated host stream/asset endpoint for large binary data, never arbitrary path access |

Suggested command families: project open/import/list; session create/resume/fork/select/snapshot; prompt/steer/follow-up/cancel; task/plan/approval actions; files read/save/diff; artifact list/search/page; memory inspect/mutate; model list/select/runtime settings; process/build/run/stop/logs; diagnostics export. A command family is not permission to expose unrestricted shell/fs IPC. The existing RPC command set maps into this interface but needs worker adapter additions for evidence/task/runtime APIs.

Durable operations record a command ID and committed result so retries after connection loss do not repeat file writes. Tools with external side effects record admitted intent before launch and final outcome after completion. A crash in between yields **unknown/interrupted**, requiring inspection, not automatic replay. Do not promise exactly-once external effects.

Initially schema generation can use one reviewed schema source with Rust/TS checks; choose a maintained binding tool only after a small serialization fixture proves tagged unions, optional fields, u64 IDs and errors. JS cannot exactly represent arbitrary 64-bit integers: use opaque string IDs/decimal sequences where necessary. Domain timestamps are not ordering keys.

## Persistence direction

Recommend a single Rust storage service with SQLite transactions for the journal, task/plan/memory/index metadata and a filesystem content-addressed blob store. The reason is the current collection of inconsistent JSON replacements, source-of-truth fragmentation and concurrent append complexity—not a belief that SQLite is automatically faster. Preserve branch parent IDs, custom records, model/thinking changes, compaction trajectory references, artifact IDs and canonical source references. Keep source files/Git separate from the application database.

Implement an importer from reference JSONL v1–v3, memory v1–v3, artifact records/blobs/indexes/keys/leases and needed checkpoint/job metadata into a **new destination**. Preserve original bytes, hash manifests and unknown extension record payloads. Import is idempotent, restartable and versioned. Cursor compatibility needs imported store/session identity and key material or an explicit old-cursor compatibility reader; never silently issue a cursor pointing to different bytes. Derivable catalog indexes may rebuild; immutable evidence cannot.

Publish blobs by write+sync+atomic rename before committing referencing metadata. Crashes may leave unreferenced blobs; startup reconciliation may collect them after a safety interval. Never commit a reference before its blob is recoverable. Branch/fork/import must transfer the authorized artifact closure, including cursor compatibility, or deliberately share immutable records under explicit branch authorization; B04 proves simply copying a journal is insufficient. Single-writer storage and deliberate transaction boundaries apply across task/episode/artifact publication. WAL/checkpoint/durability settings must be documented and tested on supported filesystems; no false power-loss claims.

During migration, use **one writer per state family**. Keep TS authoritative first; run Rust readers/compilers in shadow against captured immutable inputs. Transfer a complete family's authority only after import/replay parity; never let SQLite and TS JSON journals write through to each other opportunistically. A rollback chooses the prior writer using an explicit checkpoint/export, not divergent dual-write reconciliation.

## Runtime/context preservation

Port immutable evidence storage and source snapshot correctness before the compiler depends on them. Then port deterministic retrieval, task/epistemic state, memory selection, episodes and request compiler with golden/replay tests. Preserve reduction order, recent tool pairs, lease budgets, stable prefix and explicit over-target handling. Source bytes and digest must originate in one validated snapshot (B03). A learned reranker or new context algorithm is out of scope until parity and quality gates pass.

Port provider normalization and scheduler behind captured fake stream fixtures. Keep local process runtime independent from provider wire adapters: an external OpenAI-compatible endpoint need not be an owned llama process. Model switching has explicit unload/start/health/identity/ready/failed states, reconciles in-flight work, and records actual model/context/vision capability. Requesting a model name without verifying server identity is insufficient.

The runtime state machine must subsume current mode-owned addenda/completion decisions without silently changing mode behavior. Preserve manual/auto compaction, emergency reduction, retry arbitration, pending persistence barriers, XML fallback, tool policy and subagent isolation. Long-running detached jobs remain jobs with a recovery policy, not hidden promises owned by a React hook.

## General project-runtime model

AG1 already supports multi-language `check` and a Godot native sidecar; Dyad contributes a mature create/load/files/install/run/preview/logs/history workflow. Therefore use a `Project` with one or more typed targets, not `Project.url`.

| Concept | Minimal responsibilities |
|---|---|
| Project identity | canonical workspace root, repository/worktree identity, settings and discovered targets |
| Runtime target | stable ID, kind, working directory, required toolchain, explicit capabilities and lifecycle |
| Build/run adapter | derive argv/environment, package/dependency operations, structured diagnostics, readiness and shutdown policy |
| Preview capability | web URL with isolated session; external/native window launch/focus/observe; artifact-based visual evidence; or no visual preview |
| Verification capability | compiler/test/runtime/playtest/visual kinds with source/target revision and captured evidence |

Start with web (React/Vite or detected project command), Rust/Tauri desktop and generic command/native targets. Preserve Godot host transactions as an adapter contract; do not force native editing into generic text-only tools. Adapters have typed errors and capabilities, not scattered `if tauri` conditionals in chat/UI code. Avoid a generalized plugin framework before these concrete cases prove the interface.

For Tauri projects, discover frontend/package config and `src-tauri/Cargo.toml`, Tauri configuration, capability files, plugins/permissions/commands, windows and app state. Supervise frontend dev server plus Cargo/Tauri dev process as one cancellable resource group with independent diagnostics. Parse Cargo JSON compiler diagnostics, frontend build output and native process exits. Build/packaging actions expose target/toolchain requirements. `invoke`, events and channels are code-understanding concepts and protocol fixtures, not direct authority grants to generated apps. External binaries need per-target packaging; Tauri supports sidecars but that does not solve ownership/reaping automatically. [Sidecar packaging](https://v2.tauri.app/develop/sidecar/).

Untrusted web previews must have no AG1 commands/secrets. Restrict capabilities to trusted application windows; do not match arbitrary remote URLs or preview labels to privileged command scopes. Validate paths/arguments in Rust even when frontend types exist. [Tauri capabilities](https://v2.tauri.app/security/capabilities/), [capability schema](https://v2.tauri.app/reference/acl/capability/).

## Frontend architecture and operational surfaces

React router and query cache store **projections**, Jotai stores local presentation choices. A transport client maps generated protocol types into UI view models; components never import core persistence structures or Dyad Pro types. Optimistic UI is allowed only with pending markers and version-aware reconciliation; completion/freshness/approval are core facts. Lazy-load Monaco, diff and terminal; virtualize messages and large file/log lists. Reuse the mapped CSS/primitives and decoupled component layouts.

Add only the status strip and Task/Context/Artifacts-memory/Activity drawer described in UI_REUSE_MAP. Retrieval events link to exact source revisions, evidence pages and stale reasons. Model details distinguish resident context limit, estimated input, actual token/cache usage, generation/prompt throughput and retry health. Progress relates to real task/checkpoints, not decorative percentages.

## Rejected shortcuts and decision gates

- Full Dyad fork/backend swap: rejected because agent schema/runtime/Pro/Electron coupling would define the product internally.
- Big-bang Rust rewrite: rejected because context/recovery correctness emerges across many owners and existing tests are valuable oracles.
- Per-session TS workers with independent local schedulers: rejected as an initial concurrent architecture; single-slot contention would be uncontrolled.
- Permanent daemon by default: deferred until required by lifetime/remote-client evidence; library/protocol keeps the option available.
- Reimplement React widgets in Rust: rejected; no observed benefit to offset lost proven UI behavior.
- Mandatory Chromium bundle: rejected for shell; optional browser automation has a separate dependency/license/performance budget.

Before implementation plans freeze, resolve preview platform capabilities, target persistence durability, extension compatibility scope and numerical performance budgets against the specified experiments. These are bounded planning decisions with proposed defaults here; they do not justify beginning implementation during the research/planning stage.
