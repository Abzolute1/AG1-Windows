# GitHub aggregate storage checkpoint — installed

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

GitHub-only work remains authorized, with a production build/systemwide installation and short remaining-work/effort summary at each major verified checkpoint. Full GitHub acceptance is still pending.

## Implemented

Native local Git measures aggregate metadata and retained working-tree recovery/staging bytes, plus app-private Git journals, index preimages and temporary Git data. It uses pinned no-follow traversal, bounded depth and entry counts, counts link bytes without reading their targets, and never deletes existing records to satisfy a limit. Index preimages now use the shared private Git store; existing active-host legacy storage is also counted.

Admission limits are **4 GiB repository metadata/retained Git data**, **1 GiB private Git storage for new work**, **2 GiB private storage for explicit recovery**, and **100,000 entries per budget**. Current ordinary source files are not subject to pruning or a whole-folder disk quota. The common metadata store and current worktree's retained Git directories are measured together. Native plans show usage and reserve conservative additional bytes/entries before confirmation. Examples: 32 MiB repository/16 MiB private for staging, 128/128 MiB for transitions, 1 GiB repository for fetch, and 1.25 GiB for clone. Large operations may be refused even if their eventual transfer would be smaller than the reservation.

A private no-follow kernel file lease serializes admitted local Git writes across projects/processes; previews do not retain leases. Capacity is rechecked when consuming approval, periodically during owned work and unconditionally after successful completion. Native callbacks check growth without scanning on every individual file publication. Quota/cancellation errors retain prepared-reference abort/uncertain handling. Fetch keeps received objects in a pack (`fetch.unpackLimit=0`) instead of expanding the incoming set into loose files; the existing 256 MiB per-file process limit remains.

These are native admission/runtime budgets, **not kernel filesystem quotas on external writers**. A concurrent external writer can exceed a measured limit between checks. After possible effects, excess is conservatively uncertain and never automatically replayed. Read/status/recovery inspection remain available at capacity. Explicit recovery has a separate allowance; it can itself be refused at the recovery ceiling. No automatic pruning, infinite recovery-space guarantee or cleanup of unknown records is claimed.

## Verification

- **70 GitHub unit tests** pass, including four new quota tests: repository/private/entry growth between review and execution refuses index publication; reads remain available; link targets are not followed; independent lease acquisition is exclusive; cancellation interrupts scans; post-publication external growth remains uncertain with originals retained; actual restore succeeds using the reserved allowance when normal admission is full.
- Actual owned HTTPS fixture verifies kept pack output and normal/linked fetch/push plus existing clone/pull/cancellation/uncertainty/recovery cases.
- Full frontend **220 tests / 28 files** and renderer typecheck/build pass. Affected local/client/review/folder contracts pass (14 local Git and three client contracts).
- Diagnostic candidate `native-candidate-GZFhVc/quota-ZuiqTV/result.json` passes actual native budget preview, sparse-file aggregate growth after review, pre-effect refusal with unchanged index, status at capacity, and linked UI/model interrupted recovery/restore/finish plus exact link/deletion staging. Two prompts/four scripted provider requests; main worktree index/HEAD/files preserved. Narrow refusal and recovery screenshots inspected. The initial passing quota run `quota-cKpqBk` captured a screenshot above the error; the probe now scrolls the error into view and verifies overflow before capture. No product authority changed.
- Same candidate baseline `github-F8IY7n` passes. Owned fixtures only; no live GitHub mutation, paid CI or real inference. Frozen expectations unchanged.

Evidence: `.local/p14-github-resume-20260910/quota-*` and the native directories above.

## Package

Production package `.local/p14-packages/bc43f6b698ad4d0fa1c219ee81384963/ag1-0.1.0-rc-linux-x86_64.tar.gz`; archive SHA `6408e4a1823fdd26ba0b0f1c26d8cb760e2d98fac84f1b09b8bcf37aaecda926`; source digest `49e3cb0fe606ad0a511a35f0dc53f42c6148eeedca438e938dc3257105a88500`. Production bundle/license/loader checks pass. Isolated production startup passes, screenshot inspected, zero provider traffic and no leaked owned children.

Installed `/opt/ag1/releases/ab00eb7e6ee940a2a03bde282f3399c2`; desktop SHA `1f2829fc3ad714b401230007a1e9a23ffbdc8d4437d64bd9d5754d3eace27473`. All 1,142 payload files match bytes/size/mode; archive receipt matches; no pending installer state. Previous linked checkpoint retained and all four protected helper/runtime/configuration hashes unchanged. Report: `quota-installation-verified.json`. Authentication used private stdin; no password was stored.

Remaining: final GitHub capability/permission/cancellation/uncertainty/recovery matrix; broader native regressions, advanced credential-field and local metadata edge review, relevant frozen comparison revalidation and final ledger reconciliation. This phase may still expose required fixes; do not declare GitHub complete yet.
