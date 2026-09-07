# Tool execution contracts — P09 in progress

These APIs are native tool foundations. The TS worker remains the desktop's execution authority. P09 has not passed its gate. File/search/Git/shell/job/web adapters, package operations, resource loading and the optional extension broker have native contracts. Check/visual targets now have a capability and durable dispatch boundary; P11 provides their concrete implementations. Remaining SDK compatibility, fixture coverage, integration and final measurements are still being completed. No live model, credentialed service or GUI was used to establish the contracts below.

## Call and approval boundary

`tools::registry::Registry` holds the 27 pinned SDK definitions separately from their TUI renderers. Definitions begin inactive. The host must explicitly activate a subset of capabilities that actually exists. Extension registration cannot replace a built-in name. Arguments are bounded, prepared with the legacy edit normalization, validated and coerced before producing a private-field `ValidatedCall`.

`tools::executor::prepare` runs a before hook. Replacement arguments are prepared and validated again. The caller then captures any target revision and runs `Authorizer::preflight`; a hook does not receive an authorization ticket or a parent storage handle. The current native fixture driver executes this full sequence.

An `Authorizer` requires a host incarnation identifier. The parent must allocate a fresh ID on restart and maintain its own current epoch, session revision and policy revision. Client-provided copies are not current state. An approval binds operation, call ID/arguments, scope, registry/policy revision, target revision and expiry. A one-use `AuthorizedCall` is consumed by the adapter. A changed registry or file revision, expired/forged approval, or mismatched operation fails before publication. Cancellation removes pending approval; it does not imply an executed shell had no effects.

The policy matrix follows the pinned modes, planning shell checks and headless selection. This policy is a guardrail, not an OS sandbox. In particular, the reference shell allowlist cannot establish that arbitrary shell syntax has no side effects. Physical file containment is enforced independently by P03 directory descriptors. `append` and `editApply` retain the reference's unmanaged-tool policy classification; enabling their autonomous use requires explicit host policy configuration.

The supported schema vocabulary covers every captured built-in definition. Unknown validation keywords are explicitly unavailable; this is not a complete general AJV implementation. Native validation always runs, including environments where the old validator's CSP fallback skipped it. Schema failures expose a bounded validation error, without echoing complete arguments.

## Durable lifecycle and projections

`tools::executor::execute` validates the ticket and writes an acknowledged P05 metadata intent before calling an adapter. The adapter consumes the ticket and returns an effect classification separately from its result. The exact native result is retained before post hooks. A replacement post-hook result is retained separately and its details are placed under `extensionDetails`; it cannot manufacture native inspection/task/runtime evidence in the trusted fields.

The core then stores the projected request representation and acknowledges its terminal metadata record before emitting `end`. Events have stable operation-derived IDs. A repeated call replays its durable result without another effect or another event. A recovered intent is uncertain and must never be automatically executed again. The parent can use `replay` before preparing an edit against a file the original operation already changed. Malformed, truncated and policy-rejected calls have durable rejected results and no execution intent. These operations do not append the parent journal: P10 must append the receipt once under its own revision/epoch fence.

B01 is corrected by one aggregate UTF-16 text budget across blocks, including framing. Images remain ordered and have separate count/encoded-byte limits. Every original result container is retained, including unknown details and images. Shortened output includes the exact-artifact marker and a real requester-authorized P05 cursor. Explicit built-in artifact page reads are exempt from recursive externalization. Post-hook output and the original native output have distinct artifacts linked through provenance; “original” in a post-hook projection means the complete input to that projection.

Small acknowledgement and summary blocks remain in place around individually externalized text; multiple large previews share the aggregate budget including every frame. If small blocks or framing would exceed that budget, text is captured together and represented by one bounded preview. Normal text thresholds remain 8 KiB for write, 56 KiB for read/edit/diff and 24 KiB for other tools. Write previews retain at most 1,000 head / 500 tail bytes before framing; other previews retain 6,000 / 2,000, with valid UTF-8 boundaries. Manageable code and small results remain resident. P05 still captures their original result containers for custody even when no separate text artifact is created; this intentionally differs from calling the SDK externalization helper alone.

A native `ProducerArtifact` grant can reuse existing complete producer text. P05 verifies the requesting origin, immutable record, tool and call identity, artifact kind and payload; the preview reads bounded head/tail pages. Shell/job dispatch can obtain this grant from native adapter output, but JSON metadata alone cannot grant it. A successful post-hook replacement clears the producer grant, so its own content receives distinct evidence. The retained shell spool's completeness remains explicit in native result metadata; exact retained bytes are not a claim that output beyond a spool limit was captured.

The P05 adapter verifies origin access and immutable payload hashes during replay. Its full JSON load is an internal bounded host operation; renderer/model paging continues to use authenticated cursors. Failure before intent acknowledgment prevents execution. Failure after execution but before acknowledgment retains uncertainty or an already committed terminal record, which recovery reads without repeating the effect.

## Native file operations

`ag1-platform::file_tools::FileService` owns a registered workspace. Preparation captures bytes and computes bounded diffs without creating parents or changing files. Ordinary dispatch uses `prepare_queued` to take the bounded FIFO per-file turn before capturing bytes, so a following edit sees the preceding committed edit. The private proposal retains that turn through approval and execution; rejecting or dropping the proposal releases it without a write. It holds no workspace/Git claim while awaiting approval. Explicit optimistic proposals can still use `prepare` and must conflict if their captured bytes change. Execution retains the queued turn (or acquires it for an optimistic proposal), consumes authorization, checks source identity again and calls P03 atomic replacement or absent-file creation. Reads run independently of the mutation queues. P03's workspace/Git claim still serializes publication with cooperative workspace/Git operations. The descriptor and revision checks detect symlinks, replaced roots/parents and stale captures; they do not claim a kernel compare-and-swap against every possible uncooperative external writer at the final rename instant.

Standard `edit` uses simultaneous unique, non-overlapping matches against the original normalized text; fuzzy NFKC matching is opt-in. BOM and the reference's detected line ending are restored. `write` preserves the supplied text exactly and may create parents only after authorization. Existing modes are preserved, while set-ID/sticky copying is refused. `append` preserves the reference separator and no-parent-creation behavior. `editApply` preserves ordered first-match replacements, including empty-old-text insertion. A failed later replacement reports its in-memory match count but publishes no bytes. Its `verified` field confirms application, not tests or task completion.

D03 is corrected: all four mutation variants use the same atomic/CAS publication primitives. Creation uses no-replace rename so a competing creator is not overwritten. Cancellation checks occur before publication; cancellation after a successful publication cannot undo it or relabel it as “no effect.” Created parent directories may remain after an interrupted creation. A successful edit preview retains a session/epoch/path revision fence. A later apply against changed bytes conflicts even if the old substring still exists. The preview registry is bounded and does not silently evict a stale fence.

An additional confirmed preview defect was discovered during P09 (separate from the existing D01–D16 research IDs). The reference `oldConsumedBefore`/`newConsumedBefore` count only previous changed lines. For a replacement on line 8, the captured SDK emits a hunk starting at line 1 with unrelated context. Native previews use actual old/new offsets, merge overlapping context windows and produce hunks that can be applied back to the original text to obtain the proposed result. Two of the 39 captured file cases intentionally differ for this reason. Empty `oldText` in `editPreview` would loop forever in the SDK; native preview rejects it before matching. Neither defect is used as an excuse to alter normal edit/apply matching.

Text read windows reproduce the SDK's default stream-decoder BOM handling, UTF-8 byte truncation, continuation notices, progress and byte-derived inspection references. Line arguments must be bounded integers; zero/negative/fractional limits are rejected instead of producing a non-advancing continuation. Ordinary file snapshots are capped at 8 MiB. Text reads above that limit use a bounded streaming window up to 64 MiB, with full byte-derived revision checks before returning inspection evidence. Directory listings use pinned ICU collation, include dotfiles and preserve the SDK's entry/byte notices. Symlink targets are not followed by native directory or mutation capabilities; listing a link does not grant permission to its target.

Explicitly loaded skill directories may receive host-owned `FileService::set_read_roots` capabilities (at most 64 registered roots). These grants permit only `read`; writes, previews, listing and search retain workspace containment. Replacing the grant set uses a revision check and invalidates pending external reads. Prepared reads bind the resource root and byte revision, and execution rechecks both. Shared read guards cover external execution without serializing ordinary workspace operations. External resource results do not claim workspace inspection evidence; their arguments and results retain the normal P05 artifact custody. Native P03 reads reject symlinks, including links within a granted resource directory. Four actual SDK cases and native revocation, freshness, scope, cancellation and escape tests cover this boundary.

PNG/JPEG/GIF/WebP attachments use native Rust codecs, with no Node or browser dependency. Originals already within the default size/dimension limits are returned unchanged. Resizing follows the reference's orientation, dimension calculation, Lanczos3 filter and ordered PNG/JPEG quality choices. Encoded PNG/JPEG bytes are not claimed to be identical across codec versions; resized dimensions, coordinate notes and pixel error are checked against the actual SDK. Decoder pixel/allocation limits and one concurrent decoder bound the native host's work. Eight additional SDK cases cover small/large WebP, EXIF rotation during resizing and oversized encoded input. Native tests also exercise valid-CRC dimension bombs, decoder exclusion and cancellation between bounded codec/resample stages. Individual codec calls remain synchronous. `FileService::prepare_with_control` propagates native cancellation through image stages and streamed read chunks; callers use the native blocking worker.

## Check and visual target boundary

`tools::targets::register_visual_tools` atomically adds the three pinned schemas
(`screenshot_url`, `interact_then_screenshot`, `verify_ui`), initially inactive.
The ordinary `check` schema stays in the baseline catalog. `Binding` identifies the
native target/workspace/revision, actual check/capture/interaction capabilities and
vision access. Direct screenshot tools require caller vision; `verify_ui` may use
an admitted vision judge. Missing target capabilities produce an unavailable result.
Schema presence does not make a browser, compiler or judge available.

`ag1-platform::target_tools::Service` captures a native `Port::Plan` before approval.
The concrete port must resolve URL/file/toolchain grants and source/runtime revisions
without launching a process, navigating or requesting inference. The prepared stamp
binds the service incarnation and target revision. Native review metadata accompanies
the request, with form fill values redacted from its display. The executor retains
original arguments under the existing private artifact ACL.

Execution requires the P05 durable executor, rechecks authorization and the target
before entering the port, and never treats compile success as visual proof. Completed
receipts replay without contacting an unavailable target. Tests use owned fixture
adapters and real P05 storage; no concrete browser/compiler implementation or desktop
visual check is claimed in P09. P11 supplies those implementations through this port.
Visual requests have explicit limits: 256 actions, five-minute total waits/timeouts,
16,384-pixel integer viewport dimensions and bounded selectors/form values. P11 must
also enforce capture pixel/byte budgets, per-navigation URL policy and cancellation.

## Executable fixture host

`cargo run --manifest-path product/Cargo.toml -p ag1-platform --example tool-driver -- <workspace> <private-store> <requests.json> <new-results.jsonl>` runs the currently delivered file/list adapters through schema preparation, hooks, policy, one-use approvals, P05 intent/result storage and replay. It requires an isolated workspace marker containing `P09 isolated tool fixture\n`. It does not start models or desktop processes. Explicit approval answers come from fixture records; this is a deterministic test host, not a replacement user approval UI.

Do not count this document or the fixture host as phase completion. The remaining selected P09 adapters, extension compatibility surface, failure cases and performance gate must pass first.

## Web result presentation

`tools::web_presentation::render` provides bounded heading, input, summary and body
text independently of tool definitions and execution. It ports compact/expanded
search rows, source labels, fetch facts/previews, coverage slots and placeholder
reasons from the pinned `ToolExecutionComponent`. Four original renderer tests run
against the actual SDK component in memory; native tests compare its exact summary
and body fields in both expansion states. Input structure/output size are bounded
and terminal control sequences are removed. UTF-16 preview limits retain valid Rust
Unicode strings if a boundary crosses a surrogate pair.

P12 clients supply terminal framing, styling and expansion interaction around this
presentation. These P09 tests establish display text semantics; they are not a
claim that the native TUI or desktop layout has received its P12 visual checks.

## Active tool catalog and prompts

`Registry::activate_defaults` intersects the pinned default tool names and admitted
custom/visual definitions with host-supplied capabilities. Registration alone
leaves a tool inactive. `public_tools` exposes bounded descriptive provenance;
built-in/visual provenance is synthesized from native origin, while `register_sdk`
stamps host-admitted custom tools with SDK provenance. Labels never grant execution.

`Registry::build_prompt` composes P07's prompt from current active definitions,
normalizes explicit snippets or falls back to the description, and includes active
extension guidelines. It leaves captured base resource instructions unchanged.
P10 calls it after registry changes. Dynamic helper tests materialize an inline
factory in an owned hash-checked entry while retaining its supplied `<inline:1>`
display provenance; the real entry path still identifies execution and trust.
