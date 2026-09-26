# GitHub UI and workflow implementation — 2026-09-13

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Implemented against the complete `github.md` handoff and the existing dirty working tree. No applicable AGENTS.md was found in the checked repository/parent/project paths. Existing untracked implementation was treated as user work. No reset, clean, stash, commit, app installation, restart, push or deployment was performed. No live GitHub account was used for testing; native HTTP/TLS and desktop tests used owned fixtures.

## Delivered behavior

- GitHub Settings opens on a restrained Overview with account/connection semantics, on-demand quota/reset information, repository and inbox entry points. Repositories load when that section is entered, rather than spending quota on Overview. Sections expose Repositories, Notifications, Variables & secrets, Access and Advanced. Repository tabs expose PRs, Issues, Checks, Code & more, configuration and available Local Git. Existing Workbench, schema discovery, transfers, graph details, inbox and recovery remain accessible.
- The Settings parent is widened only for GitHub. Controls use existing theme/font tokens and responsive layouts. Repository auxiliary tools no longer precede the primary issue/PR lists. Quota errors show structured retry guidance.
- Repository search submits real `repos.search` requests, separately from Your repositories. Refresh and pagination retain the active query and use native next-page metadata, with explicit bounds. New personal-repository and fork forms retain native review.
- Repository configuration loads current data and prepares only edited fields. Description, default branch, reported feature flags, merge options and branch deletion preferences have controls. Visibility and archive changes use a separate deliberate form. Reported permissions are distinguished from unknown authorization.
- Issues and PRs have explicitly repository/type/state-scoped server search, assignee/label/milestone filters and paged label/milestone choices. Search text cannot inject competing repository/type/state qualifiers. Label/assignee replacement starts with displayed current values; replacement/concurrency/removal semantics remain explicit. Milestone creation is available.
- Merge offers only methods reported enabled by the repository, requires an explicit choice, rechecks settings before native preparation and retains the reviewed head SHA. No silent method fallback. Review history is readable and paged.
- Single-line inline review comments select an exact old/new diff anchor and submit through `pulls.review` with `commit_id`, path, side and line. Complete hunk counts and reported additions/deletions must agree. A fresh head/base read precedes preparation. Refresh, file paging or target changes discard the form/draft. No automatic submission or retry.
- Shared branch/tag selectors carry exact SHAs into Code and tag targets. Branch-only selectors feed local fetch/push/clone/pull/checkout and PR head/base/default-branch forms. Local Git still requires its existing exact native plans and filesystem grants. Code can open a selected folder/file at the selected SHA.
- Actions Variables have repository, environment and organization list/create/update/delete forms. They are explicitly plaintext. Organization access can be all/private/selected repositories; updating a value preserves visibility unless the user opts into changing access. Environment names containing spaces/slashes use typed encoded paths.
- Collaborator lists and permission inspection are readable. Invitations/permission changes/removal prepare an exact named-user native review, with organization/team inheritance explained.
- Secret access offers paged repository names/IDs, retaining selections across pages. IDs reset on destination changes. The explicit all/private/selected semantics and deliberate empty selection remain; secret values still enter only through the existing native private dialog.
- Projects accepts an independent explicit organization/user owner and discards late prior-owner replies. Discussion creation loads repository/category IDs, rechecks repository identity and prepares the existing graph mutation. Standalone lightweight tags and reviewed release deletion are available; release defaults/assets/transfers remain intact.
- Successful applicable form mutations refresh their details/lists. Unknown outcomes are not retried. Navigation invalidates obsolete previews and suppresses late search, detail, preparation and publication results.

## Confirmed defects fixed

1. The active `github/browse.rs` Issues projection returned PR rows. It now excludes rows with `pull_request` while preserving pagination from the original API response, including all-PR pages. Generic `issues.list` remains unchanged. Pull/run projections also preserve real next-page metadata.
2. The repository list used displayed row count instead of native next-page information. It now uses the server indicator.
3. `tags.create` admitted generic branch refs. It now validates a real `refs/tags/...` lightweight-tag namespace and Git ref-name restrictions. Native schema help, model schemas and executable examples agree. Generic ref creation remains consequential `api.rest`.
4. Normal merge was hardcoded to squash. It now uses the enabled-method flow above with exact SHA protection.
5. Touched compose/navigation paths now dismiss retained previews and discard stale results; compose fields cannot drift while preparation runs and expired previews are visibly disabled.

## Backend scope and important files

The existing transport, credentials, service authority, agent dispatcher, approval policy and pinned REST snapshot were reused. Eight catalog adapters were added using the existing compiler/schema/service design:

| Operations | Risk |
| --- | --- |
| `variables.repository_list`, `variables.environment_list`, `variables.organization_list` | Read |
| `access.collaborators`, `access.permission` | Read |
| `variables.environment_create`, `variables.environment_update`, `variables.environment_delete` | Consequential |

This makes 108 catalog operations (60 Read, 26 Write, 22 Consequential), separate from the existing 15 local Git operations. Repository/org variable writes and collaborator writes use schema-validated `api.rest` through native preview/confirm. Raw REST GET/GraphQL were not downgraded into read authority. Model schema/example tests cover the new catalog adapters. The fixed Discussion categories query additionally returns the repository node ID. No protocol or full pinned schema regeneration was necessary.

Main UI changes: `GitHubSettings.tsx`, `Settings.tsx`, `redesign.css`, the existing Action/Browser/ItemActions/PullReview/Local/Secrets/Release components, and new `GitHubControls.tsx`, `GitHubRepository.tsx`, `GitHubReviewActions.tsx`, `GitHubInlineReview.tsx`, `GitHubAdministration.tsx`, `GitHubMetadataPicker.tsx`, `GitHubDiscussionCreate.tsx`. Native changes: `github/browse.rs`, `operations.rs`, `model_contract.rs`, `schemas.rs`, `graph_reads.rs`. New UI coverage is in `GitHubImprovements.test.tsx`; existing Settings tests now navigate through Overview.

Exact source hashes and logs: [evidence directory](../../github-improvements-20260913).

## Validation actually run

| Check | Result |
| --- | --- |
| Focused Settings/Browser/ItemActions/PullReview milestone | 20 tests passed |
| Final `npm test -- --run src/GitHub src/bridge.test.ts src/ReviewPanel.test.tsx` with repository Node 24.14.0 | 139 tests passed across 16 files (109 GitHub, 30 shared) |
| `cargo test --locked -p ag1-platform --lib github::` | 92 passed; 0 failed/ignored |
| `cargo test --locked -p ag1-platform --test github_local_contract --test client_local_git_contract` | 15 passed across the two contracts |
| `cargo check --locked -p ag1-desktop` | Passed |
| TypeScript check and renderer production build | Passed; Vite reports large main/editor chunks, with no build error |
| Owned visual fixture | 12 captures at 1600, 760 and 480 px: overview, repositories, items, configuration, variables/access, empty/loading/quota-error states; no page exceptions or horizontal document overflow |

Build targets and renderer output were redirected to `/tmp/ag1-github-target` and `/tmp/ag1-github-desktop-dist` because `/home` had only about 89 MiB available at orientation. The initial fresh-target native run had 21 missing-process-host failures; building `ag1-process-host` in that target fixed the prerequisite, and all 92 native tests then passed.

The broader App-inclusive run under the system Node 26 failed App setup (`localStorage` unavailable). With the repository's existing Node 24 toolchain it reached **168 passed / 1 failed**. The remaining test, `model choice and appearance remain usable while opening a chat is slow`, expects `Opening chat / preparing request…` before it opens Settings; the unchanged App currently uses a different startup flow/copy. This unrelated App code/test was not modified. Its failure log is retained and the full App suite is not claimed as passing.

Visual validation used an isolated headless browser, an in-memory Tauri fixture and a network allowlist limited to the loopback Vite server. The fixture rejects native execution/confirmation and all non-GitHub commands. Twelve PNGs, measured state/calls, exact HTML and the visual script are retained in evidence. This is browser renderer validation, not a packaged WebKit/native-dialog test or live-account verification. The temporary desktop fixture entry was removed after capture. No installed/running AG1 state was changed.

## Explicit remaining boundaries

- Inline review supports one single-line comment per submitted review. Multi-line ranges, multi-file pending-review composition, existing thread resolution/edit/delete, and binary/omitted/truncated patches remain in Advanced/GitHub. A complete draft-review/thread model needs persistent draft ownership, independent file-page anchors, thread identities and invalidation semantics; it was not approximated with unchecked manual positions. The displayed commit can become outdated after preparation; comments remain bound to that reviewed commit, never silently retargeted.
- Configuration displays feature/merge checkboxes only when GitHub returned boolean values. Unknown permissions/settings are not treated as granted. Label/milestone selectors are bounded; manual assignee/reviewer logins/team slugs remain available. Custom organization roles and broader org/team/branch-protection administration remain Advanced workflows.
- A dedicated hosted-file editor, annotated tag-object composer, exhaustive remote inventory/index and unbounded pagination are outside this UI pass. Repository search and independent Projects owner reads retain GitHub's permission/query/connection limits.
- No live writes or private-dialog/package smoke tests were run. Actual account scopes, organization restrictions, workflow billing/publication behavior and live mutations remain unverified.

## Persistent policy, authentication and synchronization decision

Persistent GitHub-specific action policies/repository allowlists/read-only enforcement are deferred as a shared authorization extension, not exposed as nonfunctional settings. Current generic policy remains in `ag1-core/src/tools/policy.rs`: local catalog/schema discovery is allowed, Planning/Headless deny account operations, Interactive/Autonomous ask, and the explicit user-selected bypass suppresses prompts while native fences remain.

A coherent next implementation should:

1. Store account-bound policy records/revisions in the existing configuration/storage ownership model. Resolve Allow/Ask/Never in generic preflight for agent calls, using the normalized compiled operation rather than a renderer label.
2. Apply the same decisions to UI prepare/read and native execution (`Service::prepare_operation`, `bind_operation`, `execute`, and the local Git admission/execute paths). Bind policy revision to retained plans and recheck immediately before effects; changes invalidate pending previews. A renderer-only check would not cover the agent route; a generic-only check would not cover direct desktop reads/actions.
3. Specify raw REST and account-wide GraphQL handling explicitly. An unknown operation or unresolved node target cannot inherit permission from a selected repository. A restrictive allowlist needs verified repository identities for every affected target, or denial of graph/raw calls that cannot be bounded. Transfers, private secret publication and local Git need the same policy coverage.
4. Define policy versus explicit bypass semantics without silently changing the existing mode. Separate denial enforcement from prompt suppression. Define whether read-only allows downloads, clone/fetch or local index writes; do not claim a GitHub policy restricts independent shell tools.
5. Test persistence/revision races, account changes, raw/graph escapes, UI/agent equivalence and in-flight invalidation before offering the settings.

Account switching, Enterprise hosts, OAuth scope management, cache/ETag revalidation, sync/watchers/webhooks, retry scheduling and automatic indexing remain architectural deferrals from the brief. They require credential/account isolation, response ownership, scheduling/cancellation and migration decisions across service and host composition. Existing authentication and user data were preserved.

## Acceptance coverage

Items 1–5, 7, 9–20 are delivered within the UI scope and validation limits above. Item 6 uses real branch/ref selectors with exact SHA plans, while retaining manual entry and native rejection of stale targets. Item 8 is delivered for readable history and safely anchored single-line input, with advanced cases explicitly bounded above. Item 21 is addressed for named secret repositories, independent Projects owners, Discussion creation and tag/release controls. Item 22 is addressed by the shared-policy design/deferral, with no pretend auth/sync/policy settings. No claim is made that all 1,229 upstream schemas have dedicated forms or were live-tested.
