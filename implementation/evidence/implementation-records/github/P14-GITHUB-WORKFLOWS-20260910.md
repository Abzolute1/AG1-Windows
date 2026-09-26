# GitHub workflow, release and inbox checkpoint — installed

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

GitHub-only continuation remains authorized. Build/install each major verified checkpoint, report remaining work and effort, finish the full GitHub matrix and then stop. Full integration remains incomplete.

## Implemented

Native workflow, run and job details now retain exact IDs through independent pagination, show bounded literal steps/status/branch/SHA/attempt metadata and refresh after requests. Dispatch accepts a branch/tag and up to 25 scalar inputs; dispatch/rerun/cancel require deliberate consequence acknowledgement and native account/scope-bound one-use approval. A successful request means accepted, not a completed workflow. Typed responses validate target IDs and documented statuses, including API 2026-03-10 dispatch HTTP 200 with run ID/URLs, rerun 201 and cancel 202.

Release forms create drafts by default and edit an explicitly loaded release. Tag, target, name, notes and draft/prerelease status appear in the exact preview; publishing is consequential. Bounded Markdown preview and full paged source complement existing exact-byte asset uploads/downloads. Reply validation requires the requested visible fields and release identity.

The compact inbox supports account or selected repository scope, unread/all and participating filters, bounded independent pages, exact thread read and deliberate all-read with a captured cutoff. Account operations cannot accidentally inherit the selected repository. HTTP 202 indicates background processing; explicit settled refresh determines current read state. Subject links are reconstructed only from fixed GitHub repository/issue/pull identities. Native detail panes fetch lazily when opened. Shared presentation enforces expiry, stale-preview disposal, cancellation and no replay of uncertain writes.

References: [workflow API](https://docs.github.com/en/rest/actions/workflows?apiVersion=2026-03-10), [run API](https://docs.github.com/en/rest/actions/workflow-runs?apiVersion=2026-03-10), [release API](https://docs.github.com/en/rest/releases/releases?apiVersion=2026-03-10), [notification API](https://docs.github.com/en/rest/activity/notifications?apiVersion=2026-03-10).

## Verification

- Full frontend: **220 tests / 28 files** pass; final targeted workflow/browser 10 pass after bounded text/input guards. Production renderer typecheck/build passes after final styles.
- **63 GitHub unit tests** pass after final native guards. New matrix tests cover all aliases, pinned schemas, malformed targets/status/fields, dispatch input bounds and one-use uncertain mutation behavior.
- Final diagnostic candidate `native-candidate-BQJEUY/flows-SXsSJj/result.json` passes: seven UI writes plus one model dispatch, exact workflow run/job page identities, draft creation/edit refresh, scoped inbox paging, exact thread read, account cutoff and settled second-page read state. One prompt / two scripted requests. Narrow workflow, release and settled inbox screenshots inspected.
- Same candidate: baseline `github-4elOjI`, download `download-4kDvMY`, secret GTK/model `secret-NAcxQG` regressions pass.
- Retained failed harness run `aBq8zZ/flows-f5gtMh` incorrectly treated collapsed form controls as visible and selected create instead of edit in the owned fixture. Visibility detection and exact operation assertions corrected; exact reruns passed. No product authority relaxed.
- Production package/license/loader audit and isolated startup pass, screenshot inspected, zero provider traffic and no leaked owned children. All writes used owned fixtures; no real GitHub or paid CI changes, no live inference. Frozen oracles unchanged.

Evidence: `.local/p14-github-resume-20260910/workflow-*` and the native directories above.

## Installed identity

Package `.local/p14-packages/3887202566d943daba5eeeb7f0edf03b/ag1-0.1.0-rc-linux-x86_64.tar.gz`; archive SHA `b9b4977246f337b0abe2a59a59bded6319a1881b8e8c9d424b459b9372f4f4f9`; source digest `1cb2f6c24389f38939037c0ccbaefffa9fce29e4e7f41b49e055feb67e74e998`.

Installed `/opt/ag1/releases/68fc1a13ad24467ea0f30c8709e1aa43`; desktop SHA `00e0a5a236f2d9d2a9c9684ebd496cff3f343cdbeddcf9539f569ec9a702603b`. All 1,142 payload files verified for bytes/size/mode; archive receipt matches; no pending installer state. Prior secret release retained and all four protected helper/runtime/configuration hashes unchanged. Authentication used private stdin; no password stored.

Remaining: writable linked-worktree metadata authority and aggregate repository/private-journal storage limits; final full permission/cancellation/uncertainty/recovery and native regression matrix. Two substantial areas remain, several hours rather than another 30 minutes; local Git authority/recovery is the main uncertainty. Next: explicit metadata grants, shared local mutations and retained recovery identity.
