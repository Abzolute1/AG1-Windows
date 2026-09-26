# Artifact and change-review closeout

Status: planned closeout; implementation already substantially exists. Read the [task index and shared rules](README.md) before resuming. This document does not authorize execution on its own.

## Existing implementation

The shared resizable Review panel, narrow-window layout, artifact inspection, paged workspace diffs, native turn-bound file receipts, compact change/output cards, retained before/after views, and guarded single-operation Undo/recovery are implemented. Do not use the old plan's unchecked boxes as evidence that these features are missing.

The [artifact checkpoint record](../implementation/evidence/implementation-records/desktop/P14-ARTIFACT-REVIEW-CHECKPOINT.md) identifies existing tests and native evidence. The final GitHub run also passed receipt, Undo and review-related contracts, but does not establish a complete current native audit of every non-GitHub output flow.

## Remaining tasks

- [ ] Reconcile the original review requirements against current code and evidence. Record each as implemented/verified, implemented/needs current verification, missing, or explicitly limited. Fix actual gaps without restarting completed receipt or panel work.
- [ ] Verify selected output, chat draft/scroll and unsaved editor buffers through open/close/resize, fullscreen, conversation/workspace changes and reopen. Confirm that stale asynchronous results cannot replace the current selection and focus returns to a stable control when an opener unmounts.
- [ ] Close any remaining artifact presentation gaps: searchable/filterable list, readable type/name, loading/error/empty states, text/source/JSON/diff/raster inspection, copy/download and optional metadata. Keep internal receipt/executor custody under diagnostics.
- [ ] Verify literal handling of untrusted HTML/SVG, immutable identities/digests, lazy bounded reads, exact full-content downloads, missing/malformed/oversized content handling and object-URL cleanup.
- [ ] Verify turn-bound cards, reliable file counts, numbered diffs, source selection, file filtering/navigation and bounded rendering on large changes. Clearly distinguish selected-turn edits from all workspace Git changes, including changes made through shell tools.
- [ ] Confirm failed, no-op, prepared and uncertain effects are not presented as successful edits. Imported/older conversations need useful artifact/workspace fallbacks without manufactured receipts or Undo authority.
- [ ] Review the existing supported effect boundary: exact native create/edit receipts and single-operation Undo. Shell/rename/delete attribution/reversal and multi-file batch Undo are not delivered. Preserve explicit unsupported labels; record any unmet original requirement instead of silently claiming coverage or expanding Undo without a design review.
- [ ] Verify Undo/recovery against current postimage, active session/branch, folder rights and activity. Preserve later user/model edits and recoverable originals; do not substitute broad Git reset, checkout or dirty-buffer discard. If batch behavior is proposed, require full target prevalidation and explicit partial/crash outcome design first.
- [ ] Close evidence gaps for dirty baselines, overlapping turns, links/outside grants, revocation, stale sessions, failed/partial tools, binary/large/missing snapshots, interrupted reads and out-of-order replies. Reuse existing tests and add only meaningful missing coverage.
- [ ] Obtain current native wide/narrow coverage of non-file output cards, turn review, large diff transport, keyboard/focus, images/source/download, resize, errors, Undo/recovery and preserved drafts/buffers. The older output-card addition explicitly postdated its first native Undo candidate.

## Code and verification starting points

- Frontend: `product/apps/desktop/src/ReviewPanel.tsx`, `WorkspaceReview.tsx`, `ArtifactsPane.tsx`, `artifactData.ts`, `App.tsx`, and the turn/output-card components and tests in the same directory.
- Native effect custody: `product/crates/ag1-platform/src/change_receipts.rs`, `file_tools.rs`, and `product/crates/ag1-storage/src/file_changes.rs`.
- Contracts: `change_receipts_contract`, `client_receipt_contract`, `undo_crash_contract`, `client_change_review_contract`, and `client_folder_access_contract` in the platform crate.
- Inspect existing `tools/probes/p14_native_*` review/change/editor drivers before selecting the exact probes. Preserve old result directories and use a new candidate for changed binaries.

## Acceptance

A successful attributable file operation produces a truthful compact card that opens usable retained review. Non-file outputs are easy to inspect. Later work cannot be overwritten by Undo, and historical/unsupported effects remain clearly labelled. Current native evidence covers the final visible behavior; image, folder, editor and GitHub regressions relevant to changed code still pass.

When this task is resumed and reaches a major verified checkpoint, follow the build/install and remaining-work summary rules in the index. Continue to [MCP integration](02-MCP-INTEGRATION.md) only within the user's resumed scope.
