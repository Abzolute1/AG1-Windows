# Resumed GitHub checkpoint and requested systemwide build

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user resumed GitHub implementation on 2026-09-10, then explicitly requested a build and systemwide installation of the latest working checkpoint for later testing. Full GitHub integration remains incomplete; the GitHub-only scope and stop-after-completion boundary still apply. The earlier OS-switch pause is historical.

## Implemented since pNRN7f

Four shared typed reads (`projects.field_users`, `projects.field_labels`, `projects.field_pull_requests`, `projects.field_reviewers`) now independently page nested project field values. These bind the item ID, field ID, field name, value type and cursor; renamed, replaced, inaccessible or mismatched fields fail closed. GitHub's first-matching-field-by-name lookup is checked against the requested field ID before display. Item field pages now expose the field metadata needed to open these reads. UI cursors reset across connection changes and late results remain discarded.

Project organization/user lists and discussion lists now receive the same native target, bounded page and forward-cursor validation as detail reads. Queries request the owner/repository identities required for those checks.

`tools/probes/p14_github_graphql_schema_check.py` validates the fixed source templates against the downloaded official [public schema](https://docs.github.com/en/graphql/overview/public-schema), using isolated `graphql-core==3.2.6`. No product dependency was added. The schema is retained only under `.local/p14-github-resume-20260910`, SHA-256 `4d75bf4a20c06623f6bce580ffc5bdad52a10bbafd09ff481b2b5a98a8adffbf`. All 22 source query/mutation literals validate (this count includes four older fallback literals superseded by shared reads). This establishes static schema compatibility, not live account permissions or mutation effects.

## Verification

- Typecheck, **177 frontend tests / 23 files**, normal renderer build, and **47 GitHub unit tests** pass.
- **33 native contracts** pass: 14 local Git; 13 receipt/Undo; 2 abrupt Undo; one each client local Git, folder access, change review and receipt query.
- Native diagnostic desktop SHA: `6f5384f35f89d325c82219e5b73c14613b8b27e40fef5c958401f371a22df326`; renderer `index-Dwc4BY1l.js`.
- `native-candidate-c0u8xwoh/github-graph-0xn81M`: 29 owned read-only API calls, no prompts/provider calls/writes. New users/reviewers narrow screenshots inspected.
- Same candidate: baseline `github-Vij3ZW` (4 prompts / 8 scripted requests / 11 API calls / 3 fixture writes); recovery `github-recovery-2otYUU` (2 / 4 / 4 / 0); item actions `github-items-IU7brV` (0 / 0 / 27 / 9) all pass.
- The identical binary in shorter `native-candidate-3TAeUt` passes PR review `github-pr-review-z2dJEz` (1 / 2 / 33 / 0); numbered-patch narrow screenshot inspected.
- The identical desktop in `native-candidate-ehxYNY`, with a copy of the user's currently configured process helper (SHA `f60b7d4bd3e6ff9694335bb4e64653a3e3b148341fe52733af6ef2a8d314f63a`), passes baseline `github-P69yIM` and recovery/staging `github-recovery-CRUgR9`. No helper/configuration adoption is needed for these verified workflows.
- Production binary startup passes on a private display with isolated desktop state, zero prompts/provider requests and no leaked owned children. The screenshot shows the rendered new-chat shell. This is a shell smoke check, not live inference or full production feature acceptance.
- Native license/provenance check passes: 513 dependency sets, 3 native ports and 67 extension-loader components. Renderer packaging audit and native dynamic-library checks pass.

All exact logs and native evidence summaries are in `.local/p14-github-resume-20260910`. Failed harness/setup attempts are retained: the first public schema URL was obsolete (official linked `/public/fpt/` URL succeeded); one npm invocation used the repo root instead of the desktop directory; PR audit exceeded Unix socket path length before app launch; the configured-helper audit initially lacked the copied renderer inventory. Each was corrected and its exact check rerun successfully. No product boundary was loosened.

## Sealed production package

Package: `.local/p14-packages/b597a12f98724714822e1d835de194d1/ag1-0.1.0-rc-linux-x86_64.tar.gz`.

- Archive SHA-256: `395ab5a9a8622eb200ac8e522e94578a8539b2821b81daa39cfa5cd9e1406b08`.
- Production desktop SHA-256: `d44c2b19d2e4d48c28b0786c602ec2a3e2c4e0ad429d4d868db254d1c0470c18`.
- Packaged helper SHA-256: `8d71f050c6d97f5522108264c925818d64fe9478ac83fcd3a937a3e75bb5ff45`.
- Source digest: `ea14474e982b43a8e3941ce677a825781e7fe42f1a71d98d56ccc202dcc5200b`.
- Production features only; visual-test broker and owned GitHub fixture are not enabled.

Installation status: **installed and verified** at `/opt/ag1/releases/d8c467c4649742bd8db1a821937b85a2`. The OS desktop authentication completed the `pkexec` request successfully. All 1,140 payload files match the manifest in bytes, size and mode; installed desktop SHA matches the production package. The installer receipt records the exact archive and prior release, and no pending install remains. Verification is retained in `.local/p14-github-resume-20260910/installation-verified.json`. The initial unattended sudo refusal made no changes. No password was needed through the assistant execution channel. Launch using the application menu or `/usr/local/bin/ag1-desktop`.

Prior current release: `releases/a459017613ba4372a84f2dafece16dc8`, desktop SHA `ea8bb4a870f9acbb62a636543ccaa6a14681b1804acfc8887e8a3b0b84f8e92f`. The versioned installer retains it. The protected release process host, model profiles, runtime capture and native host configuration remain unchanged; only the application installation is newly authorized to change.

Remaining GitHub work still includes richer graph mutation forms and semantic/permission fixtures, binary transfers, private secret entry, workflow/release/inbox detail, writable linked metadata, aggregate quotas, and final full matrix verification. No MCP or other phase has started.

## Projects and Discussions action checkpoint — installed

The user now explicitly requests a build and systemwide installation after every major verified GitHub checkpoint. This supersedes the historical no-install constraints; runtime/model configuration remains protected. Administrator credentials are not retained in project files or persistent memory.

Added dedicated forms for project edits/state, linked items/drafts, field values/clear, archive/unarchive; discussion edits/state, comments/replies, comment edits and answer/unanswer. Fifteen typed graph mutation responses now validate returned target IDs and relevant values/state; errors or mismatched/partial replies are uncertain, with no replay. Inputs are bounded, account-wide node targets are explicit, and every change uses the same one-use native confirmation as model tools. Eleven operations are newly catalogued; four existing mutations gain semantic reply checks. Graph display rejects malformed visible scalar types.

Verification: typecheck; **196 frontend tests / 24 files**; **50 GitHub unit tests**; static official GraphQL schema validation; native candidate `3e1oet` passes graph actions `graph-edit-tW7JjC` (35 calls, 14 confirmed fixture writes), graph reads `github-graph-Tmwatt` (29 reads), and shared UI/model baseline `github-dljSO9` (4 prompts, 8 scripted provider requests, 11 API calls, 3 writes). Narrow project-edit and answer previews were visually inspected after collapsing full query source. No live GitHub or inference. Initial form-test invoke-name, socket-length setup and Rust mutex-reference mistakes were corrected and exact checks rerun; previous failed evidence remains retained.

Production package `58d5784959c345dca2b2f7b958e61bbb`, archive SHA `1e73f698be2855a3da10eaf39f81041a7da04237353a9475f2b37159571a494a`, source digest `72f770d03c2d6bc6ac706fdabe4c17331355e14fc8b321a89d37dec5988569be`. Packaging, licenses and isolated production startup pass. Installed and verified at `/opt/ag1/releases/b0b1490b737c459ea008ad99e5e39449`, desktop SHA `5ff2a6befd296de6f1dd791a081da6ac7463b0c410951c6ff551d17a5e417ce9`. All 1,140 manifest payloads and four protected runtime/configuration hashes verified. Prior release `d8c467c4649742bd8db1a821937b85a2` retained. Evidence: `.local/p14-github-resume-20260910/actions-*`.

Download work started after this sealed package; it is not included in this installed checkpoint. Binary transfer/upload, private entry, richer Actions/releases/inbox, writable linked metadata/quotas and the final acceptance matrix remain. Full GitHub integration is still incomplete.
