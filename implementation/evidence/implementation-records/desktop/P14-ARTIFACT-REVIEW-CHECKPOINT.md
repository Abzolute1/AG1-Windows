# Artifact and change-review checkpoint record

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Consolidated 2026-09-11 from the retired combined feature plan. This is implementation/evidence history, not an instruction to resume. The current remaining requirements are in [artifact/review closeout](../../../../tasks/01-ARTIFACT-REVIEW.md).

## Implemented behavior

The React/Tauri UI has a collapsible/resizable shared Review panel, narrow overlay/full-width presentation, focus restoration, bounded persisted width, artifact source/raster/download inspection and paged workspace Git diffs. Scope changes discard stale responses. Internal executor and native snapshot custody are diagnostic content, separate from ordinary artifacts.

Native file services retain exact before/after bytes before publication and settle committed/not-committed/uncertain receipts. Receipts bind the immutable assistant journal entry observed at the effect boundary; queries validate active session/workspace/branch, native call identity and folder rights. Imported unbound receipts remain diagnostic. Ordinary model/tool-result JSON cannot create edit or Undo authority.

`files.changes` and `files.change_read` back compact turn cards, selected-turn filtering, numbered retained diffs, before/after source and digest-checked binary-safe downloads. Workspace-only shell/rename/delete changes are not falsely attributed to a turn. Non-file output cards use an active-branch/native-call-filtered catalog.

Single-operation Undo rechecks the exact postimage, active branch/session, grant/activity and cancellation/deadline. Original bytes are staged before displacement, no-replace publication retains recoverable bytes, and durable records prevent automatic replay after a crash. Explicit interrupted-Undo recovery restores the displaced postimage only into an absent target. Multi-file batch Undo, automatic recovery-copy deletion and shell/rename/delete reversal are not implemented.

Diff rendering pages files/rows/source. A later native transport change added an immutable scoped workspace-diff snapshot with 24 KiB byte pages to handle diffs beyond the native response limit. Its contract checks digest fidelity, page stability through external edits, stale-snapshot rejection and sibling-conversation denial.

## Retained native evidence

These are historical, isolated native WebKit runs with owned fixtures. They do not prove a later binary, real inference or physical input.

| Candidate / result | What the run established |
| --- | --- |
| `native-candidate-zrSyRq/review-XJfQBh` | Initial panel/artifact wide/narrow views and original download; 0 prompts/requests |
| `native-candidate-zrSyRq/editor-DLhmZq` | Editor layout, file refresh, unsaved-buffer conflict protection and recovery |
| `native-candidate-o5dy0d/review-7LMTu5` | Hardened panel and actual configured-helper Git changes; dirty bytes preserved; 0 prompts/requests |
| `native-candidate-o5dy0d/editor-sokrlv` | Editor regression; 0 prompts/requests |
| `native-candidate-HUIBkw/changes-szCs81` and `changes-auDIho` | Approved file effect, truthful turn card, retained CRLF original/download, literal postimage, unrelated dirty-file and forged-claim exclusion; 2 prompts/3 scripted requests each |
| `native-candidate-73qnIm/changes-1RfvHs` | Guarded Undo, original/postimage fidelity, retained recovery copy, later-edit refusal; 3 prompts/5 scripted requests |

All paths are under `.local/p14-live-fixes/`, with `result.json` and screenshots in each result directory. The newer non-file output-card addition postdated candidate `73qnIm`; its complete current native visual coverage remains a closeout item.

The final [GitHub acceptance run](../github/P14-GITHUB-FINAL-20260911.md) passed the full 220-test frontend suite and relevant receipt/review regressions: 13 change-receipt contracts, 1 client-receipt contract, 2 crash/Undo contracts, plus client review/folder checks. Its 12 desktop audits establish GitHub workflow coverage, not every non-GitHub artifact flow.

## Failure history to preserve

- Initial panel probe `review-opnmWU` lacked private XDG download configuration. `review-p1P1UE` tried a 640px width below the app's 900px minimum. Corrected fixture `review-XJfQBh` passed without loosening product limits.
- `review-2imL2z` used a private PID namespace with host procfs, causing pinned file-descriptor paths to fail. The fixture gained matching private procfs; product custody was unchanged.
- Initial receipt tests used invalid artifact IDs; corrected to the existing immutable artifact-ID contract and rerun.
- A turn-review narrow screenshot lacked a resize wait; `changes-auDIho` corrected and verified the screenshot evidence.
- Large-diff contract setup initialized isolated authority too late; corrected setup ordering. Helper restrictions required an appropriate outside-sandbox rerun, without changing the product grant model.
- A full frontend run intermittently failed editor reset; the exact test, App suite and subsequent complete suite passed without product/assertion changes. Keep the observation for future relevant verification.
- Five core and two platform frozen comparison mismatches existed at baseline and reproduced unchanged in final GitHub verification. Original evidence remains in `.local/p14-review-goal/`.

Exact original narratives, candidate hashes, commands and superseded checklists are preserved in the [pre-cleanup archive](../../../../.local/plan-doc-cleanup-20260911-wgz0foyz/original-documents.tar.gz) with [hash manifest](../../../../.local/plan-doc-cleanup-20260911-wgz0foyz/manifest.json). No failed-run files, binaries or frozen expectations were deleted during the documentation reorganization.
