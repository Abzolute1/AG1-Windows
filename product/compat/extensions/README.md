# Optional extension host — P09 work in progress

The native owner launches this Node 24 helper only after resource capture and
project trust admission. `runtime.mjs` loads admitted JavaScript/TypeScript entry
points using the vendored Jiti and TypeBox distributions. It imports no
AgentSession, parent journal, credential store or model HTTP client.

Trusted extension code has the host account's process privileges. This is **not an
OS sandbox**. Entry SHA checks protect the admitted entry revision. Jiti evaluates
the captured bytes directly, preserving the entry filename for relative imports and
its own `node_modules`; it does not reopen the entry after verification. Tests replace
the path between capture and evaluation and verify the admitted factory still runs.
These checks do not confine imports or an extension's own filesystem/network/process operations.
Loading a trusted extension authorizes its factory to run. Native tool approvals
control subsequent core-dispatched tool execution and parent callbacks.

The native implementation is in `ag1-platform`: `extension_host` owns the process,
`extension_tools` binds tools to the native registry/approvals/durable executor,
and `extension_inference` uses the shared P06 scheduler and native HTTP transport.
The concrete `ConfiguredModels` resolver binds admitted extension owners and
scope to canonical model configuration, allowed options and endpoint identity. Its
native credential port supplies API keys, OAuth bearer/Gemini CLI credentials or
AWS credentials to the P06 request builder and signer. Incoming model URLs/credentials never construct a request.
The composition host must share its existing Scheduler and use a native blocking
worker for this synchronous process adapter. `extension_providers::RegistryBridge`
binds dynamic registrations to one helper incarnation and explicitly granted provider
names. `CustomModels` separately grants each caller, model, hook owner, origin and
shared endpoint; private native credential options enter only the admitted hook.
The native `extension_auth::Service` now runs version-bound login, refresh and API-key
hooks against the shared P06/P05 vault. `extension_login` supplies the native login
UI port, including optional manual-code entry and cancellation. `KeyBridge` grants
key disclosure separately from private custom-stream credential recipients.
Registered OAuth credentials refresh through the native vault before admitted
custom-stream and native HTTP inference. Native HTTP credentials require explicit
model and destination grants plus an exact match to the current catalog model.
Native logout stores a revision-preserving revocation marker, preventing an older
interactive login from restoring credentials. After credential changes, the native
controller calls `RegistryBridge::refresh` to rebuild admitted models and acknowledge
the public view in the helper. P10/P12 supply desktop authentication routing.

## Compatibility status

| Surface | Implemented behavior | Remaining work / explicit limits |
| --- | --- | --- |
| JS/TS loader | Admitted entry hash, factory, local imports, TypeBox; load error isolation | All 27 selected discovery cases have audit evidence; captured-entry race and own-node_modules/relative-import cases pass |
| Tool definitions | First name wins, prepare/execute/update, schema through Rust; tool/flag conflicts reported with both owners | Dynamic refresh publishes at the native dispatch boundary; same-schema replacements invalidate approvals; built-in collisions rejected atomically |
| Tool lifecycle | Native preparation, validation, policy/approval ticket, acknowledged intent, original artifact, post-hook projection, acknowledged terminal result | P10 composition routing; arbitrary trusted-code I/O cannot certify a native transaction |
| Hooks | Input, tool call/result, context, provider payload, before-agent-start, resources, bash, generic/session cancellation; error isolation | 36 captured SDK cases pass; every selected Linux child case is mapped to fresh native/helper evidence in fixtures/tools/case-coverage.json |
| Process requests | `process.exec` reaches the acknowledged native callback with operation identity; P09 supplies the separately approved native shell service | P10 connects command/session callback routing; no helper-side unrestricted native exec fallback |
| Commands | Duplicate invocation names, async native context requests | Callback-bearing new-session/compact options explicitly unsupported |
| Flags and shortcuts | Defaults/overrides, legacy untyped flag metadata, first flag definition; reserved key conflicts, case folding, later shortcut winner | Desktop dispatch pending |
| UI requests | Select/confirm/input/editor; notifications/status/title/editor text and string-array widgets through acknowledged native callbacks | Terminal input/components/custom UI factories, synchronous theme mutation and render invocation are not yet supported |
| Context | Native-provided immutable snapshots and acknowledged action requests; unknown custom JSON retained | Provider registration/removal getters update synchronously after native acknowledgement; other session getter freshness remains pending |
| Extension AI calls | Complete/stream APIs through native broker; shared endpoint queue, native credentials/HTTP, streamed assistant projections, cancellation and helper-death observation | Ten provider final-message captures and 269 partial-JSON cases match; private native vault credentials are composed; desktop routing and durable inference evidence integration pending |
| Custom providers | Native registry/owner grants; 17 SDK registration steps; versioned stream/OAuth hooks; staged OAuth model validation; bounded reentrant calls in the same helper | Custom streams use shared admission and native validation; sequential nested transport borrows the actual permit; synchronous register/unregister and staged model hooks pass; native vault login/logout/refresh/key and model refresh are composed and tested; desktop routing remains pending |
| Event streams | Assistant stream/result/factory; bounded queue and explicit terminal/error settlement | Six SDK settlement/fallback captures match. SDK environment opt-in watchdogs are not consumed by this helper; native admission deadlines, cancellation and quotas own termination. Both preserved SDK watchdog defaults are disabled; no implicit reasoning idle timeout is added. |
| TUI rendering | Minimal Text/Container/Spacer and style passthrough helpers | Full terminal component/rendering APIs explicitly unsupported; no native terminal renderer claimed |

Unsupported property calls report `UnsupportedAPI` rather than succeeding silently.
The helper catalog reports duplicate tool and flag definitions in `conflicts`,
separately from factory/handler `errors`. Both arrays are native-visible diagnostics.
Command collisions retain both handlers under suffixed invocation names. Native
resource loading orders explicit extension entries before discovered entries;
the first tool/flag definition wins in that order. Catalog reads recompute conflict
diagnostics without appending duplicate errors on every read.
A registered JavaScript tool remains inactive until the native capability selection
activates it. Before-tool and post-tool hooks receive a deny-by-default parent
callback bridge. Native execution rechecks the host/catalog/argument/scope binding;
completed replay reads persisted evidence without invoking JavaScript. A crashed
host leaves an uncertain intent, which is never automatically executed again.

## Transport and bounds

Private stdin/stdout JSON lines use protocol version 1. One parent operation is
active at a time. Requests, results, native callbacks, stream events and cancellation
have distinct packet kinds. Native callbacks bind the active operation, loaded
entry, generation and unique callback ID. Console output goes to the owned stderr
spool. Direct extension writes to protocol stdout can invalidate the transport.

While a native callback is pending, the native owner can invoke a versioned provider
hook in that same process. Invocation IDs bind to the pending callback, and async-local
tags distinguish its nested callbacks from unrelated parent work. Other callbacks are
deferred within the shared backlog bound. Nesting is limited to four levels and 256
invocations per parent operation. Nested hooks do not flush the parent's pending work,
which would wait on their own caller. Ordinary hook failures remain isolated; a stuck
hook still obeys the native operation deadline.

A custom provider holds its endpoint permit until its versioned hook and validated
terminal stream both settle. Nested native requests resolve their own model and
credential grants. Calls to the same endpoint borrow the actual permit exclusively;
other endpoints acquire ordinary scheduler admission. The inner HTTP response does
not release the outer provider's slot. Interrupted inner transport latches a fault,
so catching its error cannot release quarantine or start another transport on that
lease. Tests cover live event delivery, sibling queue exclusion, terminal release
and a provider catching an interrupted child. These are owned loopback fixtures;
no real model runtime is launched.

The helper retains at most 256 provider hook versions. Native acknowledgements specify
which versions remain referenced by current registrations or active API/OAuth bindings.
Replacing settings does not discard an earlier active stream function. Rejected
registrations remove staged versions, and unregister updates the helper only after
native acknowledgement. Post-bind register/unregister wait for that acknowledgement before
returning, so model getters in the same extension callback see the native catalog. An I/O
worker owns both protocol descriptors; the extension thread pumps native responses,
cancellation and synchronous OAuth model hooks while waiting. No speculative model catalog
is published. The SDK-typed synchronous `modifyModels` hook is supported here; a hook
returning a Promise during synchronous registration is explicitly rejected. Async hooks
remain supported during initial binding and ordinary asynchronous provider invocations.
OAuth credentials enter only the explicitly admitted hook invocation.

The native authentication service grants provider names and hook owners independently
of catalog registration. Its private OAuth target binds the current catalog generation,
helper incarnation and retained hook version. Login captures the prior credential
revision before interactive work, then takes the shared vault lease and creates or
replaces only that revision. Refresh holds the shared lease across load, hook execution,
key validation and durable compare/exchange. Failed, malformed or still-expired refresh
results preserve the existing credential; hook error text is sanitized before it reaches
the requester. Existing provider-specific credential fields pass through to the hook.
The registration model hook can read credentials through the same granted native service.
Logout replaces the current record with a marker containing no credential material;
this is not erasure of historical vault versions. A subsequent explicit login can
replace that marker. Independently configured runtime, file and environment sources
retain their precedence, and fallback remains explicitly requested.
These paths are exercised through an owned helper and actual P05 fixture vault.
Native login UI callbacks, private inference credentials and extension API-key
disclosure have separate grants. The desktop supplies the concrete login UI later.

Catalog refresh starts from the admitted baseline and reapplies retained provider
registrations with current granted OAuth records. It increments the catalog
generation while retaining hook identity, invalidating older model/auth targets.
Hook failure preserves the prior native and helper views. After native publication,
failed helper acknowledgement stops that helper so it cannot serve an older model
snapshot. Credential commits are independent: a catalog refresh failure does not
roll back login or logout. Four captured SDK refresh states and an actual shared
vault/helper integration check cover login, repeat refresh and logout.

Frames are at most 1 MiB; each I/O worker queue is bounded to 4 MiB of wire data /
4096 frames (decoded string memory can be larger); callback backlog is 256 frames /
1 MiB. JSON values have depth/node/byte bounds and reject non-finite numbers.
There are at most 64 loaded entries, 256 registrations per category and 64 handlers
per event. The extension thread has a 128 MiB old-generation heap cap; the I/O
worker has separate 32 MiB old / 8 MiB young-generation caps. These are heap limits,
not whole-process RSS limits. Synchronous native waits have a 60-second ceiling and
remain subject to the native operation deadline. Stream queues are bounded to
10,000 events / 16 MiB; native inference projection output is capped at 512 KiB.
Limits reject excess rather than claiming a complete truncated result. Native
operations have a five-minute maximum deadline. No prefill/reasoning idle timer is
added to the preserved model settings.

Partial tool arguments are display projections only. The bounded native port of
`partial-json` preserves the SDK's incremental values and provider-specific raw
argument fields; final messages still use strict P06 parsing. Truncated final calls
retain the malformed marker and cannot gain executable arguments from a preview.
Non-finite partial values are rejected at the typed JSON boundary. The MIT port
notice is in `product/licenses/partial-json-port`; no parser runtime is added.

The owner stops only its own process on timeout, cancellation or protocol failure.
An acknowledged ordinary JS callback error leaves the host available. Restart uses
a fresh host identity and does not replay partially executed callbacks. Diagnostics
remain in the owned spool; canonical recovery metadata is still pending.

## Dependencies and verification

`tools/prepare_extension_host.py` copies and verifies four pinned third-party
package distributions from the immutable executable reference, entirely offline.
`dependencies.json` records every copied file hash, exact version and package
license. No installer or dependency lifecycle script runs. The exact published Jiti archive matches all 15 vendored files. Its published
source commit and package lock identify a conservative 67-component dependency
closure, with 68 retained notice/metadata files. `bundle-notices.json` records
archive integrity and source hashes; `tools/tool_licenses.py` verifies them offline.
The dependency closure deliberately includes dependencies replaced by build
aliases; it does not claim all 67 execute. One package publishes an MIT declaration
without a standalone notice: its original metadata and explicitly labeled
supplemental permission text are retained, with no invented copyright notice.

The `test` directory runs owned fixture code. Source oracle probes in
`tools/probes/tool_extension_{runner,collections,stream,providers}_reference.mjs` capture actual
pinned SDK behavior, independent of this implementation. Rust tests additionally
exercise trust/scope fences, owned process death, durable execution and the native
inference transport. These checks do not constitute completion of the P09 gate.
