# Decisions for implementation

Status: planned, not implemented. The source pins and findings in `research/README.md` still apply. User requirements in VISION and LLAMA_CPP_PRESERVATION take precedence over research recommendations.

## D01 — Host and packages

Embed the GUI-independent Rust core in the Tauri host. A session actor has a bounded mailbox and one mutation owner. Blocking filesystem/Git/SQLite work uses bounded executors. Model engines, project builds, PTYs and optional browsers remain external processes. No default daemon.

Create `product/` only when implementation begins. Its Cargo workspace contains `crates/ag1-core`, `crates/ag1-storage`, `crates/ag1-platform`, `crates/ag1-protocol`, and `apps/desktop/src-tauri` (package `ag1-desktop`). P12 adds `clients/cli` (package `ag1-cli`): a small sixth package is justified as a composition host and non-GUI/JSONL compatibility proof, not another subsystem abstraction. Frontend lives in `product/apps/desktop`; temporary worker in `product/compat/agent-worker`; optional JS extension host in `product/compat/extensions`. Reference snapshots live outside product packaging inputs under `reference/`.

Core declares domain/storage/platform/provider ports. Storage and platform depend on core; protocol serializes explicitly mapped core views; desktop/CLI compose these. Core imports neither adapters nor Tauri. SQLite/HTTP/process handles cannot leak into frontend DTOs. Modules, not more crates, separate context, tools, scheduler, state and recovery.

Renderer close detaches presentation and leaves active host work running by default, with a visible tray/reopen path where supported. Explicit quit drains/cancels/checkpoints owned work; if an OS cannot support detached presentation, provide an explicit running-work choice instead of silently claiming survival. Host death requires restart reconciliation; no promise that an in-memory task survives it. External user model services are not quit-owned children.

## D02 — Protocol and transport

Use versioned Rust/Serde wire DTOs as the protocol authority, generated JSON Schema and TypeScript bindings committed together. P02 selects and pins the generation tooling only after fixtures prove tagged unions, nullable/optional fields, unknown extension payloads, error variants and decimal-string sequences. Do not hand-maintain divergent Rust and TS models. Core types and public DTOs are separate.

Tauri commands return bounded results; channels carry scoped streams. Core maintains durable command results and ordered session events. Snapshot+subscription is a single watermark/barrier operation; gaps cause replay or explicit resnapshot. Message IDs, command IDs, operation IDs and workspace/session IDs are distinct. Idempotent command acceptance is not exactly-once shell execution. Persist intent and uncertain outcome when external effects cannot be reconciled.

Initial limits: 256 queued commands per session, 128 waiting inference requests per endpoint, 1 MiB queued transient data per subscriber, 64 KiB normal JSON envelopes, 32 KiB artifact pages. Oversize prompts/attachments use explicit upload/artifact references. Limits are typed/configurable and exercised at their boundaries; P01 measures fixtures that might need an explicit larger operation type. Never truncate durable outcomes to fit a transport envelope. Coalesce progress; spool full logs; terminate/resync stalled subscriptions without losing authoritative state. Initial display flush ceiling 16 ms, subject to measured rendering cost.

Approvals bind operation, argument hash, workspace/session epoch, revision and expiry. Generated/preview windows receive no AG1 invoke scope. Auth secrets remain behind platform ports. All filesystem requests have explicit canonical workspace identity.

## D03 — Persistence and authority

Use SQLite for journal and transactional metadata, SHA-256 blobs for immutable evidence. Use WAL and `synchronous=FULL` initially; document filesystem/platform limits, fsync blob+directory before referencing transaction, integrity verification and startup orphan reconciliation. These settings are a target contract to test, not a claim of universal hardware durability.

Use one writer process per destination database, advisory ownership lock plus database transaction protections, and a session actor for ordering. Preserve branch ancestry, raw unknown JSON records, task/episode/memory distinctions and artifact authorization. Forks share immutable blobs through explicit per-branch authorization and reference roots; old cursor validation retains imported origin/store keys and checks the requesting branch's access. Old cursors must never grant unrelated session access. Retention walks all authorized branch/import roots. Derived indexes can rebuild.

Import only frozen snapshots into a new destination. Record import IDs, source hashes and checkpoints so interrupted/repeated imports are safe. Corrupt source remains retained with a diagnostic; no empty replacement. Imported process records are historical observations, not permission to kill a PID. Retain original-format archives and a tested export for rollback; unsupported new fields stay in an accompanying lossless archive and block an unsafe old-runtime resume.

TS owns executing sessions and their files until P13. P05–P12 Rust work is isolated/shadow with immutable captures; no chatty per-request TS↔Rust compiler calls, and no dual writes. P13 atomically transfers a complete session/workspace authority bundle: journal/task/memory/artifacts/runtime/scheduler/policy. Desktop-owned preferences/project catalog are separate from TS agent state from P04 onward.

## D04 — llama.cpp and providers

L01–L08 are binding. Retain all three configured backend selections and settings; no bundled generic engine replaces them. First distribution uses external, user-installed tuned runtimes referenced by an explicit local manifest. An AG1 app update must not modify those paths or run a tuning job. System loader/toolchain prerequisites are diagnosed separately.

The TS bridge keeps its existing scheduler and resolver, guarded by runtime locks and safer process ownership. Rust shadow providers never call the user's endpoint concurrently. At cutover Rust owns one endpoint permit through terminal completion or explicit server reconciliation. Within an endpoint, preserve FIFO and capacity 1 local / 3 remote by default; different endpoints can progress independently. No priority algorithm change during parity migration. Queue bounds introduce visible admission errors, not silent job loss.

Keep every active provider family and auth/usage/tool/thinking behavior represented in the P01 provider ledger and P06 fixtures. Local-first integration does not declare other providers removed. Credential-dependent live tests may be pending by provider, but cannot be represented as passed. Unsupported provider capabilities return explicit status. Proxy secrets resolve in the host, not renderer.

## D05 — Extensions and adjacent clients

Preserve skills/templates/resource precedence in core. Existing JavaScript extensions use an optional, separately supervised compatibility host implementing a declared tool/provider/hook/command/UI-request API. It may execute extension code; it cannot own the parent AgentSession, journal, scheduler, approval authority or hidden model loop. Core brokers model calls and state changes; timeout/crash settles extension operations. Pin and test custom payload preservation and import/export APIs. Arbitrary extension behavior cannot be guaranteed: P01 inventories actual API surfaces and P09/P12 provide supported/unsupported feature responses plus migration notes for every gap.

Keep terminal presentation in the existing reference client; map its product actions and mode policies to core/React. Provide a Rust CLI/JSONL adapter and explicit SDK compatibility facade in P12. Adjacent Lit web-ui, Slack mom and pods stay preserved in the reference as separately released clients; porting their UIs/services is outside desktop release. Publish compatibility status, not a false claim that they already run on Rust. Partial MCP/DAG exports remain preserved reference APIs, not default desktop integrations. Removed autonomous runner stays removed; active workspace helpers migrate.

Planning rechecked A85: `packages/coding-agent/package.json` advertises `./hooks`, but `src/core/hooks/index.ts`, `dist/core/hooks/index.js` and `dist/core/hooks/index.d.ts` are absent in the inspected original tree. Treat that export as broken/unavailable at this pin, not an active hook API to recreate. Active `extensions/*` hooks remain a separate supported-surface inventory. Preserve explicit import diagnostics and document any intentional new API instead of resurrecting the missing path silently.

## D06 — UI and runtime targets

Reuse individually audited Dyad CSS/primitives and decoupled layouts. New AG1 query/view models replace its IPC and agent schema. Keep React/Tailwind/Base UI composition, theme geometry, keyboard/focus/resize behavior, message virtualization and lazy Monaco/xterm. No Pro source/type/asset closure or branding enters product. P01 records file-level provenance; P04 verifies actual copied closure.

P04 proves isolated web preview on the development Linux platform and records platform limits early. P11 implements web dev-server, generic native, Rust/Tauri and Godot target adapters. A native application can launch/focus/stop/report artifacts without embedding its window. Browser automation is an optional separately budgeted process; shell distribution does not require Chromium. Tauri projects expose frontend and Cargo diagnostics, build/run/package capabilities and explicit toolchain errors. Component tagging, Pro annotation and Dyad cloud/billing are not required migrations.

## D07 — Acceptance and uncertainty

Adopt the provisional performance budgets in PERFORMANCE_BASELINE as release gates: ≥30% lower shell memory, ≥50% lower shell install size, <0.5% one-core idle CPU, <1.5s packaged startup p95 excluding model load, <50ms display lag p95, no >10% p95 core fixture regression after noise characterization, bounded queues and zero leaked owned children. P01/P04 establish matched baselines; thresholds may change only through a documented measurement-based decision, never by disabling quality behavior or changing tuned llama backends.

Linux x86_64 is the first tuned-runtime acceptance platform because the observed binaries are Linux ELF. Test shell/process/protocol capability on Windows/macOS too, and report platform-specific readiness. A missing runner or incompatible tuned backend is a pending gate, not a pass. Long-horizon acceptance requires 8h and 24h workloads, cold/warm context and per-model sequential quality/performance comparison.

No extra user input is needed to produce this plan. Runtime loader closure/weights/actual sampling, packaged UI baselines, extension inventory, provider credentials and OS runners remain bounded implementation evidence tasks in P01/P04/P06/P09/P14. Live model canaries must use an explicitly available endpoint window; never evict the user's service to obtain a benchmark.
