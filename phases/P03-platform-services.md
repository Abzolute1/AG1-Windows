# P03 — Explicit workspace and owned platform resources

## Objective and reading

Deliver filesystem/Git/process/PTY/network/secret ports without agent execution. Read common instructions, DECISIONS D01/D04, AG1 tools/path/mutation/bash/job/check source, D01/D03/D05/D12 and llama ownership L06. Scope A37,A61–A68 platform foundations; I18,I19,I21,I24; L06,L07.

## Entry and allowed work

Require P02 protocol/core ports and P01 fixtures. Implement `product/crates/ag1-platform` and explicit test doubles. Work only on disposable fixture workspaces/process trees. Do not load or stop user models, adopt existing PIDs, change global cwd or use Cruzible as a Git restore fixture.

## Work and authority

Implement canonical workspace/worktree IDs, contained path resolution, validated byte snapshots, bounded reads/search/indexing and Git status/diff/HEAD expectations. Filesystem events are hints; watcher overflow triggers rebuild. Include worktrees, symlinks, ignored/untracked paths and external edits. Implement shared atomic/CAS mutation primitive with mode/BOM/CRLF preservation and clear expected-revision conflicts. Full tool-specific edit semantics come in P09.

Implement a process owner with explicit argv/cwd/env, resource-group handle, start identity, bounded output spool, exit/timeout/cancel and descendant reaping. Unix groups/start tokens and Windows process-tree ownership require real platform fixtures; no PID/port ownership inference. PTY has byte input/output, resize, exit and disconnect semantics. Separate observed external services from owned children; they cannot be killed by Stop or Quit. Reject an occupied port as conflict. Never call the old llama port eviction helper.

Implement filesystem/Git resource claims without holding them across inference/UI approval. Provide network/secret ports with fake implementations for tests and host-only credential resolution; all request roots and policies explicit. Build minimal compiler diagnostic and target-discovery foundations without claiming P11 runtime adapters are complete. Preserve source index/map/manifest bounds for A37.

## Interfaces and state

Deliver `WorkspaceService`, `MutationPort`, `ProcessSupervisor`, `PtyPort`, `SecretPort` implementations of core contracts, or equivalent documented names. Platform handles are ephemeral; persistent records carry identity observations and must reconcile after restart. Do not introduce a second journal. P04 owns its worker via these services; TS remains the only agent scheduler until cutover.

No Dyad frontend code is needed. Audit retained OS/native dependencies, especially PTY/process and bundled helper binaries.

## Tests and commands

Create ag1-platform `platform_contract`. Spawn only controlled fixture children. Cover descendant holding stdout after parent exit, Windows pipe-close hang, termination escalation, repeated cancel, wrong executable/start token, foreign port listener, permission denial, process disappearing during stop, symlink swap, same-size external edit, stale CAS, rename and dirty/conflicted Git restore refusal. A failed filesystem write cannot report success.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-platform --test platform_contract
python3 tools/phase_gate.py P03
```

Cross-platform fixtures absent from this machine remain pending in evidence and are mandatory before that platform's release. No destructive live process repro is permitted.

## Measurement and completion

Compare small/large fixture status/diff/search/read/write latency and blocking time against P01; measure bounded log memory and tail cost. Require zero remaining owned fixture children after stop and no effect on foreign processes. Document per-resource cancellation deadlines and unsupported OS capabilities. Exit needs tested real platform services on the development platform, fake cross-platform state-machine cases, explicit pending OS runs and a process ownership API sufficient for P04/P06.

## Rollback and handoff

Remove only disposable fixture trees/processes via recorded ownership. No user state moves. P04 can supervise a worker; P05/P07 can rely on byte/identity/CAS ports; P06 can use safe process ownership without reimplementing the backend engine.
