# Migration risks and mitigation

Risk rankings concern loss of AG1 behavior during migration, not speculative feature value. All mitigations must appear as executable roadmap gates. H = high impact; M = moderate impact. See parity IDs and invariant IDs for traceability.

| Risk | Impact / likelihood | Evidence | Mitigation and release gate |
|---|---|---|---|
| Porting the loop but omitting the harness | H / high | AgentSession request builder, coordinators, modes, task/CV/evidence and recovery are outside `packages/agent` | Map A01–A85; require fixture coverage for every active owner; full turn traces before retiring TS |
| Mid-loop context becomes stale | H / high | Loop snapshots versus compaction commits; event drains and epoch checks | Port request admission atomically with compaction/replay; pause tool results/append and force compaction in tests; I02–I05 |
| Retrieval freshness silently weakens | H / high | Separate snapshot/hash B03; source metadata/cache/Git/access interplay | Correct B03 first in reference oracle; raw-byte snapshot identity, canonical refs, external edits/rename/worktree cases; I06,I09 |
| Task/epistemic truth degraded to UI state | H / medium | taskState, runtime observations, checkpoints, episodes and print completion gate | Core-owned reducers/transactions, verify kind/evidence freshness, page unmount/crash tests; I01,I07,I08 |
| Persistence format loses branch/evidence/custom entries | H / high | JSONL tree, unknown extension records, artifact keys/cursors and memory versions | New-destination import with manifest/checksums; unknown payload retention; restartable migration and rollback/export; I13–I15 |
| Two authorities during staged migration | H / high | TS owns journal/loop/scheduler; new Rust services proposed | One writer per state family and explicit cutover checkpoint; shadow read-only comparison; no dual tool execution |
| Scheduler oversubscribes local model | H / high | Single process-global TS semaphore; per-worker pools would be independent | One active TS execution initially; host endpoint ownership before concurrency; never nested acquire the same generation permit; I16 |
| Cancellation replays external work | H / medium | retry/compaction callbacks, detached jobs, unresponsive streams | Operation ID and intent/outcome log, unknown outcome recovery, no automatic mutation replay; stale response/epoch tests; I04,I18 |
| Local model kill/restart targets wrong process | H / medium | raw PID/port eviction D01; detached launch state | Owned handle/start token/executable verification, fake PID-reuse test, explicit foreign-port conflict; no live destructive repro |
| Model behavior loses compatibility | H / high | multiple provider schemas/auth/tool formats/thinking/vision/fallbacks | Record fake streams/errors for each retained provider family; preserve usage/stop semantics; live canaries with fixed configuration |
| Long reasoning killed by new deadlines | H / medium | generic watchdog comments disagree with zero defaults; loop idle 120s, TTFT 0 | Explicit separate TTFT/idle/reasoning limits; preserve default behavior until tests justify change; slow-prefill fixture |
| TUI extraction misses behavior | H / high | addenda/completion/plan/approval/workflow/diagnostics in mode code | Test API clients against mode fixtures; move decisions, keep render adapters separate; core has no terminal/React dependencies |
| JS extensions stop working | H / medium | Jiti-loaded tools/providers/hooks/resources with arbitrary extension payloads | Define retained API surface and compatibility worker; inventory custom tools/commands; do not claim full Rust-only runtime if TS agent still owns execution |
| Pro code enters portable UI via imports | H / high | PreviewIframe/Annotator, DyadSearchReplace/parser, type barrels | Approved copy manifest, transitive import/CSS/asset analysis, package/bundle denylist, notices; I30 |
| Visual fidelity degraded during backend changes | M / high | complex panes, virtualization, Monaco disposal, focus/native overlays | Pinned visual scenarios and interaction tests; preserve composition before adding AG1 drawer; no broad redesign |
| Preview automation cannot transfer to Tauri | H / high | Electron WebContentsView/CDP and ephemeral partitions | Early platform capability experiment with isolated preview; optional Playwright sidecar, native observation adapters; don't force project=URL |
| Cross-platform process/file behavior | H / high | Windows shell quoting/pipe hangs, Linux PID tokens/bwrap, platform webviews | Windows Job Objects/process groups, Unix reaping, explicit path semantics and atomic replacement tests; CI per OS |
| Performance claim hides extra processes | M / high | temporary worker + webviews/model/browser/dev servers | Include process tree/PSS; separate shell vs models; matched workloads; report bridge and final architecture independently |
| Context savings conceal task regression | H / high | replay evaluator separates quality, rehydration, forbidden and task metrics | Same model/trial order; external verifier, zero stale-authoritative claims; reject quality regressions regardless of tokens |
| Unbounded logs/artifact/memory growth | H / medium | global queues, memory archive, text records, full-log reads | bounded RAM and disk spool, explicit reference/retention policy, 8h/24h slope and evidence availability tests |
| Rust abstraction overhead or unsafe optimization | M / medium | many TS coordinators tempt one-crate-per-file port | Coherent five-package default, explicit ownership, benchmarks before arenas/lock-free work; review allocation/locking profiles |
| Crash safety assumed from rename/SQLite | H / medium | heterogeneous writes; database and blob fs are separate | write/sync/publish then transaction, orphan reconciliation, fault tests and documented acknowledgment durability; I14,I15 |
| Background job/model survives unexpectedly | H / medium | detached/unref jobs and server are intentionally persistent today | Distinguish owned task vs adopted resident service; window-close/quit policy and recovery reconciler; never orphan accidentally |
| Native app “verified” by compile success | H / high | Godot task contract and runtime observation types distinguish evidence | Separate compile/test/launch/playtest/visual proof tied to target/revision; Tauri frontend and native diagnostics both required |
| Old docs drive feature scope | M / high | removed auton, uncalled MCP/DAG runner, CV/episode defaults | Evidence-first status taxonomy; partial/legacy dispositions in roadmap, no speculative resurrection |

## Highest-risk authority handoffs

1. **Journal/artifact import:** readers must preserve references, tree branches, binary hashes and cursor keys; fail closed on corruption, never alter the original. Do not import live process handles as if they remained valid.
2. **Compiler/recovery cutover:** shadow identical request snapshots and compare messages/provenance, then switch at quiescence. Keep the previous implementation available until long-session gates pass.
3. **Loop/provider/tool cutover:** fake-provider traces first, then bounded real tasks. A partially migrated loop must not use two different policy or scheduling authorities.
4. **Window lifecycle change:** prove close/reopen, crash/reconnect and explicit quit separately. A library independent of React does not automatically mean tasks persist after process death.

Open questions are scoped experiments, not excuses to make the next model repeat discovery: exact preview capabilities on each OS; release-grade shell/GPU baseline; extension ecosystem compatibility scope; SQLite durability settings and import/export format; per-endpoint fairness policy after parity; optional adjacent mom/pods/web-ui client support. Default recommendations and experiments are specified in TARGET_ARCHITECTURE and PERFORMANCE_BASELINE.
