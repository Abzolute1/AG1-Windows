# Edit failure investigation and correction — 2026-09-09

The investigation used a read-only query of the native journal for `/home/alex/Tests/COD_Z`, session `initial`, beginning with the user's surgical-comment request. Raw evidence is retained privately under `.local/p14-edit-failure/`; no additional changes were made to that project. Installation identity and validation are recorded in [the receipt](desktop-edit-reliability.json).

## What happened

The run made 39 tool calls across 18 model generations. Five comment edits ultimately succeeded across four files, but the path to them was inefficient and included real harness defects.

| Outcome | Calls |
|---|---:|
| editPreview incorrectly blocked by autonomous policy | 5 |
| editApply incorrectly blocked by autonomous policy | 4 |
| edit rejected: mismatched text, overlap, or nonunique target | 12 |
| edit succeeded | 5 |
| read succeeded | 10 |
| shell checks succeeded | 2 |
| git diff failed because the folder was not a Git repository | 1 |

The first nine rejections came from a missing policy classification: native preview/apply tools were exposed to the model but treated as unknown tools under “Allow routine changes.” Basic edit itself worked. Several supplied replacements used nonexistent text or incorrect indentation, included overlapping regions, or matched two locations. In the repeated WeaponSystem attempt, the model supplied 6/8-space indentation where the recorded source used 4/6 spaces. Re-reading whole files did not reliably correct its next arguments.

The last assistant record is explicitly `aborted` / `Cancelled`, after successful changes and verification output. Its zero usage fields overwrote the old UI's displayed latest usage. They do not mean no tokens were consumed.

## Why it took so long

Provider-reported totals for the recorded generations are 261,277 uncached input tokens, 9,063 output tokens and 373,432 cached input tokens. These are repeated request totals, not unique conversation size or a new performance benchmark.

The independent audit found a changing request-local working-state block inserted into an earlier user message, ahead of accumulated assistant/tool rounds. Inspection/source freshness changed that block repeatedly. System prompt text (6,767 characters) and tool definitions remained identical across the retained requests. This demonstrates an avoidable early-prefix mutation; reported cache reuse dropped on several corresponding rounds, with 20–39k uncached input tokens on many later calls.

The retained request artifacts precede the compiler projection, so they do not establish the cause of every cache miss (notably rounds 8 and 15). Provider templates, context reduction and server behavior can also affect reuse. No live before/after model latency measurement was performed.

## Corrections

- Native editPreview is classified as read-only. editApply and append use the same mode/workspace write controls as edit. Related diff and runtimeObservation classifications were audited and corrected. Unknown extensions remain blocked in autonomous mode; built-in origin/registry checks prevent impersonation.
- Planning no longer advertises append/editApply as available mutations.
- Failed preview/apply/append results explicitly set isError. Aborted editApply batches report zero applied changes and publish no partial bytes. Exact edit, preview and apply reject ambiguous targets, including overlapping ASCII/Unicode matches. Successful sequential edits, explicit insertions and existing fuzzy normalization remain covered.
- Native queued exact-edit failures return bounded candidate line ranges and LF-normalized source strings to help recover indentation or uniqueness. Advice does not change matching or authorize an edit. The prompt asks for a small current read, forbids identical retries and clarifies that preview is neither mandatory for ordinary edits nor write approval.
- Request-local state now appears once at the request tail. Earlier messages and tool/result ordering remain unchanged; the suffix is not journaled and does not become new user intent. This removes the demonstrated earlier-message rewrite without promising universal cache hits.
- A zero-usage cancellation no longer erases an earlier measurement within the same turn, and cannot display another turn's usage. Edit-family rows have readable action labels.

## Verification

The native regression runs the real session, HTTP codec, tool dispatcher, approval preflight, file service and durable journal against a temporary project and controlled loopback provider. Under autonomous mode with an approval handler that refuses requests, read → preview → apply → edit succeeds in four tool calls. Separate tests verify that incorrect indentation returns a precise read range without changing the file and that failed apply/preview/append calls are journaled as failures with no partial writes.

Full native session, core/platform tool, context/prefix, provider, policy and frontend test results are retained in the receipt. The independent auditor reviewed origin checks, authorization/revision bindings and failure reporting. Recorded legacy oracles remain unchanged; tests explicitly assert intentional corrections where those oracles captured unsafe or misleading behavior. This is stronger than treating exact legacy parity as proof of correctness.

Tests use controlled model responses. They prove deterministic harness behavior, not that Qwen will never produce another invalid edit or always finish in four calls. No new GPU inference or duration workload was started.

## Rollback

The preceding systemwide installation `/opt/ag1/releases/cc3a4e969a1847218089fe1936d4be63` is retained. Source rollback is guarded against later edits; see [rollback instructions](../../DESKTOP-UI-ROLLBACK.md). Model profiles, the bound process host and accepted runtime capture remain preserved.
