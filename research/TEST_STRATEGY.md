# Test strategy and parity oracles

Tests must demonstrate behavior, not mirror a new implementation. The reference tests are essential inputs but are not sufficient alone: B01–B04 were not caught by the selected existing suite and required additional probes. The rewrite must preserve intended behavior while correcting confirmed defects.

## Existing reference evidence

43 selected coding-agent test files produced **465 passing tests**, zero failed or skipped, in the isolated reference copy. The separate Godot adapter suite added **26 passing tests** (491 total). Raw reports and selected paths live under `research/evidence`. The selected coverage includes compiler, capsules/source retrieval/revisions, episodes, memory/task state, artifact cursor/lease/catalog behavior, scheduler, llama runtime/tuning, session manager, recovery/compaction request boundaries, safety, tool orchestration, mutation queue, RPC codecs, jobs/subagents and native runtime-observation contracts. This is not full monorepo or live-model certification.

To reproduce the same selection after rebuilding the copy, run from `research/scratch/ag1-reference/packages/coding-agent` with isolated config:

```sh
AG1_CODING_AGENT_DIR=/home/alex/Projects/AG1/research/scratch/test-config \
  ../../node_modules/.bin/vitest run \
  $(cat /home/alex/Projects/AG1/research/evidence/ag1-selected-tests.txt) \
  --maxWorkers=2 --reporter=json \
  --outputFile=/home/alex/Projects/AG1/research/evidence/ag1-tests.json
```

The selected path file contains trusted generated test paths without spaces. For a different workspace, replace the absolute paths. Do not run root `npm run check` against Cruzible: its Biome command writes changes. Do not rebuild or install dependencies in the read-only reference repository.

## Oracle groups

| Behavior | Existing exact sources (prefixes in README) | Rust proof |
|---|---|---|
| Loop/stream/queues | `packages/agent/test/`, `T:suite/agent-session-queue.test.ts`, `T:agent-session-continue-replay.test.ts` | fake providers emit deterministic fragmented text/thinking/tools/errors; compare lifecycle/committed messages/settlement |
| Context admission/compaction | `T:agent-session-context-boundary.test.ts`, `agent-session-auto-compaction-queue.test.ts`, `agent-session-compaction-coordinator.test.ts`, `context-compiler.test.ts` | pause append/tool/summary, force epoch change, assert next provider request uses updated state and complete pairs |
| Retrieval/quality | `T:context-replay-eval.test.ts`, `context-replay-trajectory.test.ts`, `source-context-tool.test.ts`, `source-retrieval-signals.test.ts` | same source/branch/Git/access/query/budget inputs; compare file/range selection, order, required/forbidden files, status and bounds |
| Source/epistemic state | `T:source-revision.test.ts`, `task-state.test.ts`, `task-state-executive.test.ts`, `inspection-evidence.test.ts`, `runtime-observation.test.ts` | exact bytes+hash snapshot, source changes, claim kinds/confidence/proof, user instruction continuity |
| Artifacts/CV switches | `T:artifact-store.test.ts`, `artifact-tool.test.ts`, `tool-result-artifacts.test.ts`, `cv-architecture.test.ts` | ID/content separation, UTF-8/CRLF pages, tamper/cross-store cursors, expiry/budget/cap, catalog fault recovery, binary retention |
| Memory/episodes | `T:memory-store.test.ts`, `memory-context.test.ts`, `episode-memory.test.ts`, `episode-retrieval.test.ts`, `episode-candidate.test.ts` | migrate formats, deterministic injection/order/labels, archival, exact trajectory and non-authoritative draft |
| Journal/import | `T:session-manager/migration.test.ts`, `lock.test.ts`, `save-entry.test.ts`, `build-context.test.ts`, `tree-traversal.test.ts` | parent IDs, async/concurrent append, unknown records, corruption quarantine, idempotent import/rollback |
| Recovery | `T:agent-session-retry.test.ts`, `agent-session-recovery-arbiter.test.ts`, `agent-session-recovery-replay-scheduler.test.ts`, `agent-session-dispose-recovery.test.ts` | one replay owner, generation/session epoch rejection, ignored abort, no duplicate external effect |
| Scheduler/models | `T:inference-scheduler.test.ts`, `llamacpp-runtime.test.ts`, `llamacpp-vram-tuning.test.ts`, capability/thinking/vision tests | permits across terminal result, fake process/health/identity/exit, model switch/config, actual provider canaries later |
| Tools/policy | `T:tools.test.ts`, `file-mutation-queue.test.ts`, `safety-policy.test.ts`, `tool-orchestrator.test.ts`, `bash-close-hang-windows.test.ts`, job/subagent tests | permissions below every entry point, CAS/atomic writes, signals/PTY/output, no recursive read-only subagents |
| UI independence | `T:rpc-jsonl.test.ts`, `rpc-validation.test.ts`, `turn-addenda.test.ts`, `print-mode.test.ts`, interactive action tests | same domain traces via fake protocol/CLI/React; renderer pause/detach/crash cannot own decisions |
| Native embedding | `packages/godot-agent-runtime/test/`, native host protocol/staging/tool contracts | schema/nonce/version mismatch, transaction lifecycle, scoped tools, parent death and vision evidence |

Use normalized transcript fixtures with deterministic clock, seeded IDs, explicit file bytes and fake process/provider interfaces. Normalize timestamps/random IDs only where semantically irrelevant. Do not normalize away tool order, branch ancestry, freshness, cancellation reason, usage type or missing evidence.

## Required fixture packages

- **Provider turn:** user prompt → reasoning/text → multiple tools → result → continued answer; error before first event, mid-argument interruption, auth failure, retry-after, terminal-without-body, ignored cancellation, model change.
- **Long context:** compiler over soft target, protected recent evidence, metadata pressure, cursor/lease rehydration and fallback; run repeatedly after reopening the journal.
- **Source evolution:** exact-symbol, vague follow-up, changed-line recovery; edit outside the tool path, branch/worktree switch, deleted/replaced/symlink file; old references never regain current authority incorrectly.
- **Task evidence:** objective/latest instruction, plan review, passing check followed by mutation, runtime failure after successful build, incomplete native visual criteria, contradictory old episode.
- **Crash:** admitted tool without outcome, journal commit before UI event, blob before DB reference, interrupted importer, corrupted memory and damaged artifact index, stale lease key and app restart.
- **Transport:** snapshot/subscription race, out-of-order stale replies, duplicate command/approval, worker restart with same session, queue overflow, 30-second frozen renderer, large binary page streaming.

## Confirmed defect regression tests

| Defect | Fixture | Passing target behavior |
|---|---|---|
| B01 | Two 300-character blocks with 400-character aggregate projection budget, plus mixed Unicode/image/large-block variants | Combined projection is bounded using documented framing rules; full original evidence recoverable; event/request/persisted representations deliberately consistent |
| B02 | Truncated memory v3, malformed version, partial temp/replace stages | Explicit corrupt status; original retained; update cannot silently overwrite with defaults; recovery/import remains possible |
| B03 | Inject a source edit after returning text but before old implementation hashes; same-size/rename variants | Displayed bytes and canonical digest from one validated snapshot, or retry/unavailable; never old text labeled current |
| B04 | Fork/import a branch containing text/binary/capsule/episode artifacts and cursors, reopen child and exercise parent retention | Child evidence remains authorized and recoverable; no dangling references, no cross-session authorization bypass |

D01 PID/port ownership and D03 specialized mutation durability also need fake-process/fault tests before those implementations are adopted. These corrections are intentionally allowed to differ from old behavior; the defect record and invariant are the oracle.

## Property tests, fuzzing and integration tests

Use property tests where broad input combinations matter: journal branch reconstruction/migrations; cursor encode/decode/tamper; Unicode byte windows; tool pair preservation under arbitrary reduction; cancellation/permit ownership; bounded queue admission and snapshot watermark; file edits with repeated matches/CRLF/BOM. Use model-based transition tests for session/recovery/model/project lifecycle state machines.

Fuzz external parsers: JSONL/IPC envelope limits, cursor decoding, provider fragmented tool JSON/XML fallback, source paths, artifact catalog and imported persistence. Apply strict input size limits. A fuzzer panic is a bug; production parsing returns typed errors. Keep the corpus tied to actual contracts, not speculative formats.

Integration tests run fake HTTP/SSE model servers and fake child processes before live providers. On supported OS runners, spawn real controlled process trees, PTYs and tiny compiler/dev-server fixtures. Test path roots, cancellation/reaping, stdout/stderr exit races, file locks, atomic replacement and Git dirty/conflict behavior. Never use real user model processes or repositories as kill/rollback fixtures.

## Live model and long-horizon gates

Use existing `T:fixtures/context-intelligence/provenance-parity.json`, `fixtures/repository-context/externalized-contract.json`, `long-horizon-safety.json`, `fixtures/episode-candidates/completed-phase-fidelity.json`, `fixtures/context-replay/long-session.json` and RelayForge contract/source fixtures. Preserve fixture provenance and hashes.

`scripts/long-horizon-context-benchmark.ts` seeds roughly 2–3M raw-token historical artifacts, requires sequential distinct repairs, reopens sessions and changes source between checkpoints. Run the same chosen model sequentially for TS/Rust, record fresh-prefix/cache conditions, settings, tool artifacts, diffs and external verifier outcomes. Context-intelligence A/B calls and episode drafting are tool-free isolated same-model calls. Task success, claim recall, evidence availability, contradiction avoidance, required recall, forbidden avoidance and stale-authoritative claims are separate gates. Require zero stale-authoritative claims and no quality regression; do not average away catastrophic forbidden retrieval.

Add 8h/24h repeated sessions with observed memory/disk/process metrics, injected model crashes, window detachment and source changes. No live benchmark result is claimed by this research. Each migration phase should first pass bounded deterministic fixtures; long live gates occur at coherent authority changes and final acceptance, not after every cosmetic edit.

## Frontend/core tests

Reuse Dyad tests as interaction specifications where licensing permits: FileEditor/FileTree/CodeView/FileDiffEditor; chat input/tabs/list; PreviewPanel/toolbar/overlay; native theme bootstrap ordering; version preview and app-run lifecycle. Build a fake AG1 protocol host for component tests. Require loading, empty, pending save, conflict, denied approval, retry, persistence failure and reconnect states.

Capture approved visual baselines from pinned Dyad at matched viewport/theme/scale before extraction is accepted. Compare intentional AG1 adaptations, then test real Tauri integration on Windows/macOS/Linux. Test packaged fonts/Monaco workers, keyboard shortcuts, window chrome, clipboard/dialog permissions, URL navigation, preview isolation and native process launch. Electron Playwright E2E cannot be called a Tauri driver; choose supported platform drivers/manual coverage where automation is unavailable and record that gap.

## Per-phase completion template

Every phase records: parity IDs and invariants; preconditions and dependency artifacts; exact commands and fixture/environment setup; expected assertions; new failure/defect tests; performance budget and measurement scope; license/dependency changes; output artifacts; residual gaps and rollback. Passing unit tests alone cannot retire an H-risk TS authority. Avoid rerunning unrelated enormous suites for simple documentation or visual-only changes; select appropriate tests, then broaden at integration/cutover gates.
