# AG1 Desktop Vision — Rust/Tauri Core with Dyad-Derived UI

## Purpose

This document defines the vision and research mandate for creating a new desktop-native evolution of AG1.

The existing AG1 source code is located at:

`/home/alex/Projects/Cruzible/`

Treat that repository as the authoritative source of AG1's current behavior, capabilities, architectural decisions, long-horizon machinery, and accumulated implementation knowledge.

The desired UI/UX reference is Dyad:

`https://github.com/dyad-sh/dyad`

Dyad is to be used primarily as a visual and interaction blueprint. Its UI should inform the new application's appearance, layout, interaction patterns, project workflow, preview experience, settings surfaces, dialogs, navigation, and general product feel.

The final product must NOT simply be "Dyad with AG1 bolted into it."

The intended result is a new AG1 desktop application with:

- AG1 as the authoritative intelligence/runtime architecture.
- Rust as the long-term systems/core implementation language.
- Tauri 2 as the desktop application boundary.
- React + TypeScript + HTML/CSS as the primary frontend/UI stack where practical.
- Portable Dyad frontend code reused when legally and architecturally appropriate.
- Electron/Node-specific Dyad machinery removed rather than inherited unnecessarily.
- A clean, lightweight, highly optimized architecture designed around AG1 rather than around Dyad's original agent implementation.
- A core that remains independent from the GUI so AG1 is not reduced to "a React app with agent logic hidden inside callbacks."

This first task is RESEARCH AND ARCHITECTURE ONLY.

Do not begin the production rewrite.

Do not modify the existing AG1 repository during this research task.

Do not start implementing the new product.

Your job is to inspect both systems deeply enough that you can produce an implementation-grade `MASTER-PROMPT.md` for a fresh Astra instance.

---

# 1. High-Level Vision

The desired product is conceptually:

```text
┌──────────────────────────────────────────────────────────────┐
│                       AG1 Desktop                            │
│                                                              │
│         Dyad-derived visual language and workflow             │
│                                                              │
│ React + TypeScript + HTML/CSS                                │
│                                                              │
│ Projects | Chat | Files | Diffs | Preview | Settings | Git   │
│ Tasks | Artifacts | Context/CV | Diagnostics | Runtime       │
└───────────────────────────┬──────────────────────────────────┘
                            │
                    Tauri 2 typed boundary
                            │
┌───────────────────────────▼──────────────────────────────────┐
│                         AG1 Core                              │
│                            Rust                              │
│                                                              │
│ Agent runtime                                                 │
│ Context virtualization                                        │
│ Context compiler                                              │
│ Retrieval                                                     │
│ Structured task state                                         │
│ Persistent memory                                             │
│ Artifacts                                                     │
│ Scheduler                                                     │
│ Tool runtime                                                  │
│ Model/runtime supervision                                     │
│ Recovery                                                      │
│ Provenance/freshness                                          │
│ Project/source understanding                                  │
│ Long-horizon execution                                        │
│ Persistence                                                   │
│ Diagnostics/telemetry                                         │
└───────────────────────────┬──────────────────────────────────┘
                            │
               FS | Git | PTY | Processes | Models
```

This diagram is a vision, not a mandate for exact crate names or process topology.

You must derive the actual target architecture from repository evidence.

---

# 2. Sources of Truth

There are two source systems, and they serve different roles.

## 2.1 AG1 / Cruzible

Path:

`/home/alex/Projects/Cruzible/`

AG1 is the source of truth for:

- agent behavior
- orchestration
- long-horizon execution
- context handling
- context virtualization
- retrieval
- source understanding
- structured state
- memory
- artifact handling
- model interaction
- tool execution
- planning behavior
- recovery behavior
- scheduling
- persistence
- debugging/telemetry
- failure handling
- any mechanisms that allow AG1 to remain coherent over long runs
- any runtime behaviors or invariants that make AG1 meaningfully different from a generic coding agent

Do not assume the list above is complete.

The repository itself is authoritative.

Your job is to discover EVERYTHING currently making AG1 work.

You must distinguish:

1. Active production code.
2. Partially implemented systems.
3. Legacy/replaced systems.
4. Experimental systems.
5. Dead code.
6. Planned systems described in docs but not actually active.
7. Behaviors that emerge from interactions between multiple modules rather than from one obvious subsystem.

Do not treat comments, README claims, or old design documents as equivalent to active implementation.

Trace actual execution paths.

## 2.2 Dyad

Clone/fork the current public Dyad repository for inspection.

If GitHub tooling and permissions permit creating an actual remote fork, do so.

If remote fork creation is unavailable, create a clean local clone and a dedicated research branch. Do not let lack of remote-fork capability block the research.

Dyad is the source of truth primarily for:

- visual composition
- layout
- spacing
- typography
- theming
- navigation
- chat presentation
- project selection/management UI
- file browsing
- editor surfaces
- diff/review surfaces
- preview surfaces
- dialogs
- settings
- notifications
- panel sizing
- resizable layouts
- interaction flow
- polished frontend behavior
- useful desktop product patterns

Dyad is NOT the source of truth for AG1's intelligence architecture.

Do not inherit Dyad's agent machinery merely because it already exists.

Do not inherit Electron/Node architecture merely because Dyad uses it.

---

# 3. Non-Negotiable Target Stack

The long-term target is:

## Frontend / UI

- React
- TypeScript
- HTML
- CSS
- Existing portable Dyad frontend code where appropriate
- Existing mature libraries where they reduce unnecessary reinvention

Do not rewrite proven React/CSS UI into native Rust widgets merely for ideological purity.

The goal is a lightweight desktop application, not a "Rust everywhere" vanity project.

## Desktop Boundary

- Tauri 2

The final application should not depend on Electron.

Do not bundle a full Chromium runtime unless a later repository-grounded finding demonstrates an unavoidable reason.

## Systems/Core

- Rust

AG1's long-term core/runtime should be implemented in Rust.

However, if research shows that a staged migration temporarily keeping a specific TypeScript subsystem is substantially safer, document that as an intermediate migration strategy.

The final architecture should still converge on a Rust-owned AG1 core.

---

# 4. Core Architectural Principle

Do not create a Frankenstein merge.

This is not:

```text
Dyad + AG1 glued together
```

It is:

```text
Dyad
 ↓
extract/reuse visual language and portable frontend pieces

AG1
 ↓
extract/preserve capabilities, behavior, invariants, and proven architecture

                ↓

        NEW AG1 DESKTOP PRODUCT
```

The new product should eventually be understandable as:

"AG1 Desktop, whose visual ancestry came from Dyad"

rather than:

"a Dyad fork with increasingly invasive AG1 patches."

---

# 5. Preserve AG1 Behavior, Not AG1 Implementation Accidents

AG1's successful behavior and capabilities are sacred.

Implementation mistakes are not.

The rewrite must preserve:

- intended behavior
- long-horizon correctness
- state continuity
- context/retrieval semantics
- provenance
- freshness rules
- artifact behavior
- tool semantics
- recovery behavior
- model interaction expectations
- relevant user-facing workflows
- any proven performance/correctness properties

But if you discover a confirmed:

- bug
- race condition
- stale-state bug
- incorrect invalidation rule
- data-loss risk
- broken recovery edge case
- unnecessary hot-path allocation
- avoidable serialization bottleneck
- lock contention issue
- duplicated computation
- brittle persistence behavior
- deadlock possibility
- correctness flaw
- architectural contradiction
- known harness failure mode

then the new implementation must NOT intentionally reproduce it.

Document the issue.

Explain why it is a defect rather than an intentional semantic choice.

The future implementation must include a regression test proving the corrected behavior.

Do not use the rewrite as an excuse to casually redesign working AG1 behavior.

Evidence is required.

---

# 6. AG1 Capability Discovery Requirements

The following areas are known to be important enough that they MUST be explicitly investigated.

This is not permission to assume their exact current form.

Verify each one against the repository.

## 6.1 Agent Runtime

Investigate:

- main agent loop
- turn lifecycle
- message construction
- tool-call lifecycle
- tool result handling
- error handling
- retry behavior
- cancellation
- timeouts
- agent completion conditions
- continuation logic
- long-running task behavior
- session persistence
- resume behavior
- any recursive or nested agent behavior

## 6.2 Context System

Investigate:

- context construction
- context budgeting
- compaction
- legacy compaction
- context compiler
- system prompt construction
- project context
- recent history handling
- tool-result retention
- file/source inclusion
- token accounting
- context overflow protection
- long-session behavior

## 6.3 Context Virtualization

Inspect the actual current implementation and determine which pieces are active.

Explicitly look for mechanisms related to:

- externalized context
- artifact externalization
- structured task state
- objectives
- constraints
- decisions
- facts
- hypotheses
- provenance
- source capsules
- revision awareness
- lexical source information
- symbol information
- relationships
- Git/revision state
- recent-access state
- immutable artifacts
- artifact rehydration
- deterministic retrieval
- freshness gates
- stale-fact handling
- canonical provenance
- invalidation
- retrieval budgets
- context expansion/consolidation
- episodic memory if present
- allocator/GC behavior if present
- delta context if present
- reranking if present

Do not assume planned CV work is already implemented.

Classify each feature as:

- active
- partial
- experimental
- legacy
- absent/planned

## 6.4 Retrieval and Source Understanding

Trace:

- how files are selected
- symbol lookup
- semantic/lexical retrieval
- source relationships
- changed-file detection
- revision invalidation
- cache invalidation
- provenance
- stale knowledge prevention
- forbidden/stale retrieval handling
- exact-symbol retrieval
- vague-follow-up recovery
- changed-file recovery
- how retrieval integrates with the context compiler

## 6.5 Structured Task State

Investigate:

- objectives
- constraints
- decisions
- facts
- hypotheses
- task progress
- plan mode
- state ownership
- serialization
- persistence
- provenance
- state mutation
- state recovery
- stale state
- cross-turn continuity

## 6.6 Persistent Memory

Inspect:

- memory formats
- memory kinds
- write/update rules
- injection rules
- caps/eviction
- subagent memory interaction if any
- project vs global memory
- stale/incorrect memory handling
- persistence
- recovery

## 6.7 Artifact System

Inspect:

- artifact creation
- storage
- immutable/mutable distinction
- metadata
- provenance
- rehydration
- retrieval
- lifecycle
- cleanup
- file associations
- large outputs
- binary artifacts if supported
- failure behavior

## 6.8 Scheduler / Inference Coordination

Inspect:

- inference scheduling
- single-model scheduling
- queueing
- concurrency
- task isolation
- subagents if present
- model residency assumptions
- batch behavior
- contention
- cancellation
- fairness
- prioritization
- failure recovery

## 6.9 Model Runtime

Inspect:

- provider abstraction
- local-model paths
- OpenAI-compatible APIs
- llama.cpp integration
- runtime configuration
- context-size management
- KV-cache assumptions
- GPU-layer configuration
- process launch
- model-server lifecycle
- health checking
- crash detection
- reconnect/restart
- logs
- token throughput
- prompt-processing metrics
- model switching
- streaming
- backpressure

## 6.10 Tool Runtime

Inspect:

- tool registration
- schemas
- dispatch
- validation
- permission/approval model
- shell execution
- file operations
- Git
- search
- web/browser tools if present
- timeout behavior
- cancellation
- tool result shaping
- artifact capture
- retries
- error propagation
- audit logs

## 6.11 Failure Recovery

Inspect:

- retries
- recoverable vs fatal failures
- model crashes
- tool crashes
- process crashes
- malformed model output
- interrupted writes
- partial state
- checkpointing
- resume semantics
- corruption prevention
- log/diagnostic capture
- long-session recovery

## 6.12 Persistence

Inspect:

- JSON/state files
- databases
- checkpoints
- session logs
- artifact storage
- transactional guarantees
- atomic writes
- migration/versioning
- corruption handling
- startup recovery
- cleanup
- stale files

## 6.13 Git / Revision Awareness

Inspect:

- Git state tracking
- changed files
- diffs
- revision-aware caches
- branch/worktree assumptions
- repository identity
- source-capsule invalidation
- rollback/checkpoint interaction
- provenance

## 6.14 Logging / Diagnostics / Cost / Telemetry

Inspect:

- agent logs
- debug logs
- token accounting
- cost tracking
- tool traces
- runtime traces
- model metrics
- replay/evaluation logs
- failure diagnostics
- performance instrumentation

## 6.15 Replay / Evaluation

Inspect any systems for:

- deterministic replay
- long-session replay
- retrieval evaluation
- freshness checks
- forbidden-result checks
- artifact rehydration tests
- benchmark suites
- regression fixtures
- harness evaluation

## 6.16 User Interface Coupling

Determine which parts of AG1 are currently entangled with:

- TUI rendering
- terminal-only input
- terminal state
- slash commands
- keyboard shortcuts
- synchronous console output
- ANSI formatting

The new core must not depend on a graphical UI, but neither should it remain accidentally dependent on a TUI.

Identify a clean UI-independent protocol/API boundary.

---

# 7. Dyad Exploration Requirements

Do not inspect Dyad superficially.

Trace the actual implementation paths.

## 7.1 Visual System

Map:

- app shell
- layout
- navigation
- home/project screen
- chat
- sidebar
- file tree
- editor
- diff/review UI
- preview
- project cards
- settings
- model picker
- dialogs
- notifications
- resizable panes
- status indicators
- themes
- typography
- spacing
- colors
- icon system
- loading states
- empty states
- error states
- transitions/animation
- responsive behavior

Identify which parts can be directly reused in a Tauri/React app and which are coupled to Electron or Dyad-specific machinery.

## 7.2 Frontend Architecture

Inspect:

- React component hierarchy
- state management
- data stores
- query/cache layers
- routing/navigation
- hooks
- shared components
- styling system
- theme tokens
- dependency injection/context
- IPC wrappers
- command abstractions
- event subscriptions
- frontend tests

## 7.3 Electron Coupling

Find every meaningful dependency on:

- Electron main process
- preload scripts
- context bridges
- Electron IPC
- Node APIs exposed to renderer
- native Node modules
- window management
- system dialogs
- clipboard
- notifications
- process spawning
- filesystem access
- updater
- packaging

Determine what must be reimplemented as:

- Tauri commands
- Tauri events/channels
- Rust services
- Tauri plugins
- standard frontend code

## 7.4 Project Runtime

Trace end-to-end:

1. Project creation.
2. Project loading.
3. File read/write.
4. Package installation.
5. Dev-server startup.
6. Preview startup.
7. Runtime output capture.
8. Error capture.
9. Agent iteration.
10. Process shutdown.
11. Project persistence.
12. Checkpoint/version history.

Even if Dyad's original runtime is not adopted, these flows are part of the UX blueprint and may contain reusable product behavior.

## 7.5 Agent/UI Coupling

Identify:

- which UI elements depend directly on Dyad's agent types
- which state stores assume Dyad's model/session representation
- which components can survive an AG1 backend replacement
- which pieces should be rewritten around an AG1 protocol

Do not preserve Dyad-specific agent concepts merely to avoid frontend refactoring.

---

# 8. Licensing and Provenance Audit

Before copying code or assets, inspect licensing carefully.

Dyad has historically separated permissively licensed open-source code from separately licensed Pro functionality.

Do not assume the entire repository can be reused identically.

Create an explicit license/provenance audit.

At minimum identify:

- repository-level license
- `src/pro` or equivalent restricted areas
- third-party components
- fonts
- icons
- illustrations
- bundled assets
- copied design resources
- dependencies with redistribution restrictions
- code that is safe to reuse
- code that should only be treated as a visual reference
- required notices/attributions

The final AG1 Desktop implementation must have a clean provenance story.

Do not copy restricted/proprietary Dyad code merely because it is present in the repository.

---

# 9. Lightweight / Performance Vision

"Super lightweight" is a real engineering requirement.

Do not reduce it to vague claims.

The new desktop shell should aim to be materially lighter than Dyad's Electron application in:

- idle system RAM
- idle CPU
- process count
- startup time
- binary/install footprint
- background activity
- GPU memory where practical
- frontend event/render overhead

The AG1 core itself should be designed for:

- low allocation rates
- minimal unnecessary copying
- efficient serialization
- bounded queues
- backpressure
- low lock contention
- low idle CPU
- predictable memory growth
- fast context compilation
- fast retrieval
- efficient artifact I/O
- fast persistence
- fast startup/resume
- resilient process supervision
- efficient token streaming
- efficient filesystem watching
- efficient Git operations
- stable long-session memory behavior

Do not use "zero-copy," "lock-free," async, arenas, custom allocators, or similar techniques merely because they sound optimized.

Optimization must be evidence-based.

Correctness, determinism, maintainability, and clear ownership boundaries come first.

---

# 10. Rust Craftsmanship Standard

The new core should be designed as serious systems software.

Investigate and plan for:

- explicit ownership boundaries
- strong domain types
- typed errors
- minimal `unwrap()`/panic in production paths
- RAII
- structured concurrency
- cancellation
- bounded channels
- backpressure
- clear async boundaries
- minimal shared mutable state
- explicit state machines where lifecycle matters
- transactional persistence where needed
- atomic file replacement where needed
- crash-safe writes
- recoverable startup
- deterministic testing
- property tests where valuable
- fuzzing where valuable
- benchmarks
- tracing/instrumentation
- profile-guided optimization only where justified
- graceful shutdown
- resource cleanup
- process reaping
- no orphaned child processes
- no hidden global mutable state
- versioned persistence formats
- explicit migrations

Avoid overengineering.

Sophisticated craftsmanship means making complexity explicit and controlled, not maximizing abstraction count.

---

# 11. Frontend Craftsmanship Standard

The frontend should remain visually close to Dyad while becoming an AG1-native interface.

Prefer reuse over needless recreation when licensing and coupling allow it.

The frontend must not own AG1's core state.

The UI should:

- render state
- send commands
- subscribe to events
- present approvals
- present diffs
- present artifacts
- present diagnostics
- present runtime state
- expose AG1-specific observability

The UI must not become the source of truth for:

- task state
- agent state
- CV
- memory
- artifact lifecycle
- model lifecycle
- tool lifecycle
- recovery state

Those belong to the core.

---

# 12. AG1-Specific Desktop Surfaces

The new UI should preserve Dyad's core visual language, but AG1 requires additional surfaces.

Research and design how to expose, without clutter:

- active context usage
- virtualized context size
- retrieval events
- retrieval provenance
- freshness/staleness
- revision mismatches
- structured task state
- objectives
- constraints
- decisions
- facts
- hypotheses
- artifacts
- rehydration
- memory
- tool activity
- scheduler state
- retries
- failures
- recovery
- model health
- local model runtime status
- token throughput
- prompt-processing throughput
- context limits
- long-running task progress
- subagents if present
- Git/revision state
- diagnostics

These should be useful operational surfaces, not decorative dashboards.

Do not redesign the entire Dyad UI during the research stage.

Determine the smallest coherent additions.

---

# 13. Native App / Project Runtime Direction

AG1 Desktop should not be limited to editing files.

It should be capable of supervising running projects and applications.

Because the desired development environment may produce both web and native applications, the architecture should not hardcode:

```text
project = URL
```

Investigate a generalized project/runtime target model.

Examples of concepts to evaluate:

- ProjectTarget
- RuntimeAdapter
- PreviewProvider
- BuildProvider
- PackageManager
- DevRuntime
- ApplicationTarget
- DiagnosticsProvider

Do not force these names.

Derive appropriate abstractions from the actual code.

The architecture should make Tauri/Rust desktop projects first-class without making Tauri a special-case conditional scattered everywhere.

---

# 14. Tauri Desktop Development Awareness

The future AG1 Desktop should be able to create and work on Tauri 2 applications.

Research what the core will eventually need to understand and supervise:

- React/Vite frontend
- `src-tauri`
- `Cargo.toml`
- Rust dependencies
- Tauri configuration
- capabilities
- permissions
- plugins
- commands
- `invoke`
- events
- channels
- windows
- application state
- dev process
- Cargo compiler diagnostics
- frontend build errors
- native crashes
- production builds
- packaging

This research belongs in the target architecture.

Do not implement it during this task.

---

# 15. Model Runtime Direction

## User requirement recorded 2026-09-04: preserve the tuned llama.cpp runtimes

Reuse the user's existing llama.cpp builds and model-specific tuning. They are a required compatibility baseline for this migration. Rust takes over AG1 orchestration and supervision; it must continue using those inference backends. Do not replace them with stock/upgraded llama.cpp, rebuild them, retune them, replace model weights/projectors, or silently fall back to another backend as part of the migration. Preserve each model's resolved executable, shared libraries, launch arguments/environment, context/KV/offload/batch/speculation settings and request-side model behavior. Any later backend/tuning change is separate work requiring explicit user authorization and measured comparison.

See `planning/LLAMA_CPP_PRESERVATION.md` and `planning/evidence/llama-runtime-lock.json` for the observed three-model configuration, fingerprints, invariants and remaining runtime-verification gates. Configuration evidence is not proof of the currently running model. Planning must not disturb existing model processes or modify the runtime directories/configuration.

AG1's local-model capabilities should be treated as first-class systems functionality.

Investigate whether the future core should own:

- llama.cpp process lifecycle
- model-server startup
- model-server health
- OpenAI-compatible API adapters
- local endpoint discovery
- context configuration
- KV-cache configuration
- GPU-layer configuration
- crash restart
- logs
- resource metrics
- token streaming
- throughput metrics
- model switching
- model residency
- queueing

Do not blindly inherit Dyad's provider abstraction if AG1 already has a stronger model/runtime concept.

---

# 16. Core/UI Protocol

A major deliverable of the research is a proposed boundary between the Rust core and React frontend.

Investigate whether this should be:

- Tauri commands
- Tauri channels/events
- an internal service layer
- a typed protocol
- a separate local daemon
- an embedded Rust core
- some hybrid

The final choice must support:

- streaming model output
- tool events
- process output
- diagnostics
- state snapshots
- incremental state updates
- cancellation
- approvals
- file changes
- runtime lifecycle events
- large artifacts without wasteful copies
- crash/restart behavior
- future non-GUI clients if desirable

Do not choose the architecture before inspecting AG1's actual runtime shape.

---

# 17. Preserve UI Independence

AG1 Core should remain usable independently from the desktop UI.

Closing the GUI should not conceptually redefine AG1's state model.

At minimum, the architecture should not make future support for these impossible:

- CLI
- TUI
- headless mode
- API client
- remote client

Do not implement all of those now.

Just avoid making the new Rust core impossible to use without React.

---

# 18. Required Baseline Measurements

Before recommending performance optimizations, measure or construct reproducible baselines where practical.

For current AG1, investigate measurable baselines for:

- cold startup
- idle RAM
- idle CPU
- context compilation latency
- retrieval latency
- artifact lookup/rehydration
- persistence latency
- Git status/diff latency
- file indexing/search
- scheduler overhead
- tool dispatch overhead
- model streaming overhead
- event throughput
- long-session memory growth
- crash/recovery behavior
- shutdown behavior
- replay/evaluation runtime

For Dyad, measure or document where practical:

- cold startup
- idle RAM
- idle CPU
- process count
- rough GPU/VRAM footprint if measurable
- packaged size
- main UI responsiveness

If direct benchmarking is not possible, document why and define how the future implementation phase should measure it.

---

# 19. Testing Philosophy

The rewrite must be testable against AG1's existing behavior.

Research and propose:

- unit tests
- integration tests
- end-to-end tests
- deterministic replay tests
- regression fixtures
- golden tests where appropriate
- property tests
- crash/recovery tests
- persistence migration tests
- long-session tests
- retrieval correctness tests
- freshness/invalidation tests
- artifact rehydration tests
- tool runtime tests
- scheduler tests
- model-process lifecycle tests
- frontend/core protocol tests
- UI smoke tests
- performance benchmarks

Where possible, identify existing AG1 tests that can become parity oracles.

---

# 20. Migration Strategy Principles

The master plan should favor incremental parity over a heroic big-bang rewrite.

The original AG1 must remain a reference implementation until parity is proven.

The research should determine safe migration seams.

Possible intermediate architecture:

```text
Tauri/Rust host
      │
      ├── native OS services
      ├── process supervision
      └── protocol
             │
             ▼
     existing AG1 TypeScript
```

followed by staged subsystem migration into Rust.

This is only an example.

Do not force it if repository evidence suggests a better route.

Each migration phase should leave the new product in a coherent state.

Avoid phases that require dozens of unrelated subsystems to be half-migrated simultaneously.

---

# 21. Research Safety Rules

During this task:

1. Treat `/home/alex/Projects/Cruzible/` as read-only.
2. Do not "prepare" AG1 for migration by modifying it.
3. Do not begin the Rust rewrite.
4. Do not redesign Dyad.
5. Do not add Tauri to Dyad.
6. Do not delete Dyad code.
7. Do not make speculative architectural changes.
8. Do not silently assume old docs describe active AG1 behavior.
9. Do not claim parity without tracing execution.
10. Do not reproduce known bugs merely for compatibility.
11. Do not change AG1 semantics just because Rust makes another design attractive.
12. Do not overfit the target architecture to a single current model provider.
13. Do not make the React frontend authoritative for core state.
14. Do not copy restricted/proprietary Dyad code.
15. Do not let research devolve into implementation.

---

# 22. Mandatory Research Deliverables

Create a research directory in the NEW planning workspace, not inside the original AG1 repository unless explicitly instructed otherwise.

Produce at minimum:

## `research/AG1_ARCHITECTURE.md`

A repository-grounded description of AG1.

Include:

- entry points
- runtime lifecycle
- subsystem map
- data flow
- state ownership
- persistence
- concurrency
- context flow
- model flow
- tool flow
- recovery flow
- active vs legacy systems
- exact file paths
- important types/functions/classes/modules
- architectural debt
- known uncertainty

## `research/DYAD_ARCHITECTURE.md`

A repository-grounded description of Dyad relevant to reuse.

Include:

- desktop shell
- React component architecture
- state management
- Electron boundaries
- IPC
- filesystem/process access
- project lifecycle
- preview system
- editor/diff surfaces
- settings
- reusable frontend
- tightly coupled pieces
- exact file paths

## `research/AG1_PARITY_MATRIX.md`

This is mandatory and must be exhaustive.

For every meaningful AG1 capability, record:

- capability
- current owner/path
- current status
- dependencies
- state/persistence ownership
- observable behavior
- correctness invariants
- proposed target owner
- migration difficulty
- required tests
- phase dependency
- notes/risks

No major subsystem should be allowed to disappear simply because it was difficult to notice.

## `research/DYAD_UI_REUSE_MAP.md`

Map Dyad UI elements into:

- directly reusable
- reusable after decoupling
- visual-reference-only
- Electron-bound
- Dyad-agent-bound
- restricted-license / do not reuse
- unnecessary for AG1

Include exact paths.

## `research/LICENSE_AUDIT.md`

Document:

- license boundaries
- Pro/restricted code
- third-party assets
- fonts/icons
- redistribution obligations
- reusable code
- prohibited/unclear code
- attribution requirements

## `research/PERFORMANCE_BASELINE.md`

Document:

- measured AG1 baselines
- measured Dyad baselines where practical
- measurement methodology
- hardware/environment assumptions
- unmeasurable items
- future target metrics
- likely bottlenecks

## `research/BUGS_AND_TECH_DEBT.md`

Document confirmed or strongly evidenced:

- correctness bugs
- race risks
- performance issues
- stale-state issues
- persistence risks
- recovery weaknesses
- duplicated systems
- legacy systems
- architectural contradictions

For every item, distinguish:

- confirmed bug
- likely bug
- optimization opportunity
- intentional design tradeoff
- unknown

Do not "fix" these during research.

## `research/INVARIANTS.md`

Define the properties the rewrite must never violate.

Examples to verify/derive:

- core state must not depend on GUI lifetime
- task state must survive appropriate restarts
- retrieval freshness semantics must be preserved
- provenance must not be silently lost
- stale facts must not become current facts
- artifacts must remain recoverable
- model crashes must not corrupt persistent state
- tool execution must remain auditable
- UI rendering must not block agent execution
- cancellation must have defined semantics
- process children must be reaped
- persistence must be crash-safe where required

Derive the actual invariants from AG1.

## `research/TARGET_ARCHITECTURE.md`

Design the proposed final architecture only after both repositories have been mapped.

Include:

- Rust crate/module boundaries
- frontend boundaries
- Tauri boundary
- core/UI protocol
- state ownership
- persistence
- concurrency model
- process model
- model runtime ownership
- tool runtime
- context/CV
- retrieval
- artifacts
- scheduler
- project runtime
- diagnostics
- testing
- migration seams
- why each major boundary exists

Do not invent arbitrary micro-crates.

Prefer coherent ownership.

## `research/MIGRATION_RISKS.md`

Identify:

- highest-risk rewrites
- parity risks
- data migration risks
- context semantic risks
- scheduler/concurrency risks
- local-model/runtime risks
- UI coupling risks
- licensing risks
- performance regression risks
- long-horizon regression risks
- recovery risks
- cross-platform risks

Include mitigation strategies.

## `research/TEST_STRATEGY.md`

Define how the Rust/Tauri product will prove:

- feature parity
- correctness
- long-horizon stability
- failure recovery
- performance
- persistence
- retrieval correctness
- freshness
- artifact behavior
- tool behavior
- model behavior
- UI/core integration

## `research/DEPENDENCY_GRAPH.md`

Show subsystem dependencies and migration ordering constraints.

This should be detailed enough to prevent impossible phase ordering later.

---

# 23. MASTER-PROMPT.md

After all research documents are complete, create:

`MASTER-PROMPT.md`

This is the most important deliverable.

It must be written for a completely fresh Astra instance that has no access to this conversation.

The fresh instance should be able to read:

- this `VISION.md`
- all `research/*.md`
- `MASTER-PROMPT.md`

and then produce the implementation roadmap.

`MASTER-PROMPT.md` must instruct the next Astra instance to:

1. Read all research artifacts.
2. Verify important repository references if needed.
3. Design the actual migration sequence.
4. Create `ROADMAP.md`.
5. Create implementation phase files.
6. Make each phase executable by a fresh model instance.
7. Define objective completion criteria.
8. Define tests for every phase.
9. Preserve AG1 parity throughout.
10. Preserve the Dyad-derived visual target.
11. Preserve the Rust/Tauri/React target architecture.
12. Keep AG1 core independent from GUI state.
13. Keep the original AG1 repository untouched unless a later migration phase explicitly requires otherwise.
14. Avoid a big-bang rewrite.
15. Prevent later phases from assuming unimplemented earlier work.
16. Include performance and regression gates.
17. Include licensing/provenance gates.
18. Include bug-fix regression tests for confirmed AG1 defects.
19. Avoid speculative features unrelated to the migration.
20. Stop after planning. The planning Astra must not immediately execute all phases.

The phase plan should be based on the real dependency graph discovered during research, not on arbitrary phase counts.

---

# 24. What "Success" Means for This Research Task

This research task is complete only when:

- Dyad has been cloned/forked and actually inspected.
- AG1 has been deeply traced from `/home/alex/Projects/Cruzible/`.
- The active AG1 architecture is understood well enough to explain end-to-end runtime behavior.
- Legacy/partial/planned AG1 systems have been separated from active ones.
- Every meaningful AG1 subsystem is represented in the parity matrix.
- Dyad's reusable UI has been mapped at file/component level.
- Electron-bound code has been identified.
- Dyad agent-bound code has been identified.
- licensing boundaries have been audited.
- current performance has been measured or measurement plans have been defined.
- confirmed bugs/risks have been documented.
- target invariants have been written.
- the proposed Rust/Tauri/React architecture has been justified by evidence.
- migration dependencies have been mapped.
- a test strategy exists.
- migration risks are explicit.
- `MASTER-PROMPT.md` is complete enough for a fresh Astra instance to produce a phase-by-phase implementation roadmap without relying on this conversation.

Do not judge success by document length.

Judge it by whether a fresh engineer/model could understand both systems and plan the rewrite without repeating the entire discovery effort.

---

# 25. Final Instruction

Approach this as a senior systems architect performing a pre-rewrite investigation of a working, evolving agent harness.

Do not rush.

Do not optimize for producing code during this session.

Do not produce generic architecture prose.

Read the repositories.

Trace real execution.

Measure where possible.

Challenge assumptions.

Preserve what makes AG1 effective.

Discard implementation accidents.

Use Dyad's UI intelligently without inheriting its unnecessary runtime architecture.

Design for a lightweight, highly optimized, testable Rust core behind a polished Tauri + React desktop application.

The final goal is not to imitate Dyad internally.

The final goal is to give AG1 a first-class desktop body without losing anything that makes AG1 AG1.
