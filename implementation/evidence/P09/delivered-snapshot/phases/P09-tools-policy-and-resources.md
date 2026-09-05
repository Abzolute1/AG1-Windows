# P09 — Tools, approvals, resources and extension execution

## Objective and reading

Deliver tool semantics and policy independently of the full agent loop. Read common instructions, tool registry/wrappers, safety/planning/headless policy, mutation/bash/job/web/check/resource/extension sources, B01 and D03. Scope A16,A59–A66,A68,A71; foundations for A12,A26,A67,A69,A70,A72; I04,I17,I19–I21,I24; L03–L07.

## Entry and allowed work

Require P06 provider/scheduler/auth, P07 evidence/task APIs, P03 platform services, P05 storage and P02 approvals. Implement core tools/policy/resources, platform adapters and optional `product/compat/extensions`; use fixture workspaces only. No original files/real user services/credentials are test targets.

## Work and authority

Port schema registration and prepare→validate→policy→execute→post-hook stages, preserving dynamic active-tool selection and capability gates. Separate TUI renderers from definitions. Unknown/malformed/truncated calls fail safely. Before/after hooks cannot bypass core policy or write parent state independently.

Implement read/find/grep/list and all selected edit/write/append/diff/preview/apply variants with reference matching, line endings/BOM/mode/truncation behavior. Use P03 atomic/CAS primitive for every mutation variant; retain per-file queue and workspace/Git claims. Shell external edits remain possible and freshness catches them. Stale previews conflict rather than overwrite.

Implement shell timeout/exit/cancel/output, background jobs/log tails/finished notifications, workspace isolation and configured sandbox behavior. Preserve full outputs as artifacts with bounded live projections. Web search/fetch retain origin/provenance, URL/credential/SSRF rules and bounded results. Network evidence remains untrusted. Check/visual tool interfaces are capability-dispatched; P11 supplies native/browser target implementations.

Correct B01 with an aggregate content budget across result blocks, explicit projection framing and full original evidence persistence. Events/request/persisted representations must have named lifecycle semantics and stable IDs; neither repeated end events nor cancellation duplicates results. Exempt explicit artifact page reads from recursive externalization.

Port skills/templates/package discovery, precedence, trust, collisions, reload and prompt resources. Deliver optional JS host for the P01 inventoried tool/provider/hook/command/UI-request surface. Core owns parent scheduling/policy/state; extension model calls go through its broker. Timeouts/host death settle operations. Preserve unknown custom records and return explicit unsupported API errors; no entire AgentSession hidden in the compatibility host. Keep trusted-extension process privileges documented without claiming an OS sandbox it does not have.

## Interfaces, tests and commands

Deliver a tool executor callable by a deterministic test driver and later P10. Approvals are core-owned pending operations with epoch/argument/revision binding. Extension host RPC is versioned and bounded; plugin-owned state is distinct from parent journal. Audit Node/helper/web dependencies and notices; no Dyad backend reuse.

Create `tool_contract`: malformed schema, rejected call outcome, cross-workspace paths, two competing writes, stale editApply, interrupted write, CRLF/BOM, one/multiple small result blocks including Unicode/images, shell child pipes, log paging, sandbox unavailable, fetched redirects/SSRF, extension crash/reload/cancel and stale/duplicate approvals. Read-only subagent toolsets must exclude mutation/delegation even before P10 supplies subagents.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-core --test tool_contract
python3 tools/phase_gate.py P09
```

## Measurement and completion

Measure no-op dispatch separately from I/O, parallel reads versus serialized mutations, large output spool and extension roundtrip. Require bounded memory and no side effect before approval. Exit needs every selected tool child case, B01/D03 regressions, resource precedence and explicit extension API compatibility table; unimplemented provider hooks cannot be hidden by dropping their fixture.

## Rollback and handoff

Stop owned fixture jobs/extension hosts and retain uncertain outcomes. TS desktop remains unchanged. P10 receives real executable tools/policy/resources and a brokered extension host; P11 fills declared target capabilities without replacing tool validation.
