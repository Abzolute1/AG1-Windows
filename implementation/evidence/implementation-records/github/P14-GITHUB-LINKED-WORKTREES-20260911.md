# GitHub linked-worktree checkpoint — installed

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

GitHub-only scope remains active. The user authorizes builds/systemwide installs at each major verified checkpoint and a short remaining-work/effort summary. Full GitHub remains incomplete; finish and report, then stop before any other phase.

## Implemented

Local Git now resolves a native metadata binding once and retains it across preview/confirmation. Standard Git-created linked worktrees require an explicit Chat folders grant covering the shared repository metadata. Read-only grants cannot authorize a write. The exact worktree slot, common directory, reciprocal `.git` / `commondir` / `gitdir` pointers, root/ancestor identities, grant revision and optional worktree configuration are rechecked. Pointer or directory replacement, revoked rights and changed configuration invalidate the admission. UI and model approval summaries show both metadata destinations.

Shared index/HEAD and common refs/config/object operations use explicit metadata accessors. Ordinary workspace file access never follows a virtual `.git` alias. The namespace preserves the slot beneath its shared metadata directory. A standalone owned reproduction showed installed Git 2.55.0 reference lookup fails when the slot is mounted elsewhere despite `GIT_COMMON_DIR`; preserving topology fixes it. The shared read-only namespace receives the same correction.

Native retained-inode publication can now move an approved metadata preimage to a separately bound worktree recovery root. Both roots are pinned/claimed, collision/race checks preserve later writers, and cross-filesystem publication is refused before effects. New private journal records bind metadata identity as well as the working root, including after restart. Legacy in-folder records remain recognized; they cannot authorize linked metadata. Standard relative `commondir` (`../..`) and same-filesystem metadata/working files are the supported writable layout. Submodule/separate-git-dir and nonstandard/absolute commondir layouts remain explicitly unavailable. No grant to an unselected sibling working tree is implied.

References: [Git repository layout](https://git-scm.com/docs/gitrepository-layout), [Git worktree](https://git-scm.com/docs/git-worktree). These identify shared versus per-worktree metadata; exact sandbox compatibility was tested locally.

## Verification

- **66 GitHub unit tests** pass. New tests exercise missing/read-only/writable grants; real staging/unstaging/commit/branch/checkout; main worktree preservation; late grant/pointer/config/directory/cancellation changes; native interrupted recovery/reopen and changed-pointer refusal. The existing actual owned HTTPS fetch/push fixture now also exercises explicitly granted linked metadata while preserving both indices and config.
- **14 local Git + 1 client local-Git contract**, **9 native publication tests**, and **2 Git/worktree read contracts** pass. Publication adds exact cross-root displaced-inode retention, existing backup collision and later writer checks.
- Existing source-layout replacement contract now expects Conflict from its already-bound changed directory, rather than the old generic unsupported-layout error. Its rejection assertion is retained; no frozen comparison changed.
- Frontend local Git **11 tests** and renderer typecheck/build pass; prior full frontend count is 220 (not rerun in full at this checkpoint).
- Diagnostic candidate `native-candidate-QtMOzt/linked-UPIAo3/result.json` passes actual desktop missing/read-only grant refusals, explicit writable grant, UI restore and model finish after owned interrupted checkout, source/link/deletion staging, shared metadata summaries and preservation of main worktree index/HEAD/files. Two prompts / four scripted provider requests. Narrow UI/model screenshots inspected.
- Same candidate baseline `github-mzRQct` passes. No real GitHub mutation or paid CI, no live inference, no production-state test adoption.
- Retained failures: initial linked metadata tests found the namespace topology bug above, reproduced independently and fixed before successful exact/full reruns. Initial local contract had the obsolete error-class expectation described above. Evidence remains under `.local/p14-github-resume-20260910/linked-*`.

## Package

Production package `.local/p14-packages/32b5effd9c174b168b6d47f7e073324c/ag1-0.1.0-rc-linux-x86_64.tar.gz`; archive SHA `e5d813d99ec785533310c087b68beebf9ac60803216044385a7d914834d4cfd2`; source digest `5953a27a8706adbb66487994cdec22aaa9a7bb011aeac55dcbc541505b2fb0f9`. Production bundle/license/loader checks pass. Isolated production startup passes, screenshot inspected, zero provider traffic and no leaked owned children.

Installed `/opt/ag1/releases/6923ff53b2f74f0fb8c0a8b5c7e560e1`; desktop SHA `7f75834ceee429e1ac5f96301b975740d93684702504945b2ae12c2283b392bb`. All 1,142 payload files match bytes/size/mode, archive receipt matches, no pending installer state. Previous workflow checkpoint retained; all four protected helper/runtime/configuration hashes unchanged. Report: `linked-installation-verified.json`. Authentication used private stdin with no stored password.

Remaining: aggregate repository/private-journal storage limits and final GitHub permission/cancellation/uncertainty/recovery/regression matrix. Several hours remain; quota enforcement and final failures are the remaining uncertainty.
