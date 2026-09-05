# P10 — Session loop, recovery and subagents

## Objective and reading

Integrate the complete Rust agent harness on isolated workspaces. Read common instructions, AG1 architecture end-to-end trace, Agent/AgentSession coordinators, mode addenda/completion, retry/compaction/replay, controlled policy and subagent/checkpoint tests. Scope A02–A14,A17,A26,A48,A70 plus integration of all context/tools/providers; I01–I05,I07,I08,I13,I15–I19,I22,I25; L03–L06.

## Entry and allowed work

Requires P08 full context pipeline and P09 tools/policy/resources, transitively storage/platform/providers/protocol. Actual APIs and passing fixtures must exist. Implement core session/recovery/model/task actors and adapter wiring in isolated destinations. TS remains authority for the P04 desktop; no live Rust shadow mutations/inference against its workspace/endpoint.

## Work and authority

Implement session create/resume/fork/replace, current model/tools/thinking, steering/follow-up queues and one mutation owner. Runtime replacement prepares first, drains old work, changes epoch, atomically selects new owner and disposes failed preparation. No process.chdir. Distinguish generation terminal, loop terminal, queued recovery and fully settled session.

Before every provider request, wait for accepted event/persistence work and compaction, read current branch/messages/task/tools/model, compile and enforce hard admission margin. Rebuild on epoch/leaf/content/tool/model changes including within the same tool loop. A failed append blocks another request. Preserve public progress versus durable acknowledgment distinction.

Implement source-ordered preflight, concurrent eligible execution and source-ordered final outcomes, per-file mutation serialization, native/XML fallback without duplicate execution, repetition/failure guards and mode-selected controlled policy. Every executed/rejected/aborted call has one final outcome. Integrate aggregate B01 correction and durable full evidence.

Integrate manual/automatic summarization, trajectory externalization, pending suffix merge, emergency fallback, retry backoff/classification, recovery arbitration and stale replay fencing. Cancellation drains tools, model stream, direct shell, pending appends and recovery; uncertain external effects are inspected rather than automatically replayed. No arbitrary new long-reasoning deadline. Preserve mode addenda, plan/question/approval and coding completion evidence below the frontend.

Implement explore/research/implement/verify/plan subagent profiles with isolated transcripts, no recursive task tool, parent policy, bounded turns/time/tokens, partial evidence/usage and checkpoint acknowledgment. Preserve the reference asymmetry: subagents use their documented lighter hygiene, not an unrequested full-parent CV redesign. All generations, including candidate/summary/subagent calls, share one endpoint scheduler.

## Interfaces, tests and commands

Deliver `SessionRuntime`/registry facade implementing P02 commands and snapshots, reusable by desktop, CLI and Godot adapters. Add callback/extension client modes without TUI types. Store admitted intent/outcome and branch/evidence transactions through one storage owner. No Dyad code.

Create `runtime_contract` with full normalized fake-provider traces. Pause tool completion/append, trigger compaction, change model/tools/session, release pauses and verify the next request uses current state. Test repeated end, pending result, listener exception, retry versus compaction race, ignored cancellation, follow-up delivery, crash after mutation before outcome, memory/task continuity, forked cursor access and readonly/delegation/checkpoint failures. Exercise plan/print/RPC mode behavior rather than assuming SDK is equivalent.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-core --test runtime_contract
python3 tools/reference_oracles.py --suite runtime-differential --offline
python3 tools/phase_gate.py P10
```

## Measurement and completion

Compare end-to-end fake workload tool/context/queue latency and bounded heap across repeated resume/compaction cycles. All H-risk runtime rows need passing and failing traces. Exit requires headless real Rust execution in disposable workspaces, complete recovery/settlement semantics, B01–B04 integration, no hidden TS AgentSession and unchanged llama launch-plan output. Live per-model acceptance remains separately tracked; never compare two concurrent runs on the single local endpoint.

## Rollback and handoff

No production authority transfer yet. Stop owned test resources and retain uncertain outcome records. P11 gets a complete Rust runtime for target integration; P12 can bind UI/CLI to it in isolated mode. The TS product remains usable until P13.
