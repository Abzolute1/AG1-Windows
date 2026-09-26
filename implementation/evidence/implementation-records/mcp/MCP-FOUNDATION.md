# MCP implementation ledger

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**Historical A1 checkpoint. Continued implementation is recorded in [MCP-STDIO.md](MCP-STDIO.md); its status and resume point supersede this first-slice snapshot.** The original A1 test binary and evidence remain preserved.

Date: 2026-09-11. Scope: task 2, phase A, first foundation slice. **Phase A is not complete. MCP is not yet available in the desktop or to the model.** The user requested implementation, then flagged 16% remaining weekly usage. This is a bounded source checkpoint and resume record; no numerical estimate of available Codex tokens is implied.

## Delivered source

- [Native MCP module](../../../../product/crates/ag1-platform/src/mcp/mod.rs): explicit revision labels for `2025-11-25` and `2026-07-28`; all other labels refused. This is not negotiation or evidence of transport compatibility.
- [Wire intake](../../../../product/crates/ag1-platform/src/mcp/wire.rs): bounded JSON deserialization, duplicate-key refusal at every depth (including escaped-equivalent keys), envelope classification, UTF-8/trailing-content checks, incremental newline-delimited stdio framing and permanent reader retirement after invalid/truncated/oversized input. An application `isError` remains in its original envelope; classification does not pronounce tool success.
- [Private tool catalogs](../../../../product/crates/ag1-platform/src/mcp/catalog.rs): bounded `tools/list` pages, duplicate-name/cursor refusal, atomic complete publication, refresh failure retirement, per-owner and generation fences, native server/configuration/authentication/connection/scope binding, deterministic names and metadata digests. General-chat scope does not require a filesystem path. This binding carries native identity; it does not validate grants or authorize effects.
- Tool metadata and unsupported input/output schemas remain inspectable. A conservative structural subset uses the existing core validator with **no scalar coercion**. Constraint-bearing schemas are never stripped or flattened. Validation does not register or activate tools.

No commands, processes, network requests, credential lookups, persistence mutations or model tool calls are reachable through this module. There is no convenience connection or execution bypass. The existing native/GitHub dispatcher, model preparation and tool registry are unchanged.

## Current budgets

| Boundary | Limit | Enforcement |
| --- | --- | --- |
| JSON/frame input | 1 MiB, excluding stdio LF | Before parsing; stdio checks before appending |
| JSON complexity | 32 nesting levels, 20,000 values | During deserialization, before descending/allocating further containers |
| JSON limit customization | Positive values up to those ceilings | `Limits::new`; no upward override |
| Tool catalog | 1,024 tools, 32 pages, 8 MiB received bytes | Cumulative across refresh, including unknown page metadata |
| Input/output schema | 256 KiB each | Unsupported/inspectable if exceeded within an otherwise bounded page |
| Tool arguments | 256 KiB, 32 levels, 20,000 values | Byte intake before strict validation |
| Structured output | 1 MiB, 32 levels, 20,000 values | Byte intake before optional schema validation |
| Names/native identity labels | 256 bytes | Catalog intake/binding validation |
| Tool description / cursor | 16 KiB / 1 KiB | Catalog intake |

These are per-codec/refresh budgets, **not aggregate host-memory guarantees**. A future manager must bound simultaneous readers, refresh builders, queued requests and retained catalogs. `StdioReader` is a synchronous codec for owned worker use; it supplies neither deadlines nor process ownership. No live transport may adopt it on the UI/session-owner thread.

The initial schema subset accepts primitive type declarations, object properties/required/additionalProperties, array items/counts, string lengths and property counts, plus the core validator's inert annotations. References, unsupported keywords, regexes, numeric comparisons, enum/const/uniqueItems and combinators are unavailable pending an audited compatible validator. Complete originals are retained within the catalog limits. Current-revision output schemas can describe scalar/array output; legacy output schemas must describe objects. This is deliberately limited schema support, not full JSON Schema conformance or complete MCP result decoding.

## Verification and dependency identity

Contract suite: [mcp_foundation_contract.rs](../../../../product/crates/ag1-platform/tests/mcp_foundation_contract.rs). Covers malformed frames, duplicate keys, limits, stdio fragmentation/EOF, stale and foreign refreshes, scope/config/auth/connection changes, strict arguments, revision-specific output schemas, unsupported schemas and 1,000-tool inventory. The inventory test does **not** establish the phase B 1,000-tool outgoing-context gate.

```sh
CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target CARGO_INCREMENTAL=0 cargo test --offline --locked --manifest-path product/Cargo.toml -p ag1-platform --test mcp_foundation_contract
```

Compiler used: `rustc 1.97.1 (8bab26f4f 2026-07-14)`. Workspace declared minimum: Rust 1.88; minimum-version execution has not been verified. No dependency or Cargo lockfile changes. Foundation reuses `serde = 1.0.229`, `serde_json = 1.0.151`, `sha2 = 0.10.9` and `ag1-core`; upstream licenses are MIT/Apache-2.0 for these three crates and existing repository notices remain applicable. Cargo.lock SHA-256: `9eee2f8206b297c53242120a91578b5cc9f029516c1747dd7ce79caf3cc8addc`.

Test outcome and exact source/test-binary hashes are recorded in [MCP foundation evidence](../../MCP/foundation.json). No full native/GitHub regression, production distribution, installation, native visual audit or real inference was performed. This source-only slice is not a major usable product checkpoint; retain the installed GitHub checkpoint. Production build/install remains required after a major verified MCP checkpoint.

## Research refreshed at implementation start

- The [official versioning page](https://modelcontextprotocol.io/docs/2026-07-28/learn/versioning) identifies `2026-07-28` as current and describes per-request revision metadata and optional upfront discovery. Keep its lifecycle separate from the legacy initialize/initialized exchange.
- The [official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) documents explicit discovery/legacy lifecycle selection. Its [workspace manifest](https://github.com/modelcontextprotocol/rust-sdk/blob/main/Cargo.toml), read on this date, reports `3.3.0`, Rust 1.88, Apache-2.0. **No SDK was acquired, pinned or integrated.** These mutable-source observations are candidate research, not a dependency lock or conformance result. Complete exact-release/commit/dependency/license inspection before the transport slice.
- The [legacy transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) supplies newline framing and distinguishes Streamable HTTP from standalone HTTP+SSE. Only framing has landed here; neither live transport is implemented.
- [Registry consumer guidance](https://modelcontextprotocol.io/registry/registry-aggregators) still describes preview metadata, infrequent persisted aggregator consumption and the `v0.1` paged interface. The [registry overview](https://modelcontextprotocol.io/registry/about) directs host apps toward downstream sources. No public discovery source has been selected; this remains an explicit research gate for core discovery. No AG1 registry backend is planned.

## Next resume point and phase status

1. Finish phase A SDK selection and exact dependency/license pinning. Audit child custody, framing allocation, redirects, retries, caching and optional requests before adopting SDK convenience APIs.
2. Implement approved, owned fixture transports: process-group ownership and wait/reap for stdio; destination policy, bounded HTTP/SSE and deadlines for Streamable HTTP. Test current discovery and legacy initialization independently. Implement request correlation, cancellation, unsupported-request refusal and bounded continuation.
3. Add resources/templates/prompts, normalized mixed/error results and full protocol fixtures. Complete aggregate resource budgets and the A capability matrix.
4. Phase B: explicit MCP registry origin, atomic selected-schema publication, shared executor/approval/durable result path, account-only scope resolver and context/artifact integration. The catalog in this slice is private metadata, not an alternate execution registry.
5. Phases C–F remain unimplemented: issuer/resource/account-separated vault and OAuth, launch containment/trust, native manager, public discovery/guided setup and acceptance. None of those requirements have been dropped.

Architecture boundary: platform owns transport and private discovery adapters; core retains permission/tool identity/result authority; storage will own durable definitions and grants; one native composition service will own connection leases. Authentication namespaces must bind server/profile/issuer/resource/account rather than reuse provider/GitHub keys. Catalog bindings and explicit general-chat scope begin that separation, but final manager ownership, grant resolution and vault integration are still design/implementation work. Do not wire model execution until those boundaries are enforced.
