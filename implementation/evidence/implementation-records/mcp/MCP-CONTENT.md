# MCP resource and prompt checkpoint — 2026-09-12

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**B3 adds explicit resource reads, simple resource templates, and prompt retrieval through the shared executor.** It continues [B2 tool execution](MCP-EXECUTION.md). The [full MCP prompt](../../../../tasks/02-MCP-INTEGRATION.md) remains the completion boundary. The user has now requested unattended continuation through full integration; this intermediate source checkpoint is not desktop acceptance or an installed feature.

## Implemented boundary

A native selection identifies a tool, resource URI, resource template, or prompt by capability kind, originating server, exact catalog key, full metadata digest, scope, configuration, connection, authentication identity and protocol revision. Resource and prompt identities use separate wire namespaces. Existing tool identity serialization and hashes remain compatible. Peer descriptions and prompt roles cannot supply native instructions, verification or grants.

`select_capabilities` atomically installs at most 16 selected definitions within the existing 64 KiB schema budget. Existing tool-only selection remains supported. `search_capabilities` searches cached connected metadata in the exact enabled scope, returning at most 20 entries and 32 KiB, with at most 256 query bytes. Searching does not start a process, refresh a connection, read content, expand a template or fetch any URI. Unsupported templates remain visible with `available: false`; richer user-facing diagnostics belong to the manager UI checkpoint.

Resource reads accept an empty argument object and always use the selected URI. Prompt arguments are explicitly named strings with required/optional declarations, no scalar coercion or unknown keys. At most 64 prompt arguments are supported, each at most 4,096 characters under the existing schema validator. URI templates support only simple single-variable `{name}` expressions, with at most 32 distinct variables; all variables must be supplied as strings. UTF-8 bytes outside the RFC unreserved set are percent encoded. Operators, modifiers, lists/maps, malformed expressions and unbounded expansion are unavailable. Template values and both original/expanded URIs are capped at 4,096 bytes. This is a deliberately limited RFC 6570 subset, not general template conformance. Approval displays include the exact expanded URI.

Execution uses the same native selection witness, explicit approval, acknowledged durable intent, connection queue, freshness check and send gate as tools. Before sending, the worker refreshes the relevant catalog and checks the complete selected metadata identity again. The only new protocol methods are native-chosen `resources/read` and `prompts/get`; callers cannot override the method/server/URI/prompt name or submit continuation state. There is no local-file or web fallback for a resource URI. A resource-only or prompt-only server does not need to advertise tools.

## Content and custody

Complete correlated replies enter the existing scoped artifact path before projection. Resource results admit up to 256 text/binary entries, including bundles containing different URIs. Prompt results admit up to 128 messages with protocol roles `user` or `assistant`; those labels are rendered as quoted source data in ordinary tool-result text. They never become AG1 conversation roles or system instructions. Text, embedded resources and binary/media encodings use the shared inert projection. Images/audio are still retained without decoding or playback.

Malformed content/URI/base64/roles and protocol errors remain errors with original custody. Current `input_required` state and forms are discarded before artifact custody, with explicit redacted/incomplete retention and no automatic continuation. Successful delivery still has unknown effects. Failed durable intent or revocation before send has no admitted external effect; terminal receipt replay never resends. Existing operation/traffic/depth/node/time/queue/registry budgets are unchanged, and the 272 KiB execution-envelope limit now covers the three admitted methods.

## Verification

The [content evidence](../../MCP/content-operations.json) records final commands, source and binary hashes, logs and preservation checks. The selected suite has **205 passing tests**, including eight new transport/execution contracts and five new core capability contracts. Coverage includes both revisions and transports, HTTP JSON/SSE, template encoding and preview, strict routing, scope filtering, metadata changes after approval, revocation during refresh, result retention and replay, malformed content, unsupported continuation and failed intent acknowledgement. Existing native GitHub, process, storage and MCP contracts are included.

The initial execution run used Cargo's default parallel test setting and exceeded the deliberately shared four-connection limit (4 passed, 17 failed). The established serial test setting resolves the fixture contention; the original failed run remains retained. The first full candidate run also encountered `Resource temporarily unavailable (os error 11)` in the unchanged Git worktree regression; the exact candidate passed on retry, without changed expectations. Its original log remains retained and the root cause is not claimed. Clippy succeeds with existing warnings and no new diagnostics in the changed capability/execution modules. The frozen core comparison still has 22 passes and the same six assertion failures: only thread IDs and the once-per-process backtrace-help line location are normalized. Expectations are untouched.

No dependency or SDK runtime was added. Protocol shapes were checked against the official [current schema](https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/2026-07-28/schema.ts), [current resources](https://modelcontextprotocol.io/specification/2026-07-28/server/resources), [legacy resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources), [legacy prompts](https://modelcontextprotocol.io/specification/2025-11-25/server/prompts) and [RFC 6570](https://www.rfc-editor.org/rfc/rfc6570.html). Previously accepted binaries, non-MCP core logic, storage, native application dispatcher, dependencies, model configuration and installed distribution are preserved.

## Remaining integration

| Phase | Still required |
| --- | --- |
| A | SDK adoption/conformance decision, broader supported schemas/templates, input continuation, liveness/notification coverage and complete aggregate budgets |
| B | Canonical account/project/session composition, current runtime cancellation/policy fences, bounded discovery and selected schemas in outgoing model requests, context/CV and shared result/media DTOs |
| C | OAuth/private configuration and separate vault custody, scoped risk/grant rules, credential-generation fences, local execution policy/containment and startup reconciliation |
| D | Installed/detail/manual forms, connection and scope controls, inspection, auth/permissions/logs and actual native wide/narrow workflows |
| E | Revalidated existing registry source, bounded discovery/cache/provenance and reviewed setup/activation with manual fallback |
| F | Full adversarial/cross-feature acceptance and exact production package/build/startup/install verification |

A/B remain partial and C–F still have their main deliverables. The next slice composes the manager with canonical session scope, model request preparation and native runtime execution. Production installation remains due at a usable major application checkpoint. Other feature tasks, real-model testing and the separate duration programme remain paused.
