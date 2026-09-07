# Client and extension compatibility

This records the native isolated adapter and retained reference scope. It is not a claim that the legacy TypeScript SDK object or adjacent packages were ported unchanged. P13 owns the ordinary desktop/live-runtime authority transfer. P14 owns release, live quality and long-horizon acceptance.

| Extension feature | Native isolated client behavior |
|---|---|
| Static registrations and extension tools | Captured paths, explicit host trust, P09 registry and P10 preparation/revalidation fences. Changing a registration during a native operation fails the static boundary. |
| Native session/tool/request hooks | Existing P10 behavior, including DenyBridge. UI/provider callbacks are not silently enabled inside those hooks. |
| Explicit commands and shortcuts | Native queued command, cancellation, context fence and durable terminal receipt. No hidden TS parent AgentSession. Reserved client keys cannot be overridden. |
| Flags | Declared string/boolean flags, native validation, helper-session lifetime; no implicit config write. |
| select/confirm/input/editor | Correlated host question, scope and option/type validation, bounded text, cancellation and stale-answer rejection. Closing a drawer does not answer it. |
| notifications/status/title/editor/widget data | Bounded native events with retained extension identity. Generic AG1 rendering; arbitrary TUI or React component functions are not executed. |
| Extension inference | Explicit `allow_inference` host grant; admitted catalog resolves model and credentials, shared P06 scheduler/transport, output custody. A model-supplied endpoint does not replace host routing. |
| Custom providers/OAuth | P09 broker and P06 host APIs remain available with their existing contracts. Dynamic provider registration/rebinding into this P10 native parent is not supported. The client does not invent OAuth UI callbacks or an independent transport. |
| Resource loaders/packages | Native capture, trust, manifest/pin validation; approved install/update/remove and separate project settings CAS. Recreate a session to rebuild its admitted prompt/tool catalog. |
| Custom message renderers/TUI widgets | Registration metadata retained; content shown as typed blocks/original JSON. TUI widget code is reference presentation, not a second runtime or a React execution channel. |
| Arbitrary callbacks, ambient globals or dead exports | Explicit unavailable/UnsupportedAPI boundaries. The dead advertised `./hooks` import is tested against both reference absence and the facade's explicit error. |

All 85 parent rows are accounted for below; machine-readable fields are in `fixtures/clients/dispositions.json`. Native backend success/failure acceptance is retained in each prior phase's immutable evidence. P12 adds adapter, packaged JSONL, SDK, component and native WebKit integration evidence. Runtime/platform limitations are not promoted to successful operations.

| ID | Capability | P12 surface or retained scope | Disposition |
|---|---|---|---|
| A01 | CLI/config/mode bootstrap | Native CLI argument/config and mode adapter | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A02 | SDK construction/resume | Standalone NativeClient/createAgentSession facade and explicit session resume | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A03 | Session replacement | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A04 | Turn/tool loop | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A05 | Steering/follow-ups | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A06 | Cancellation/settlement | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A07 | Ordered concurrent tool batches | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A08 | Event drain/persistence barrier | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A09 | Runtime retry/replay | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A10 | Malformed/XML tool recovery | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A11 | Repetition/failure/stop guards | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A12 | Controlled runtime policy | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A13 | Coding completion gate | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A14 | Per-request authoritative context | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A15 | Deterministic working-set compiler | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A16 | Ingestion externalization | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A17 | Manual/automatic summarization | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A18 | Hygiene/emergency compaction | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A19 | System prompt/resources/addenda | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A20 | Token accounting/overflow | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A21 | Prefix/cache stability | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A22 | Task objective/phase/lists | Task drawer and native task/todo/observation tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A23 | Epistemic claims/provenance | Task drawer and native task/todo/observation tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A24 | Checkpoint verification | Task drawer and native task/todo/observation tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A25 | Todos | Task drawer and native task/todo/observation tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A26 | Plan submission/questions/mode | Plans and questions | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A27 | Runtime/inspection evidence | Task drawer and native task/todo/observation tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A28 | Explicit episodes | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A29 | Episode drafting | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A30 | Episode retrieval/pointers | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A31 | CV feature switches | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A32 | Embeddings/learned rerank/delta protocol/general allocator GC | Absent/planned algorithms remain excluded; existing feature flags and source evidence are retained. | Absent/planned algorithms remain excluded; existing feature flags and source evidence are retained. |
| A33 | Lexical/exact-symbol selection | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A34 | Vague follow-up/changed-file recovery | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A35 | Source capsules/relations | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A36 | Source revision validation | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A37 | Repo map/index/manifests | Files, CAS save, diff and native file/repository tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A38 | Memory store/migration/archive | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A39 | Memory injection/freshness | Context drawer and original-record paging | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A40 | Immutable text/binary artifacts | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A41 | Artifact paging/grep/search | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A42 | Typed evidence cursors | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A43 | Evidence leases | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A44 | Segmented catalog/reconstruction | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A45 | Binary retention/visual rehydration | Artifacts & memory drawer, journal evidence and native evidence tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A46 | Session journal/tree | History tree, fork and complete HTML export | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A47 | Session corruption/migrations/concurrent writes | History tree, fork and complete HTML export | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A48 | Branch/fork/summarize/export | History tree, fork and complete HTML export | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A49 | Auth/settings/config secrets | Native host model configuration, model picker and conflict display | Native host environment bindings; no renderer secrets or implicit user-vault import. |
| A50 | Multi-provider abstraction | Native host model configuration, model picker and conflict display | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A51 | OAuth/credential refresh | Native host model configuration, model picker and conflict display | Existing P06 OAuth/refresh host APIs preserved. This facade uses host-resolved environment bindings and does not claim a login UI. |
| A52 | Local endpoint/model discovery | Native host model configuration, model picker and conflict display | Host-admitted catalog, no unrequested live endpoint probing. |
| A53 | llama.cpp lifecycle | Native host model configuration, model picker and conflict display | P06 owned-runtime controller is preserved; P12 isolated clients do not adopt an external process. Live owner transfer remains P13. |
| A54 | Model profiles/VRAM/KV tuning | Native host model configuration, model picker and conflict display | Raw tuned profile and binary/environment selection are preserved. Tuning is explicitly unavailable in this isolated client binding; no settings effect retunes. |
| A55 | Capability/thinking/vision and switching | Native host model configuration, model picker and conflict display | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A56 | Streaming/watchdogs/fallbacks | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A57 | Inference scheduling | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A58 | Isolated model calls | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A59 | Tool definitions/extension hooks | Resources drawer and native extension/resource adapters | P10 DenyBridge and static registration fences remain; explicit commands have the separately admitted P12 UI/inference route. |
| A60 | Approval/safety/planning policy | Resources drawer and native extension/resource adapters | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A61 | Read/find/grep/list | Files, CAS save, diff and native file/repository tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A62 | Edit/write/per-file queue | Files, CAS save, diff and native file/repository tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A63 | Append/diff/edit preview/apply | Files, CAS save, diff and native file/repository tools | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A64 | Shell execution/output | Runtime pane, owned terminal and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A65 | Background jobs | Runtime pane, owned terminal and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A66 | Sandbox/workspace isolation | Runtime pane, owned terminal and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A67 | Compiler check/diagnostics | Runtime pane, owned terminal and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A68 | Web search/fetch | Native web search/fetch tools and retained artifact evidence | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A69 | Browser/visual verification | Runtime pane, owned terminal and Activity | Optional admitted independent browser and current caller vision; judge-only vision remains unavailable. |
| A70 | Subagent profiles/batches/checkpoints | Chat, queue controls and Activity | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A71 | Skills/templates/packages/extensions | Resources drawer and native extension/resource adapters | Admitted static extensions/resources; dynamic provider rebinding remains unsupported in the native parent as required by the P10 boundary. |
| A72 | Skill creation/update helpers | Resources drawer and native extension/resource adapters | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A73 | RPC/headless protocol | JSONL executable and standalone protocol SDK | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A74 | TUI/interactive commands/settings | React actions, keyboard/theme/focus and source TUI reference | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A75 | Usage/cost/debug/audit/export | Activity, full detail pages and HTML export | Uses the existing acknowledged native backend through the P12 projection/command adapter. |
| A76 | Deterministic replay/quality evaluation | Deterministic quality and replay acceptance remains P14; no new quality claim in P12. | Deterministic quality and replay acceptance remains P14; no new quality claim in P12. |
| A77 | Live long-horizon/bench harnesses | Live long-horizon and matched-model performance acceptance remains P14; no live run in P12. | Live long-horizon and matched-model performance acceptance remains P14; no live run in P12. |
| A78 | Godot native embedding | Runtime pane, owned terminal and Activity | P11 custom-Godot protocol and transaction adapter is retained; runtime pane exposes native target status/check/run. Native embedding remains unavailable and is not represented as a web URL. |
| A79 | Workflow engine | Validate an isolated JSON workflow and steer the pinned textual instruction. The independent DAG runner remains unintegrated reference code. | Validate an isolated JSON workflow and steer the pinned textual instruction. The independent DAG runner remains unintegrated reference code. |
| A80 | MCP client | Preserve the unintegrated stdio MCP client and adapter at the reference pin. No HTTP transport or configured desktop MCP is invented. | Preserve the unintegrated stdio MCP client and adapter at the reference pin. No HTTP transport or configured desktop MCP is invented. |
| A81 | Removed autonomous runner | The removed autonomous runner remains removed. Existing workspace behavior is covered by A66. | The removed autonomous runner remains removed. Existing workspace behavior is covered by A66. |
| A82 | Web UI library | The independent Lit web-ui package remains a TS reference client. It has no drop-in Rust API compatibility claim. | The independent Lit web-ui package remains a TS reference client. It has no drop-in Rust API compatibility claim. |
| A83 | Slack mom integration | The independent Slack mom CLI remains a TS reference client. No Slack messages or live integration are part of P12. | The independent Slack mom CLI remains a TS reference client. No Slack messages or live integration are part of P12. |
| A84 | Pods/GPU administration | The pods/GPU administration CLI remains a TS reference client. No SSH, deployment or vLLM substitution is part of P12. | The pods/GPU administration CLI remains a TS reference client. No SSH, deployment or vLLM substitution is part of P12. |
| A85 | Legacy SDK globals/hook export | The native SDK has explicit UnsupportedAPI errors for ambient globals/stdout takeover and the advertised-but-absent ./hooks target; no new hooks API is claimed. | The native SDK has explicit UnsupportedAPI errors for ambient globals/stdout takeover and the advertised-but-absent ./hooks target; no new hooks API is claimed. |
