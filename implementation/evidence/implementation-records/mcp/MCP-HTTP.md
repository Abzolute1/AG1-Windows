# MCP phase A — Streamable HTTP discovery checkpoint

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**Historical A3 checkpoint. The [native manager checkpoint](MCP-MANAGER.md) records the current continuation and remaining work. Original binaries/evidence remain preserved.**

Date: 2026-09-11. **Phase A is partly implemented; phases B–F are not delivered. Most of the product implementation remains.** This is an internal source checkpoint, not a production installation or a user-facing MCP feature.

## What changed

[HttpDiscovery](../../../../product/crates/ag1-platform/src/mcp/http.rs) adds asynchronous, unauthenticated discovery against a single native-configured MCP endpoint. Both JSON responses and bounded request-scoped SSE streams support negotiation, complete tool catalogs and resource/template/prompt listings. No method invokes tools, reads resource bodies, retrieves prompts or resolves credentials.

[Shared discovery contracts](../../../../product/crates/ag1-platform/src/mcp/discovery.rs) now provide the same revision validation, metadata checks and DTOs to HTTP and stdio. Existing stdio public type paths remain available through re-exports. The [incremental SSE decoder](../../../../product/crates/ag1-platform/src/mcp/sse.rs) handles split UTF-8, BOM, CR/LF/CRLF, comments, multiline data and bounded priming events. Event IDs and retry fields never cause navigation or resumption.

The [current HTTP specification](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) requires request metadata headers, removes protocol sessions, and forbids standalone server requests. Current requests carry matching native `MCP-Protocol-Version`, `Mcp-Method` and body metadata; they never send session IDs, GET streams, DELETE or client JSON-RPC responses. Cancellation closes the response stream. Unsupported multi-round-trip results fail explicitly.

The [legacy transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) retains initialize/initialized and optional sessions. Legacy session headers stay private to one native instance and immutable endpoint/binding. Replacement/duplicate/invalid session headers fail. Shutdown distinguishes acknowledged deletion, unsupported deletion and local-only disconnect. Unsupported legacy peer requests receive bounded JSON-RPC refusals at the same endpoint. Explicit cancellation after response headers attempts a bounded cancellation notification before retiring local state.

The same protocol review corrected A2's stdio handling: `2026-07-28` standalone peer requests now retire the connection instead of receiving legacy JSON-RPC responses. The existing sampling-refusal success fixture now uses the legacy revision; an added current-revision fixture proves rejection. Frozen comparison expectations were not changed.

## Network and authority boundaries

- Endpoint construction is inert. Public endpoints require HTTPS; explicit development exceptions accept only literal loopback addresses at the configured URL/port. Username/password, query strings, fragments, whitespace and backslash normalization are refused. Private LAN and hostname-based development exceptions remain unsupported.
- AG1's existing `WebResolver` and public-address policy validate DNS before each HTTP request. All returned addresses must be permitted; checked addresses are pinned into a fresh client. TLS still verifies the configured hostname and chain. Native fixture trust roots are explicit and never modify the system trust store.
- Redirects and library retries are disabled. There is no ambient proxy, cookie jar, authorization lookup, decompression, transparent endpoint fallback, session recovery or automatic replay. Schema-controlled HTTP routing is not activated; tool execution and the `x-mcp-header` compatibility gate remain future work.
- Cancellation or a dropped operation future closes its owned request and releases the shared permit. Usable state is retired before awaiting work, so dropping a future cannot leave an old executable catalog. Remote effects are never claimed to have stopped merely because the socket closed. For a legacy request cancelled before response headers arrive, no additional cancellation connection is opened; remote completion remains unknown.
- Native manager consent, config/credential/scope generation fencing and durable intent/result custody are still required before exposing production activation or effecting calls. No renderer/model entry point connects a server today. These native APIs do not authorize themselves.

## Bounds and limitations

| Boundary | Enforced behavior |
| --- | --- |
| Aggregate network work | Global ceiling of 8 concurrent HTTP discovery operations; fail-fast saturation; permits cover all pages and peer responses |
| Deadlines | Default whole operation 15 seconds, configurable up to 30; DNS/connect at most 3 seconds; body idle at most 5 seconds; legacy cancellation cleanup at most 100 ms |
| Requests | At most 4 KiB body; native header names/values only; no arbitrary caller headers |
| Bodies | JSON at most 1 MiB; aggregate received operation bytes at most 8 MiB; content-length checked before body read; chunk accounting before copying/decoding |
| SSE | 1 MiB accumulated event/line data, 128 event boundaries per stream, 128 incoming protocol messages per operation, 8 MiB total stream bytes |
| JSON | Existing 32-level / 20,000-value parser; duplicate keys and malformed/trailing bytes refused |
| Headers | Application limit 64 fields / 16 KiB after HTTP parsing; singleton protocol headers checked explicitly |
| HTTP parser | HTTP/1 only; pinned Hyper 1.11.1 has a 417,792-byte default read-buffer ceiling before application header checks; the 16 KiB application limit is not a pre-allocation claim |
| Sessions | Legacy only, 1–1,024 visible ASCII bytes, sensitive header representation, no Debug/Serialize exposure or cross-instance reuse |
| Catalogs | 32 pages / 1,024 entries and existing tool-catalog limits; repeated cursors and changed capability generations refuse publication |

These bounds do not claim aggregate host-memory containment or full MCP transport conformance. Persistent background health/subscription management, legacy SSE resumption/GET streams, compressed entities, credential-bearing query URLs, OAuth, arbitrary private networks and HTTP/2 are not supported by this slice. Catalogs remain metadata snapshots, not execution authority. Actual template expansion/prompt use/resource reads, continuations and normalized tool output remain unfinished.

## Verification and regression attribution

**All 50 MCP tests pass:** 14 live HTTP/HTTPS contracts, 15 stdio contracts, 19 foundation contracts and 2 incremental SSE unit tests. [HTTP fixture contracts](../../../../product/crates/ag1-platform/tests/mcp_http_contract.rs) cover both revisions and body formats, exact headers/session lifetime, real certificate verification, zero-effect endpoint construction, DNS/private-address refusal, redirects, errors, floods, truncation, malformed bodies, no replay, cancellation/future drop, peer requests and complete pagination deadlines. Servers run in owned local threads with bounded sockets and joined shutdown; no real service account or inference is used.

The broader `web_http_contract` suite reports **14 pass / 2 fail**. The failures are:

- `compressed_entities_are_bounded_after_decoding_and_match_reference_projections`: existing source-extract wording differs from a frozen source-summary expectation.
- `selected_fetch_content_types_docs_hints_and_complete_event_coverage_reach_native_results`: existing output omits the frozen “Coverage complete” sentence.

These names are **not** the seven failures listed in the GitHub final comparison report. A saved 2026-09-08 web-test binary passes both, so it was not used to assert an unchanged baseline. An isolated copy of the current workspace with `pub mod mcp;` removed reproduces the same two failure reasons, with all other tested source and expectations preserved. This attributes them to existing workspace web-comparison behavior rather than the MCP module. No expectations or unrelated web implementation were changed. Baseline/source checks and logs are retained under `.local/mcp-a3-web-baseline/` and in [HTTP evidence](../../MCP/http-discovery.json).

Commands used `--offline --locked`, `CARGO_INCREMENTAL=0` and the required target directory with a fresh `mcp-a3` profile (`inherits="dev"`, `debug=0`). MCP integration tests and the web regression suite were run with four test threads. SSE units were run with `--lib mcp::sse::tests`. The diagnostic baseline used a separate `mcp-a3-baseline` profile. Its first preparation attempt lacked the copied product LICENSE; that preparation log is retained, then the unchanged LICENSE was copied and the comparison completed.

The exact commands, test/binary/source hashes and baseline failures are in [http-discovery.json](../../MCP/http-discovery.json). A1/A2 binaries and Cargo.lock were checked unchanged. SDK adoption remains as documented in the [3.3.0 source audit](../../MCP/sdk-3.3.0/audit.json); no new Cargo dependency was added. No distribution, model configuration or installed runtime was changed. Future source checkpoints must use fresh profile names to preserve these evidence binaries.

## How much remains / next implementation

| Phase | Current state and remaining work |
| --- | --- |
| A — Native/protocol foundation | Discovery works on both transports and revisions. Still needs final SDK adoption/conformance, broader schema compatibility, supported input continuations, complete operation/result adapters, persistent lifecycle ownership and aggregate resource/containment evidence. |
| B — Shared AG1 integration | Not delivered: explicit MCP tool origin, selected dynamic schemas, ordinary executor/approval/durable intent/result path, scope/account filtering, context/CV and artifact integration. Existing binding types are only groundwork. |
| C — Authentication/trust | Not delivered: OAuth discovery/PKCE/callbacks, issuer/resource/account-separated vault, private config, native reviewed grants, launch containment and revocation races. |
| D — Native manager | Not delivered: Installed/details/configuration, connect/disconnect/enable/disable/remove, inspection, authentication controls, diagnostics and native visual verification. |
| E — Public discovery/setup | Not delivered: select a permitted existing public source; cache/search, inactive drafts, reviewed recipes and guided activation/manual fallback. No AG1 registry backend. |
| F — Acceptance | Not delivered: cross-feature adversarial/scale/native workflows, full regression, production packaging/startup/install verification. |

Next, implement the native definition/connection ownership and reviewed scope contracts that operations will require, then route tool/resource/prompt operations through the existing authority and durable-custody path. Finish compatible result/schema/continuation handling using owned fixtures. This avoids creating a temporary unrestricted call API while authentication and manager work are still pending.

No reliable percentage or calendar estimate follows from test counts; the remaining UI, authentication and shared execution work is larger than the completed discovery foundation. The separately paused chat cleanup, combined real-model verification and duration programme remain outside this implementation resume.
