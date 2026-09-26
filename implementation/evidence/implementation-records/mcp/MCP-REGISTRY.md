# MCP shared registry checkpoint — 2026-09-12

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**B1 is implemented at the source/service boundary. Full MCP integration remains unfinished.** This continues the [A4 manager checkpoint](MCP-MANAGER.md) with bounded tool selection, an MCP origin in the existing tool registry, conservative approval and durable execution provenance. A and B are partial; C–F still need their main deliverables. Most product integration work remains, so test counts are not a meaningful completion percentage.

This is an internal checkpoint, with no desktop/model dispatch or live MCP operation adapter. It does not create a usable production MCP feature and has not been installed. The installed application, model profiles and native GitHub adapter remain untouched. The [MCP task](../../../../tasks/02-MCP-INTEGRATION.md) remains the governing completion boundary.

## Implemented behavior

- The native manager searches only ready, enabled catalogs in the requested scope. Inactive connection-test catalogs cannot supply executable definitions. Search returns at most 20 schema-free summaries, bounded to 32 KiB, with a 256-byte query limit; it performs no new discovery or resource requests.
- Explicit selection atomically publishes at most 16 MCP definitions and 64 KiB of encoded definition metadata into the ordinary registry. Other tool origins and active selections are preserved. Repeating an identical selection does not churn the registry revision.
- Selected tools retain server, original name, scope, configuration revision, auth profile, credential generation, connection generation, protocol and metadata digest. Native selection witnesses also bind the manager owner and live connection. Disconnect, disable, configuration changes and stale identities invalidate preparation. Equal numeric registry revisions cannot substitute a different tool identity.
- Inspection and registry admission share the same conservative structural schema validator. Unsupported constraints stay inspectable but cannot be selected. MCP arguments use strict validation without native legacy argument conversion; wire preparation rejects duplicate keys and bounds bytes, depth and nodes.
- Executable definitions whitelist descriptive fields. Peer prompt guidance, source claims, annotations and trust fields cannot become native instructions or action authority. The original metadata remains available for inspection and contributes to identity.
- The existing Authorizer now binds MCP identity and scope to exact, expiring, single-use approval. Until native MCP risk rules exist, unknown actions require approval even in generic bypass/trusted-tool modes; planning and headless refuse them. This is a conservative foundation, not the final trust UI or permission model.
- The existing executor records MCP identity with durable intent and argument/original-result provenance. Recovery cannot replay an old operation under a changed server connection or metadata identity. An uncertain adapter outcome retains intent and cannot automatically repeat effects. MCP data does not become native verification evidence or acquire Undo semantics.

## Evidence and limits

See [registry integration evidence](../../MCP/registry-integration.json) for exact commands, hashes, preserved binaries and logs. All tests use owned fixtures and isolated storage; durable execution tests use mock adapters, not real MCP operations.

The 1,000-tool fixture proves that cached search adds no protocol requests and only two selected schemas enter the registry (under 4 KiB). This is registry-level evidence, **not** the final outgoing model-request/token-budget or context-virtualization acceptance test. Canonical account/project composition, live notification freshness and revalidation under the actual credential/send fence remain pending.

Final verification: 179 selected tests pass, including seven new core MCP tests, 19 manager tests, 49 other MCP transport/foundation tests, two MCP storage tests, three MCP units, 13 shared execution/storage tests, 43 storage regressions, 29 platform/process tests and 14 native GitHub tests. The separate frozen core comparison has 22 passes and six failures, reproduced with identical assertion bodies before and after this change; no expectations were rewritten. An early parallel manager test run exceeded its process-wide four-connection fixture quota; the required serial run passed without relaxing the quota. Failed logs are retained.

Formatting checks pass for the new modules and MCP files. Clippy succeeds with existing repository warnings; it is not a repository-wide warning-free claim. No dependency or SDK runtime was added. The A1–A4 evidence binaries and Cargo lock remain preserved.

## Remaining implementation

| Phase | Remaining work |
| --- | --- |
| A — Protocol | SDK adoption/conformance decision, broader schemas, tools/call, resources/read, prompts/get, supported input continuation, live notifications and complete aggregate budgets |
| B — AG1 integration | Actual transport execution through the shared executor, atomic send/credential/scope fences, canonical account/project scope, model request preparation, context/CV/artifacts and native result DTOs |
| C — Authentication/trust | Dedicated MCP OAuth and private configuration, account/resource-separated vault custody, reviewed risk rules, revocation races, local execution policy/containment and durable startup reconciliation |
| D — Native manager | Installed/details/forms/inspection, permissions/auth/log views, application service composition and actual wide/narrow Tauri workflows |
| E — Discovery/setup | Revalidate a permitted public registry source; cache/search/provenance, inactive setup drafts, supported recipes and reviewed activation/manual fallback |
| F — Acceptance/install | Complete cross-feature/adversarial matrix, discovery-to-use and manual native workflows, exact production build/startup/package/install verification |

Next slice: implement bounded MCP operation/result adapters against owned local and remote fixtures, through the existing executor, with native revalidation immediately before send. Keep model/UI execution unavailable until those fences and result custody are verified. Other feature tasks, real-model testing and the separate duration programme remain paused.
