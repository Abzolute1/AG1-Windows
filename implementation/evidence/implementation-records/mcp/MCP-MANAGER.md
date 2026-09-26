# MCP phase A — native definition and discovery ownership

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Date: 2026-09-12. The user resumed MCP implementation. **This is the A4 source checkpoint, following [HTTP discovery](MCP-HTTP.md). Phase A and the complete A–F product remain unfinished.** MCP is not yet connected to desktop navigation or model execution. This foundation checkpoint does not change the installed application; a production build/install remains required at the next major usable product checkpoint.

## Delivered behavior

- [Storage](../../../../product/crates/ag1-storage/src/mcp.rs) uses the existing Store, SQLite transactions, blob publication, checksums and fault boundaries. One atomic revisioned state contains separate server definitions and availability records. Stale CAS writes fail. The ordinary metadata writer cannot overwrite the MCP namespace. Successful superseded MCP state blobs are pruned only when unreferenced; unrelated blobs and failed-publication evidence are preserved. A failed/uncertain write retires the manager and cancels its connections until reopen/readback.
- [Definitions and import](../../../../product/crates/ag1-platform/src/mcp/definitions.rs) preserve public configuration, native UUIDs, supported protocol choice, source provenance, private-input names and opaque authentication-profile references. Definitions always enter as disabled drafts. Saving, importing and reopening perform no process launch, DNS, endpoint contact, credential lookup or automatic reconnect. Configuration edits clear availability; stale revision changes and remove/re-add cannot transfer an old review.
- [Manager](../../../../product/crates/ag1-platform/src/mcp/manager.rs) owns one moved Store and up to four discovery workers. A dedicated user MCP Store is intended for native composition; Store's exclusive writer lease prevents two owners of the same destination. Each local/remote connection has independent cancellation, scope, identity, status and private catalog custody. No generic method-call or tool-execution route is added to the renderer or model dispatcher.
- Workspace availability and general-chat availability are independent. All-workspaces does not imply general chat. Explicit workspace denial overrides all-workspaces and individual inclusion. Scope identifiers are supplied by the native caller; the later desktop/session composition must resolve canonical project identities from the existing native authority.
- A connection requires a consumed, manager-owned review witness bound to the exact record revision, scope, target and 60-second expiry. Remote HTTPS, literal loopback development and local host-account execution have different explicit consent variants. Local review captures executable digest/device/inode and cwd device/inode, rechecks the paths/content before launch, and passes held descriptor paths through the existing process supervisor. FIFOs and nonregular executables are refused without blocking on a writer. This protects against path replacement; it does **not** freeze dependencies, argument-referenced files or deliberate in-place executable mutation.
- Explicit native **Test connection** can inspect an inactive draft. Its catalog is accessible through the native inspection path and cannot enter scope-enabled catalog selection. Enabling afterward changes revision and retires the inspection connection. Scope availability alone does not approve any tool action.
- Reviewed local connections use existing owned process custody, cleared environment plus explicitly configured public variables, discarded stderr, bounded process lifetime and actual stop/reap. Local execution currently requires deliberate **host-account execution** consent; it is not filesystem/network confined merely because a workspace was selected. No package acquisition or shell interpolation is added.
- Both supported revisions negotiate and list tools, resources, templates and prompts over stdio and HTTP/JSON/SSE. The combined snapshot publishes only after all advertised lists succeed. Unadvertised lists are skipped. Unsupported tool schemas remain inspectable with their existing wire names, digests and availability reasons. Tool listing runs last because HTTP retires its tool slot during other list operations. Notification generations across the complete discovery fence prevent stale partial publication.
- Disconnect immediately blocks catalog access and signals cancellation without changing saved scope availability. Disable/edit/remove cancel affected workers and block late publication. `wait_disconnected` joins owned work off the UI/session owner thread. Manager teardown fans cancellation out before joining children. Restart restores definitions/grants but never live catalogs, sessions, reviews or connections. Cancelling one scoped connection leaves another scoped lease running.
- Diagnostics use bounded native codes without raw peer errors, headers, arguments, resources or stderr. The last 128 completed/failed connection statuses survive later connections within this owner lifetime. They are not yet persistent user-facing logs. HTTP Ready means the last discovery succeeded, not continuous remote liveness; idle push-notification/health management and execution-time freshness are still pending.

## Import matrix

The explicitly supported project format is a bounded JSON object with exactly one `mcpServers` map. Each entry accepts an absolute `command`, optional string-array `args`, and optional absolute `cwd`; otherwise the native caller supplies the canonical project directory. Import records the source path, content SHA-256 and original entry name.

Imports with relative/package-runner commands, environment fields, remote URLs, unknown fields, duplicate keys or malformed entries fail as a whole with manual-configuration/private-entry guidance. No environment value is silently copied or discarded. The native manual draft form can represent public environment values, secret variable names and remote endpoints; authentication profiles/private environment remain unavailable for connection until their native credential flows are implemented.

## Budgets and limitations

| Boundary | Implemented ceiling |
| --- | --- |
| Saved definitions | 128 servers; 16 KiB each; 1 MiB whole state; existing 32-level/20,000-node bounded parser also applies |
| Draft/import batch | 32 entries; import 256 KiB, 16 levels, 4,096 nodes |
| Local configuration | 64 arguments; 32 public/secret environment names combined; bounded 4 KiB paths/values/arguments |
| Scope configuration | 128 explicit workspaces and 128 denials per server |
| Pending native reviews | 32 per manager; 60-second expiry; consumed or dropped witnesses release the slot |
| Local review identity | Executable content at most 128 MiB, hashed in 64 KiB chunks; held file/cwd descriptors |
| Live discovery | Four workers/connections across manager instances in this process; no operation queue or automatic retry |
| Private catalogs | 16 MiB total encoded metadata per connected snapshot; at most four retained snapshots; existing per-transport 8 MiB received-operation and catalog limits still apply |
| Discovery deadlines | Up to five sequential bounded operations; each defaults to 15 seconds and caps at 30 seconds, including all pagination |
| Local process | Existing 600-second maximum lifetime; 1 GiB direct-child address-space ceiling; owned process-tree shutdown; no raw stderr retention |
| Cancellation | Local idle checks every 25 ms; active operations use their existing cancellation/deadline mechanisms; remote session cleanup gets at most 500 ms |
| Diagnostics | 128 settled status records plus four current workers; native codes only; no raw server payload/log spool |

The metadata byte ceiling is not an RSS guarantee: parsed objects, compiled schemas, transport buffers and an in-progress list have overhead. Existing per-message/parser budgets remain in force, but whole-host RSS, descendant/cgroup memory/process/disk containment and crash-orphan disk quotas are not yet established. Failed publication blobs are deliberately retained for diagnosis. Review/definition storage and join APIs must be scheduled on native workers, not the UI/session owner. No production activation path is wired yet.

The low-level transport modules remain usable by owned fixtures as before. This manager is the native composition foundation, not proof of phase B executor integration or phase C production trust/authentication. Logical scope availability grants neither filesystem access nor an action grant. Profile/secret placeholders are inspectable but explicitly refused on connection; no provider/GitHub fallback credentials are used.

## Validation and evidence

**157 selected tests passed:** 71 MCP tests (19 foundation, 15 HTTP, 15 stdio, 17 manager, two storage and three units), plus 14 native GitHub, 29 platform/process and 43 storage regressions. Formatting checks passed. Clippy completed with existing repository warnings; the new manager/definition/storage modules produced none. No full-product acceptance or native visual audit is claimed.

See [manager evidence](../../MCP/manager-ownership.json) for exact commands, source/binary hashes, suite results, diagnostic iterations and preserved prior checkpoint binaries. All verification uses owned local fixtures and isolated scratch stores; no real MCP service credentials or inference are used. The prior HTTP failure was found in the new combined-manager fixture: HTTP correctly invalidated its tool cache during subsequent list operations. Collecting tools last fixed the integration while preserving that transport safeguard. Failed-run logs remain retained.

The SDK/dependency identity remains the [existing rmcp 3.3.0 audit](../../MCP/sdk-3.3.0/audit.json); no SDK runtime or Cargo dependency was added. Official [legacy transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) and [current versioning](https://modelcontextprotocol.io/docs/2026-07-28/learn/versioning) documents were checked on this continuation. No new protocol-revision or registry-source support is claimed.

## Next implementation

| Phase | Remaining work |
| --- | --- |
| A | Final SDK adoption/conformance, broader schema support, operation/result adapters, supported input continuation, live notification freshness and complete aggregate resource budgets |
| B | Connect this native service to existing registry/authorizer/executor and durable intent/original/terminal result custody; selected schemas, canonical workspace/account scope resolution, context/CV and artifacts |
| C | Dedicated issuer/resource/account-separated MCP authentication/private entry, grants and revocation races; production local containment or reviewed host-access policy and durable startup reconciliation |
| D | Native Installed/details/forms/permissions/auth/log views, service composition and actual wide/narrow Tauri workflows; native review witnesses must come from those reviewed commands |
| E | Select/revalidate a permitted existing public registry source; cache/search, provenance, guided recipes and reviewed activation/manual fallback |
| F | Complete cross-feature/adversarial/native acceptance, normal production packaging/startup/install verification |

Next, use the manager's bound records and connections to design the existing executor's MCP origin/admission and durable intent/result adapters; keep tool/resource/prompt execution unavailable until those checks and credential/scope fences are present. This avoids treating a discovery snapshot or successful connection test as action authority. UI, authentication, execution and public discovery still account for most of the remaining implementation; test counts do not provide a reliable percentage estimate. Other feature tasks, real-model testing and the separate duration programme remain paused.
