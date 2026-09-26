# MCP phase A — owned stdio discovery checkpoint

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**Historical A2 checkpoint. [MCP-HTTP.md](MCP-HTTP.md) records the current A3 source and resume point.** A3 corrects current-revision standalone server requests to protocol refusal; legacy requests still receive explicit unsupported-capability responses. All A2 evidence binaries remain unchanged.

Date: 2026-09-11. User requested continued implementation after [A1](MCP-FOUNDATION.md). **Phase A remains incomplete.** This checkpoint adds a functioning discovery-only stdio client over native owned processes. It is tested against isolated local fixtures and is not connected to desktop commands or model execution.

## Implemented behavior

- [StdioDiscovery](../../../../product/crates/ag1-platform/src/mcp/stdio.rs) consumes an already owned native process and native server/scope/config/auth/connection binding. It provides negotiation, complete tools refresh, resource/template/prompt listing and shutdown. It cannot invoke tools, read resources, get prompts, resolve secrets, acquire packages or initiate OAuth.
- Legacy `2025-11-25` uses `initialize`, checks the exact returned revision, then sends `notifications/initialized`. Current `2026-07-28` uses `server/discover`, checks the supported revisions and complete discovery metadata, and attaches protocol/client metadata to every request. No hidden version fallback or reconnect/replay occurs.
- Single-flight request IDs are native and monotonically generated. Unexpected IDs, malformed frames/results, oversized input, repeated cursors, duplicate metadata, changing capabilities, exhausted budgets and unsupported continuations fail closed and retire live catalogs. Application/protocol error contents do not enter diagnostic strings.
- Unadvertised capabilities are not queried. Unsolicited sampling and other unimplemented client requests receive JSON-RPC method-not-found; ping receives an empty result. No sampling/roots/elicitation capabilities are advertised. List-change notifications invalidate relevant discovery generations when received.
- [ProcessSupervisor::spawn_mcp_protocol](../../../../product/crates/ag1-platform/src/process.rs) reuses the existing authenticated helper, explicit environment, capacity accounting, private control socket, pidfds and subreaper cleanup. The MCP variant discards stderr **at capture** pending a streaming redactor. Raw arbitrary server stderr is neither logged nor exported.
- Cancellation before send produces no protocol request. Cancellation/deadline expiry after a request attempts a bounded cancellation notification, then stops and reaps the owned process. Shutdown and Drop retain existing process custody, including descendants that create their own session. Cleanup failure is surfaced explicitly; no public API accepts a PID to kill.

Native process APIs are host capabilities, not renderer commands or substitutes for activation consent. The future MCP manager must bind reviewed launch identity, effective access, grant revisions and credential profiles before composing these APIs. Fixtures run explicitly reviewed code with host-account access; **no filesystem/network sandbox is claimed**. No production entry point starts an MCP process today.

## Bounds and observable limitations

| Boundary | Current behavior |
| --- | --- |
| Owned process | Nonresident; at most 10-minute configured lifetime; 1 GiB address-space ceiling; existing supervisor ceiling of 32 owned children per supervisor |
| Child file output | Existing private-process 256 MiB per-file limit; not an aggregate disk quota |
| Diagnostic stderr | Discarded at capture; metadata errors remain available; no raw stderr view yet |
| One operation | Default 15 seconds, caller may select positive values up to 30 seconds; one deadline covers every page, write, read and peer request |
| Input | 1 MiB frame, 32 KiB read-ahead, 8 MiB received bytes per operation including carried read-ahead, 128 incoming messages; A1 JSON depth/node limits still apply |
| Output | Only bounded discovery requests/refusals/notifications, up to 4 KiB each; partial writes retry within the same deadline |
| Listing | Up to 32 pages and 1,024 entries per list; opaque cursor up to 1 KiB; duplicate identities/cursors refused |
| Peer input | Unsupported continuations refused; bounded unsupported client requests answered without forwarding private payloads |
| Shutdown | Existing supervisor stop/reap deadline; an additional cancellation write has at most 100 ms |

`StdioDiscovery` is a synchronous worker component. It must not run on the UI/session-owner thread. There is no background message pump, idle health tracking or persistent connection manager yet; notifications are processed while exchanging requests, and returned catalogs/health are discovery snapshots, not execution leases. The manager must bound concurrent connections/builders and provide authoritative freshness revalidation before phase B execution. Per-process address-space and per-file limits do not establish aggregate containment.

Resources, templates and prompts retain bounded untrusted metadata only. Their URIs cannot grant local/network access or create artifacts. Prompt arguments are inspected for bounded names/types/duplicates; actual template expansion, prompt use, resource reads and normalized content/result custody remain to be implemented.

## SDK pin and adoption decision

The official `rmcp` **3.3.0** crate archive was downloaded for source audit, bounded and verified against the crates.io SHA-256:

`b88db56b8ae316560e9e868b6b978ea940f27cb883323fc90e07435e44158f5c`

The exact archive commit, compiler minimum, nonoptional dependency declarations, source hashes and license provenance are in [sdk-3.3.0/audit.json](../../MCP/sdk-3.3.0/audit.json); the upstream Apache-2.0 [license](../../MCP/sdk-3.3.0/LICENSE) is retained. Archive/source custody is under `.local/mcp-sdk-audit/`. See the [official release](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.3.0) and [crate metadata](https://crates.io/api/v1/crates/rmcp/3.3.0).

The audit identified unbounded `read_until` allocation in the convenience stdio reader, inherited stderr and async-runtime-dependent Drop cleanup in the child-process transport. HTTP also has routing-header caches and retry/recovery logic needing further authority review. Therefore the native bounded transport/process owner is retained. SDK request/discovery model source was used to check fixture wire shapes; SDK code is **not integrated, compiled or shipped**. Runtime SDK feature selection, resolved transitive license report and SDK conformance/interoperability testing remain phase A gates. No Cargo dependency or lockfile changed.

## Verification

**62 tests passed, zero failures:**

| Suite | Passed | Evidence |
| --- | --- | --- |
| [MCP stdio contracts](../../../../product/crates/ag1-platform/tests/mcp_stdio_contract.rs) | 14 | Both lifecycles/all four capability kinds; no invocation; exact revision refusal; malformed/truncated/oversized frames; ID correlation; sampling refusal; notification floods; stale catalogs; unsupported continuation; cancellation and whole-pagination deadline; independent connection cleanup; reaped detached descendants; stderr capture refusal; launch lifetime guard |
| [MCP foundation contracts](../../../../product/crates/ag1-platform/tests/mcp_foundation_contract.rs) | 19 | Prior JSON, schema, catalog and 1,000-tool intake coverage preserved |
| [Platform contracts](../../../../product/crates/ag1-platform/tests/platform_contract.rs) | 29 | Existing process/PTY/ownership/cleanup, stdout/log separation, filesystem/workspace and Git contracts passed |

The [owned Python fixture](../../../../product/crates/ag1-platform/tests/fixtures/mcp_stdio.py) needs only `/usr/bin/python3` and its standard library. It runs in a temporary directory, asserts that the parent environment is not inherited, records methods, and refuses any tool call/resource read/prompt retrieval. Tests are live local subprocess exchanges, not real inference, remote service tests or an official SDK conformance suite.

Tests used an isolated Cargo profile to preserve the A1 accepted binary and existing debug/release helpers:

```sh
CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target CARGO_INCREMENTAL=0 cargo test --offline --locked --manifest-path product/Cargo.toml --config 'profile.mcp-a2.inherits="dev"' --config 'profile.mcp-a2.debug=0' --profile mcp-a2 -p ag1-platform --test mcp_stdio_contract --test mcp_foundation_contract --test platform_contract
```

Source/test/helper hashes and result counts are in [stdio-discovery.json](../../MCP/stdio-discovery.json). The A1 test-binary hash and original Cargo.lock hash were checked unchanged. No installed distribution, model profile, inference configuration or GitHub implementation was changed. No full GitHub/UI regression or native visual audit was performed; this is not a production install checkpoint.

Use a fresh Cargo profile name for the next source checkpoint; do not rebuild binaries referenced by this accepted evidence in place.

## Resume here

1. Implement bounded Streamable HTTP/JSON/SSE on owned loopback fixtures, with destination/redirect policy and explicit revision/session behavior. Keep HTTP recovery from replaying effecting calls.
2. Complete SDK adoption/conformance decisions and the per-revision transport matrix. Then add bounded supported input continuation, resource/template/prompt operations and mixed/error-result custody under the shared native permission path.
3. Complete native manager ownership, launch containment/review, storage/vault namespaces and aggregate budgets before activating tools. Add explicit MCP core origin and shared executor/approval/context/artifact integration in B; auth/trust in C; native management in D; discovery/guided setup in E; acceptance/build/install in F.

Public registry selection remains open. The task still requires Installed/Discover and both transports; these requirements have not been dropped. Chat cleanup, combined real-model verification and the separate long-duration programme remain paused.
