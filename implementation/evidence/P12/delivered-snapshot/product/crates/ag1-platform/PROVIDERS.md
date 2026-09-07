# Provider and preserved runtime boundary (P06)

The Rust implementation is a shadow library until the P13 authority transfer. The
usable desktop still delegates sessions, scheduling and model control to the
original TypeScript worker. Loading this library, discovering models, selecting a
profile, or reading capabilities does not launch or tune a model.

## Request and stream path

`provider::requests::build_payload` converts the pinned AG1 model/context format
without I/O. The ten registered API families have independent wire decoding,
request conversion and authentication coverage. Request history repair operates
on a copy: foreign signatures are removed, missing tool results get explicit
synthetic error results, and journal bytes remain untouched. Unsupported image
input fails visibly instead of silently discarding the image. Input/output
bodies are bounded; incomplete JSON arguments cannot become executable tools.

`request_http::generation_http_request` adds host-only credentials and canonical
URLs. The explicit-header variant supports host configuration overrides. AWS
credentials are signed after final body/header construction using Signature V4.
Rust identifies its transport honestly; Node SDK telemetry and direct-browser
permission headers are not copied. Anthropic OAuth compatibility identity and
Gemini CLI integration headers retain their pinned values. API credentials,
OAuth records and `HttpRequest` have no Debug/Serialize DTO implementations.
Upstream error messages redact known request credentials before event delivery.

`ProviderHttp` uses verified native TLS, exact configured origin allowlists,
no redirects and no ambient proxy inheritance. The composition host resolves
credentials and endpoint configuration explicitly. Transport errors do not expose
credential URLs. Auth/stream HTTP responses have finite byte and time budgets.
There is no automatic replay after an ambiguous transport failure.

`generation::request_generation` consumes one scheduler permit through HTTP EOF
and normalized terminal settlement. The host supplies a durable identity per
request attempt; missing Google tool IDs are scoped to that identity. Declared
tool names restore Anthropic canonical tool names. Usage includes reasoning,
cache read/write and server-selected Responses service-tier prices. Partial
snapshots are owned values. Watchdogs distinguish prefill, progress idle and hard
request deadlines. Failure before network startup releases an unused permit;
interrupted/ambiguous generation quarantines it.

`ag1_core::inference::Scheduler` is shared by root, isolated calls and explicit
inference probes. It admits local1/remote3, FIFO, with 128 waiters and at most 256
configured endpoints. Endpoint IDs are host configuration identities, not
renderer-supplied URLs. Cancellation, permit drop, or an ignored abort cannot
release a local slot on a grace timer. The host must reconcile with real terminal
or verified owned-process exit evidence. A stale/foreign ticket cannot release a
newer generation. The NVIDIA fallback state machine permits configured fallback
before generation on the reference's 403 preflight path; a selected generation
cannot transition back to fallback. Preserved llama.cpp routes never enter that
fallback policy.

Isolated context construction accepts a purpose, system prompt and one prompt,
not a root transcript. It caps output at 16384, uses a distinct session suffix,
and exposes no tools. The isolated driver validates that structure, consumes one
existing permit, and rejects tool calls, empty results and truncation by default.
Its caller disables cache retention and resolves thinking options for the selected
provider. No isolated helper persists or injects its own output.

## Credentials and capabilities

`AuthCoordinator` preserves runtime, secret-file, environment, stored credential
and optional fallback precedence, including the nvidia-nim parent alias.
`StorageCredentialBackend` owns an adapter to the P05 private single-writer store;
clones share an async refresh lease. Blocking durable reads/writes use a bounded
executor. CAS revisions prevent stale replacement, imports require absence, and
unknown credential fields survive refresh and reopening. Host config resolution
is explicit; credential commands require the P09 policy broker. Do not point this
backend at original user files. The private store is not an OS keychain or a claim
of encrypted-at-rest storage.

Anthropic, Codex and Copilot refresh requests/results are checked against the
unchanged SDK. Codex refresh validates the account claim's shape; the service,
not local claim parsing, authenticates the token. Google CLI/Antigravity refresh
is explicitly unavailable because the pinned source contains placeholder client
credentials. Existing valid access tokens and project identities remain usable.
Interactive login dialogs belong to the later host/UI integration, not a hidden
browser launch from this phase.

Core capability inference, template thinking modes and model/runtime cache scope
are pure. Positive vision entries last 24h, negatives 15m, unknowns are not cached.
`capability::probe_introspection` uses bounded read-only LM Studio, Ollama or
llama.cpp probes. A loaded `/props` vision verdict takes precedence without image
generation. An image canary is a separate API requiring scheduler admission and
an explicitly designated endpoint. This phase only executes controlled fixtures.

## Local runtime

`llama::resolve_launch_plan` preserves each configured executable, loader/library
path, argv order, profile environment, KV cache, context tier, MoE offload,
projector and speculation policy. The three backend selections remain distinct.
Profiles and every executable/library/weight identity are inputs, not writable
migration targets. Missing/changed material and unknown flags fail visibly;
there is no `--help` fallback, binary replacement or automatic tuning.

Discovery accepts only complete split GGUF sets and preserves projector-family
selection. Read-only scans cap depth at six and entries at 100000; directory
identity deduplication also prevents symlink cycles. Verification before spawn
rechecks canonical paths, descriptor metadata and SHA-256 material identities.

`RuntimeController` separates external observation from owned startup, readiness,
stop and handoff. `observation` checks HTTP health and actual loaded model/context;
only a bindable literal loopback endpoint can be called vacant. Readiness never
creates an ownership token. The Linux adapter uses the P03 authenticated child
supervisor and kernel process identity, not persisted PID/port adoption. Model
processes are resident until explicit owned shutdown or supervisor-owner loss;
ordinary commands retain their deadlines.

A linear `RuntimeHandoff` retains the original authenticated handle, rechecks
material and process identity, and cannot be cloned or reconstructed from JSON.
Host death closes the supervisor channel and ends that ownership lifetime. P13
must quiesce the TypeScript owner and perform an explicit control transfer/restart;
no Rust shadow service adopts a live TypeScript or foreign model process.

`ag1_core::tuning::propose` is a separately invoked version-1 pure proposal tied to
an operation and expected profile digest. It preserves the reference's MoE math,
including negative half rounding. It neither samples a GPU nor writes a profile.
Applying any real tuning change still requires the user's separate authorization.

## Validation and limits

Run `python3 tools/phase_gate.py P06`. The gate reruns real Rust contracts,
unchanged SDK captures, formatter/Clippy, dependency notices and original runtime
preservation. Completion authority is `implementation/evidence/P06/completion.json`.
The complete stream/request fixtures cover all ten provider child ledger rows;
70 base/history/options requests and the six P01 llama payloads are independent
of the Rust implementation. AWS serialization and signatures use the pinned SDK.

Benchmarks deliver identical bytes in 97-byte chunks with no artificial delay,
and 100 batches of 128 waiters. They separately report scope: TypeScript includes
SDK request setup/Promise scheduling while the Rust domain benchmark isolates
framing/reduction and synchronous admission. Whole-process RSS includes each
benchmark's dependencies. These numbers are not a desktop, GPU or live-service
performance comparison.

Linux native supervision and controlled loopback HTTP are exercised. Real model
quality, TTFT, prefill/decode/VRAM, cloud credential/service canaries and native
Windows/macOS runtime runners remain explicit P13/P14 gates. No desktop launch,
foreign process eviction, GPU generation or change to the original tuned runtime
is required by P06. Dynamic extension providers integrate in P09.
