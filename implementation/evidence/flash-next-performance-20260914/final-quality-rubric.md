# Full-answer quality review criteria

Prepared before the final suite. Evaluate the frozen prompts in `prompts.json`, not later versions of repository files. Report each answer's strengths, concrete errors, omissions, formatting compliance and truncation separately. Do not turn token-distribution similarity or a summed checklist into an intelligence score. Byte-identical answers can share a failure.

## Code review

The answer should cover cancellation, admission, ownership, readiness and progress, cite visible files/symbols accurately, and provide at least 500 words. Useful factual anchors in the supplied code:

- `ModelIdentity::matches` gives an explicit weight path precedence over alias fallback; context must be at least requested and less than 1,024 tokens larger, and a configured projector requires vision support.
- External healthy observations can set readiness but do not install an owned process handle. Owned stop operations rely on the host's authenticated handle contract, not a historical PID or occupied port.
- Starting an owned runtime checks required materials, plan fingerprint, material verification and vacant endpoint. `poll_ready` checks owned liveness; an individual health timeout remains Loading while the composition host controls the overall deadline.
- `RuntimeHandoff::restore` preserves the handoff on failure. It invokes `poll_ready` but discards the returned boolean, so restoration success is not itself proof that readiness was reached.
- `load_local_model` validates local selection, endpoint and conflict state; journal admission uses scope/command identity and expected revision. Existing receipts replay without starting another load; an unsettled receipt is durability-unknown.
- The spawned accepted operation survives a renderer/request disconnect. The load shares scheduler admission by endpoint with generation, waits up to 900 seconds, and checks quarantine. It sends no inference. Failed load releases its permit after successful cancellation; otherwise uncertain custody remains quarantined on drop.
- The visible tests cover message-free replay, shared admission across chats, and retry after verified stop. They do not prove all production adapter/process/filesystem behavior.
- `ModelLoadControl` handles pending/stale progress using model and start time, maps phase labels, bounds finite percentages, displays elapsed time and readiness, and disables conflicting load actions. It displays supplied progress; the component does not measure actual model-load percentage itself.

A strong answer need not enumerate every anchor. Material contradictions, invented implementation details in absent dependencies, or treating observations as authority are substantive errors. Accurate limitations and precise coverage matter more than superficial length.

## Distributed-service prose

Check the requested 500-word length (record actual count and difference), clarity for a new colleague, and two concrete examples. Core reasoning should preserve an overall deadline through retries and backoff, bound attempts, distinguish cancellation signaling from rollback or proven termination, and handle ambiguous outcomes with appropriately scoped idempotency keys and durable deduplication where needed. Avoid unconditional exactly-once, safe-retry or instantaneous-cancellation claims.

## Crash-recovery design

Require at least 500 words and a concrete state machine, not just generic advice. Review crash boundaries around acceptance, durable intent, temporary-file preparation, atomic replacement, completion record and UI success. A sound protocol identifies operations and old/new content, persists sufficient recovery material in the right order, distinguishes atomic rename from durability, serializes or detects conflicting edits, and reconciles file state with incomplete journal state on replay. It must address cancellation before versus after the irreversible replacement point, repeated recovery, and ambiguous or externally modified states without blindly overwriting user changes. Do not demand a specific state naming scheme or platform API beyond the prompt's assumptions.

## Mechanical and integration checks

- Retrieval: strict JSON parsing and exact expected fields; separately record any fence-stripped match.
- Text editing/copying: compare exact bytes including final newline, then report content-only agreement separately.
- Tool-shaped JSON: retain the frozen expected-object check and its known schema ambiguity; native function-call smoke tests are distinct evidence.
- Actual AG1: independently inspect written file bytes, unchanged checker, actual tool execution and verification output, web tool results and citation, cancellation recovery, backend reuse, actual binary/argv/library mappings, and clean shutdown.
- Report truncated outputs explicitly; discovery screens capped at 256 tokens cannot establish full-answer instruction adherence.
