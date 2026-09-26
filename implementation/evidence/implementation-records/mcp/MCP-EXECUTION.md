# MCP tool execution checkpoint — 2026-09-12

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**B2 adds actual `tools/call` execution to the native service, through AG1's existing executor.** Owned stdio and Streamable HTTP fixtures exercise both supported revisions, including HTTP JSON and SSE replies. This continues [B1 registry integration](MCP-REGISTRY.md). A/B remain partial, and the [full MCP prompt](../../../../tasks/02-MCP-INTEGRATION.md) remains the completion boundary.

This is a source/service checkpoint, not an installed desktop feature. There is no model/renderer dispatch route or manual UI call button yet. No external MCP service, credentials, real inference, installed runtime or model configuration was used or changed. Production packaging/install remains due at the first usable application checkpoint.

## Native execution path

`Manager::execute_tool` consumes an ordinary `AuthorizedCall`, a native selection witness and the existing execution/artifact ports. It must run on a native execution worker, outside the UI/session owner thread. The existing executor validates approval and acknowledges durable intent before submitting anything to the connection worker. Failed intent acknowledgement sends no tool request. There is no public arbitrary-method/JSON tunnel.

The connection worker refreshes `tools/list` before every call and compares the selected tool's complete metadata digest and bound server/configuration/auth-profile/connection/protocol/scope identity. An incompatible or changed schema, expired approval, required unsupported task execution, or retired connection fails before tool send. Refresh does not automatically adopt changed metadata or transfer approval.

Immediately before stdio send admission, or after HTTP DNS/endpoint validation and before request submission, a native gate rechecks the ticket and connection. Disable/edit/disconnect use the same gate. An operation admitted before revocation is in flight and may have effects; cancellation does not claim to roll it back. No lock is held across network/process waits. Approval expiry includes monotonic time spent on the worker.

Only one execution may occupy a connection at a time. A second call receives `QueueFull` before durable intent. The connection mailbox is bounded to one command; the existing process-wide four-connection ceiling also bounds concurrent MCP calls. The ordinary registry snapshot copied to a worker is capped at 4 MiB of encoded definitions, at most 16 MiB across these four permits, excluding allocator overhead. Existing selected-schema, catalog, transport and storage ceilings still apply; this is not an RSS/cgroup/disk quota claim.

## Results, failure and recovery

- Complete correlated JSON-RPC responses, including protocol errors and schema-invalid results, are retained through the existing branch/session artifact port before projection. The producer is the ordinary tool producer with native MCP identity; peer provenance and verification fields cannot supply artifact grants or native receipts.
- Text and structured data enter the ordinary aggregate result projection. Current structured scalar/array/object/null values retain their shape; legacy structured results require objects. Declared output schemas are checked. Application `isError` remains an error and is distinct from transport/protocol failure.
- Resource links remain literal references and are never fetched. Embedded text stays inert. Image, audio and binary-resource blocks are retained in the original response with explicit unsupported display/playback summaries; base64, media MIME and resource URI shapes are checked, but this checkpoint does not decode image/audio formats or claim image/pixel acceptance.
- Unsupported current input continuations receive a terminal error without sending input, credentials or another tool call. Opaque state and input forms are discarded **before artifact storage**; a redacted response records that complete retention was not performed. Legacy unsupported client requests receive the existing explicit protocol refusal.
- A completed exchange still has `Effect::Unknown`: a server success assertion does not prove what external effects occurred. Failures before tool-send admission have `Effect::None`; timeout, cancellation, malformed/truncated/oversized/uncorrelated replies after admission retain uncertainty. Uncorrelated or undecodable bytes are not promoted to a tool result or stored as a purported complete response.
- The shared executor persists terminal outcome, effect certainty and provenance before its end event. Receipt replay cannot send a second request. If original/result storage fails after sending, the durable intent remains uncertain and cannot automatically execute again. Historical recovery uses the existing scoped storage path; a disconnected connection does not regain live admission from a receipt.

Tool request arguments remain capped at 256 KiB; the encoded request envelope is capped at 272 KiB. Discovery/control requests retain their 4 KiB ceiling. Responses retain the existing 1 MiB frame/JSON ceiling, depth 32, 20,000 nodes, 8 MiB operation traffic and 128 messages; result projection admits at most 256 content blocks. Each refresh and call uses the existing configured deadline (default 15 seconds, maximum 30); the service reply wait is bounded to twice that deadline plus one second. HTTP still disables retries, redirects, proxy/cookie credentials and session resurrection.

## Verification

[Execution evidence](../../MCP/tool-execution.json) records exact commands, source/binary hashes and retained diagnostic runs. The final selected suite has **192 passing tests**, including 13 new operation contracts covering both transports/revisions, SSE, original custody, structured shapes, policy/schema freshness, cancellation, queue bounds and storage acknowledgement failures. Existing MCP, core registry, storage, process and native GitHub regressions are included. Formatting passes; Clippy succeeds with existing repository warnings and no diagnostics in the new operation/execution modules.

The first run caught an invalid new artifact producer category; the adapter now uses the existing tool producer and carries MCP as provenance. A later run observed one connection-start failure that did not reproduce in the diagnostic run; its original log is preserved without claiming a proven root cause. The owned HTTP fixture now publishes its port file atomically and captures startup diagnostics. No failed-run evidence or frozen expectations were removed. A1–B1 accepted binaries, core/storage source, the native application dispatcher and Cargo lock remain unchanged.

The revision-specific request/result checks were revalidated against the official [legacy tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) and [current tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools) specifications. No SDK runtime or dependency was added; the prior SDK adoption/conformance decision remains open.

## Remaining work and next slice

| Phase | Remaining work |
| --- | --- |
| A | SDK adoption/conformance, resources/read and prompts/get, broader schemas, supported input continuation, idle notifications/liveness and complete aggregate budgets |
| B | Canonical account/project and runtime policy/cancellation composition, selected outgoing model requests, context/CV integration, native result DTOs and bounded media handling |
| C | MCP OAuth/private configuration and separated vault custody, reviewed risk/grant rules, credential-generation send fencing, local execution policy/containment and startup reconciliation |
| D | Native Installed/details/forms/inspection, auth/permissions/logs and actual Tauri wide/narrow workflows |
| E | Revalidated public registry source, discovery/cache/provenance, supported guided setup and reviewed activation/manual fallback |
| F | Complete adversarial/cross-feature acceptance and exact production build/startup/package/install verification |

The send gate currently covers unauthenticated native manager connections and the caller-supplied ordinary execution fence. It is not evidence for a credential-backed or live application runtime fence. Idle subscriptions, task-level cancellation composition and final private-result delivery checks still need native runtime integration. The existing broader local host-account execution disclosure and A4 containment limitations remain in force.

Next, extend the same native admission/custody path to explicit resource reads and prompt retrieval, then compose it with canonical scope and model request preparation. Keep unsupported features and all production dispatch unavailable until their boundaries are implemented and verified. Other feature tasks, real-model tests and the separate duration programme remain paused.
