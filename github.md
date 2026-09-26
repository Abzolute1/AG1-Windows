# AG1 GitHub implementation handoff — fresh Astra thread

Prepared from a source audit of `/home/alex/Projects/AG1` on 2026-09-13. All paths below are relative to that repository root. This is a self-contained implementation prompt: no earlier conversation is required.

## 1. Objective

Continue and substantially improve AG1's existing GitHub integration. Turn an already broad backend into a coherent, discoverable, polished and safe desktop GitHub experience. Fix the identified workflow defects, surface existing capabilities through normal controls, reorganize GitHub Settings, and preserve advanced access.

**This is NOT a task to build GitHub integration from scratch. Do not rebuild a capability because its UI is poor.** Extend the shared native service, agent dispatcher, existing components and approval architecture.

The user authorizes implementation in the new thread. The preparation thread changed only this handoff file. Begin useful implementation after proportionate orientation; do not respond with only another audit or a proposal to start later. Work through the implementation phases and acceptance criteria, documenting concrete deferrals where an architectural extension is outside the first pass.

## 2. Critical execution behavior

> **DO NOT OVER-VERIFY EVERY SMALL CHECKPOINT.**
>
> This is a substantial implementation task. Work continuously and efficiently through logically grouped changes. Do not trade correctness for speed. Avoid redundant micro-verification, not testing.

Do not:

- Repeatedly re-read unchanged files or inspect the same implementation without new evidence.
- Rerun identical checks after every tiny edit.
- Stop after every micro-change to re-prove that it worked.
- Perform redundant verification that does not materially increase confidence.
- Spend excessive time validating obvious local edits instead of progressing.
- Launch broad, expensive suites or lengthy endurance/live tests after each small checkpoint.

Use this milestone-based loop:

1. Inspect enough relevant architecture up front to understand the system.
2. Form a coherent implementation plan.
3. Implement a meaningful group of related changes.
4. Run focused tests/checks appropriate to that group.
5. Fix actual failures.
6. Continue to the next group.
7. Perform broader integration/final validation at meaningful boundaries and completion.

Give concise progress updates about completed work, findings and next milestones. Do not invent completion percentages or carry forward historical test claims as current results. Do not stop after the first small improvement while the main authorized work remains.

Do not repeatedly ask for confirmation during normal implementation unless an operation is genuinely destructive, a decision is materially ambiguous, architectural directions conflict, an applicable project instruction explicitly requires authorization, or proceeding would mutate an external/live resource in a way that should not be assumed. Make reasonable routine implementation decisions and continue.

### Preserve the current workspace

- Read applicable repository/project instructions before changing code. No applicable `AGENTS.md` was found during handoff preparation, but check the current tree and parent directories once; instructions may have changed.
- Inspect current Git/worktree state before editing. The audited workspace was heavily modified, with many untracked files, including substantial GitHub implementation. **Do not treat untracked files as disposable.**
- Preserve user work and unrelated changes. Do not reset, clean, stash away, overwrite or broadly rewrite unrelated implementation.
- Work from the current working tree, not just HEAD. An isolated checkout of HEAD would omit existing uncommitted integration work unless explicitly carried forward.
- Keep edits scoped to GitHub and supporting shared infrastructure genuinely needed for it. Do not globally reformat dirty files or regenerate unrelated artifacts.
- Reuse AG1 components, routing, styles, abstractions and native approval mechanisms. Avoid parallel API clients, competing Git implementations or a second permission engine.
- Inspect available disk space before expensive builds; this workspace has previously been storage-constrained. Never clean up user data, recovery records or unrelated build evidence to make space without authorization.

## 3. Verified current state and evidence boundaries

The audit established these source counts:

- **100 GitHub catalog operations:** 55 `Read`, 26 `Write`, 19 `Consequential`.
- **15 local Git operations** routed through the same tool, separately from the 100-entry remote/native-transfer catalog.
- **One agent tool named `github`**, with operation-based dispatch plus local `catalog` and `schema` discovery.
- **1,229 pinned REST endpoint schemas** in `product/crates/ag1-platform/resources/github-requests-2026-03-10.json`.
- The 1,229 schemas are **not** 1,229 individually implemented high-level agent operations. Many are accessible only through the bounded raw API, and some are deliberately blocked or require separate private/binary channels.

These counts describe the audited source and can change during implementation. Confirm them only if relevant; do not repeat the entire audit.

“Implemented” here means code is wired into executable production paths. The source audit inspected existing tests but did not rerun all suites or execute live account writes. Capability availability also depends on GitHub permissions, account authorization, repository configuration and native prerequisites. Do not describe all remote operations as live-verified against the user's account.

This file replaces an older `github.md` delivery report. Historical detail remains in:

- `implementation/evidence/implementation-records/github/P14-GITHUB-CAPABILITIES.md`
- `implementation/evidence/implementation-records/github/P14-GITHUB-FINAL-20260911.md`
- `implementation/evidence/implementation-records/github/GITHUB-MODEL-REPAIR.md`
- `implementation/evidence/implementation-records/github/GITHUB-USABILITY.md`

Those reports contain historical installation/test results and may call the old scope “complete.” They do not supersede this new implementation request or prove the current tree/build is validated. Read them selectively when their evidence is needed, not as a prerequisite to every edit.

### Four categories you must keep separate

| Category | Current examples |
| --- | --- |
| **1. IMPLEMENTED AND ALREADY USABLE** | CLI/browser connection; repository browsing; issue/PR create/edit/comment; PR review/merge; notification inbox; local Git; workflow actions; release create/edit/assets; Projects and Discussion detail actions; private secret entry; API Workbench. Improve these without duplicating them. |
| **2. IMPLEMENTED BUT INCOMPLETE / LIMITED** | GitHub.com-only auth; local Git's deliberate boundaries; squash-only normal merge form; incomplete review UX; page-only repository filtering; bounded/manual pagination and refresh; basic Code browser; quota handling without scheduling. |
| **3. BACKEND EXISTS BUT NORMAL UI IS MISSING OR POOR** | Real repository/issue search controls, repository configuration, review history/inline input, branch/ref selectors, variables, collaborator access, metadata selectors, independent Projects owner, Discussion creation, standalone tag creation and release deletion. Some already have JSON Workbench exposure; do not call them wholly unexposed. |
| **4. ACTUALLY NOT IMPLEMENTED / REQUIRES BACKEND WORK** | AG1 account switching; configurable OAuth scopes; Enterprise hosts; persistent cache/ETag infrastructure; background sync/webhooks/watchers; automatic indexing; GitHub-specific persistent policy/allowlists/read-only restrictions. These are not hidden settings. |

## 4. Existing capabilities and exact operation inventory

The following table lists suffixes under each prefix; for example, `repos.` + `list` means `repos.list`. Preserve these capabilities. `Consequential` is the actual catalog label, not a synonym for deletion.

| Prefix | Read-only operations | Write operations | Consequential operations |
| --- | --- | --- | --- |
| `repos.` | `list`, `search`, `get`, `contents`, `commits`, `branches`, `tags` | — | `create`, `fork`, `update` |
| `issues.` | `list`, `search`, `get`, `comments`, `labels`, `issue_labels`, `milestones` | `create`, `update`, `comment`, `set_labels`, `assignees`, `create_milestone` | — |
| `pulls.` | `list`, `get`, `files`, `reviews`, `checks` | `create`, `update`, `review`, `reviewers` | `merge` |
| `actions.` | `workflow`, `workflow_runs`, `job`, `workflows`, `runs`, `run`, `jobs`, `artifacts`, `download_artifact`, `download_logs`, `download_job_log` | — | `dispatch`, `rerun`, `cancel` |
| `releases.` | `list`, `get`, `assets`, `download_asset` | — | `create`, `update`, `delete`, `upload_asset` |
| `tags.` | — | `create` | — |
| `downloads.` | `read` | — | — |
| `notifications.` | `repository_list`, `list` | `read` | `mark_repository`, `mark_all` |
| `secrets.` | `list`, `get`, `repositories` | — | `set`, `delete`, `set_repositories` |
| `discussions.` | `list`, `categories`, `get`, `replies` | `create`, `comment`, `update`, `close`, `reopen`, `edit_comment`, `answer`, `unanswer` | — |
| `projects.` | `list`, `user_list`, `items`, `get`, `fields`, `item_values`, `field_users`, `field_labels`, `field_pull_requests`, `field_reviewers` | `add_item`, `update_field`, `clear_field`, `add_draft`, `archive`, `unarchive` | `update` |
| `rate_limit.` | `get` | — | — |
| `api.` | — | — | `rest`, `graphql` |

Important semantics:

- `notifications.read` **marks a thread read**; it is a write despite its name.
- `issues.list` uses the repository Issues endpoint; `issues.search` can search issues and PRs. Preserve explicit `is:issue` / `is:pr` scope in normal search views.
- `repos.create` is personal-repository creation through `user/repos`; organization repository creation is raw API territory.
- `pulls.review` supports review event/body/commit and schema-supported inline `comments`. Other review/comment edit/delete/thread operations generally require raw APIs.
- `pulls.checks` reads check runs, not a complete branch-protection/merge-authorization verdict.
- `tags.create` currently creates Git refs; see the correctness finding below.
- `secrets.get` and `secrets.list` expose metadata, never stored values.
- Actions dispatch/rerun may trigger billable runners or deployments. Cancellation is a request, not proof the run stopped.
- Release creation defaults to draft in the normal UI; preserve deliberate publication.
- Project field values include text, number, date, select and iteration data; related people/labels/PR/reviewer collections have separate bounded reads. Do not flatten away nested pagination.

### Local Git operation inventory

Read-only: `git.status`, `git.diff`, `git.branches`, `git.recovery_status`.

Local metadata/index/object writes: `git.stage`, `git.unstage`, `git.commit`, `git.branch_create`.

Network and/or working-copy writes: `git.fetch`, `git.fetch_pr`, `git.push`, `git.clone`, `git.checkout`, `git.pull`, `git.recover`.

- Fetch and PR fetch acquire objects/update retained or remote-tracking refs without changing working files.
- Push is a remote write and can trigger CI. It requires the reviewed expected remote SHA and fast-forward ancestry; zeros represent a new push branch only.
- Clone creates a new relative destination in a granted writable parent; it does not overwrite an existing destination.
- Checkout binds an exact locally available target SHA. Existing branches must match the reviewed commit.
- Pull is a separately reviewed fetch followed by a reviewed fast-forward of the current same-named branch. No implicit divergent merge/rebase/stash.
- Recovery inspects native records, then requires a fresh reviewed finish/restore action. Retained files/branches are preserved; unknown/live/replaced locks and later writers are refused.
- Standard linked worktrees are supported with grants to the common metadata and same-filesystem constraints. Older catalog prose mentioning only in-folder `.git` is not a complete description of the implemented metadata support.
- No dedicated native force push, reset/clean, signing, hooks, filters, automatic submodule or LFS downloads. Generic command tools are a separate execution surface; do not pretend local-Git restrictions prohibit all possible shell Git use.

## 5. Relevant architecture and files

### Execution flow — understand this before adding UI

Normal UI reads go through `api.github(...)` in `bridge.ts`, the Tauri `github_command` router, and the shared native service. Common projections (`repos`, `issues`, `pulls`, `runs`, `item`, `files`, `pull_review`) route to `github/browse.rs`; do not patch only the older similarly named branches in `github.rs`.

Remote changes go through `api_preview` and `api_confirm`. The renderer submits fields to prepare; it does not manufacture execution authority. `api_confirm` consumes the retained native confirmation token, not a newly supplied mutable request body. `api_dismiss` and `api_cancel` already exist.

The agent's `github` tool normalizes the request in `model_contract.rs`; adapters prepare the same native plan, attach native approval details, and execute after generic authorization. Only trusted-host registration gets this adapter. Local Git dispatch uses the shared local plan/host-grant path. Do not add a direct renderer fetch or shell `gh api` escape route.

`api_read` admits only catalogued `Read` operations. **Raw REST GET and raw GraphQL queries are consequential and cannot just be sent to `api_read`.** A normal Variables or collaborator viewer must either use the existing preview/confirm flow or introduce narrow, reviewed native read operations using the existing catalog/service/schema design. Do not globally downgrade raw requests to reads for convenience. Adding a narrowly typed adapter is supporting backend work; the API capability already exists.

Native UI schemas use `schemas.rs`; model schemas/examples/aliases use `model_contract.rs`. Keep both surfaces coherent if catalog operations change. The model expects executable AG1 envelopes, not untranslated upstream OpenAPI IDs.

### Native GitHub implementation

| Path | Role |
| --- | --- |
| `product/crates/ag1-platform/src/github.rs` | GitHub CLI/browser login, connection marker, account lifecycle, URL opening, legacy routes. Some old write routes remain in source but are explicitly rejected before execution. |
| `product/crates/ag1-platform/src/github/operations.rs` | Shared operation catalog, risk classes, request compiler, path/target checks, fixed GraphQL assembly. |
| `product/crates/ag1-platform/src/github/model_contract.rs` | Executable model schemas/examples, catalog filtering, upstream alias normalization and schema-search wrapping. |
| `product/crates/ag1-platform/src/github/schemas.rs` | Pinned schema discovery and REST request validation; credential endpoint restrictions. |
| `product/crates/ag1-platform/resources/github-requests-2026-03-10.json` | Pinned 1,229 REST request schemas. Do not regenerate the whole snapshot for a UI change. |
| `product/crates/ag1-platform/src/github/service.rs` | Shared account-bound prepare/read/execute authority, one-use previews, cancellation, disconnect and binary custody. |
| `product/crates/ag1-platform/src/github/transport.rs` | Native token retrieval, account verification, fixed-origin HTTPS, JSON/redaction/error/quota handling. |
| `product/crates/ag1-platform/src/github/browse.rs` | Active desktop projections and PR head/base-bound review pages. |
| `product/crates/ag1-platform/src/github/tool.rs` | Single `github` registration, discovery and adapter result conversion. |
| `product/crates/ag1-platform/src/github/graph_reads.rs` | Fixed GraphQL read templates, schemas and response validation. |
| `product/crates/ag1-platform/src/github/graph_mutations.rs` | Fixed mutation templates, input/semantic result checks. |
| `product/crates/ag1-platform/src/github/pull_responses.rs` | Review/merge semantic outcome validation. |
| `product/crates/ag1-platform/src/github/workflow_responses.rs` | Workflow read/action response validation. |
| `product/crates/ag1-platform/src/github/credential_fields.rs` | Credential-shaped argument/response fields and GraphQL-name checks. |
| `product/crates/ag1-platform/src/github/secrets.rs` | Ten typed secret destinations, metadata, private value abstraction and libsodium encryption. |
| `product/crates/ag1-platform/src/github/secret_transport.rs` | Dedicated private secret publication. |
| `product/crates/ag1-platform/src/github/downloads.rs` | Temporary account/scope-bound binary receipts and paging. |
| `product/crates/ag1-platform/src/github/download_transport.rs` | Fixed API download routes and explicitly allowed credential-free signed redirects. |
| `product/crates/ag1-platform/src/github/uploads.rs` | Exact granted-file snapshots, budgets and source revision verification. |
| `product/crates/ag1-platform/src/github/upload_transport.rs` | Dedicated release upload transport. |

### Local Git, host integration and generic approval/context

| Path | Role |
| --- | --- |
| `product/crates/ag1-platform/src/github/local.rs` | Local Git admission, exact targets, contained Git, staging/commit/ref operations. |
| `product/crates/ag1-platform/src/github/local_network.rs` | Account-bound fixed-GitHub network transport. |
| `product/crates/ag1-platform/src/github/local_clone.rs` | Bounded new-folder clone. |
| `product/crates/ag1-platform/src/github/local_transition.rs` | Checkout/pull transition and retained original state. |
| `product/crates/ag1-platform/src/github/local_metadata.rs` | Explicit `.git`/linked-worktree metadata authority. |
| `product/crates/ag1-platform/src/github/local_journal.rs` | Durable private transition records. |
| `product/crates/ag1-platform/src/github/local_recovery.rs` | Explicit recovery and stale-lock rules. |
| `product/crates/ag1-platform/src/github/local_storage.rs` | Storage admission/runtime budgets; no automatic pruning. |
| `product/crates/ag1-platform/src/client/local_git.rs` | Native UI Git preview/confirm routing and upload-source grants. |
| `product/crates/ag1-platform/src/client/mod.rs` | Host service composition, tool registration and local Git grant adapters. |
| `product/crates/ag1-platform/src/session/adapters.rs` | Agent preparation/execution dispatch and fences. |
| `product/crates/ag1-platform/src/session/files.rs` | Native GitHub approval details integration. |
| `product/crates/ag1-core/src/tools/policy.rs` | Generic approval policy including GitHub mode behavior. |
| `product/crates/ag1-core/src/tools/approval.rs` | Authorization lifecycle/execution fences. |
| `product/crates/ag1-core/src/tools/result.rs` | Large GitHub result preview plus exact local artifact retrieval. |
| `product/crates/ag1-platform/src/git_tools.rs` | Separate built-in structured `diff` adapter. |
| `product/crates/ag1-platform/src/source_tools.rs` | Local repository/source retrieval signals; separate from remote GitHub indexing. |
| `product/apps/desktop/src-tauri/src/main.rs` | Actual `github_command` routing and native private-entry composition. |
| `product/apps/desktop/src-tauri/src/github_secret_entry.rs` | GTK private secret entry outside WebKit/chat. |

### Desktop components and styling

Directory: `product/apps/desktop/src/`.

| File | Role |
| --- | --- |
| `Settings.tsx`, `ThemeSettings.tsx` | Existing settings layout/control conventions; inspect before designing. |
| `GitHubSettings.tsx` | Account/repository selection, issue/PR views, compose/merge forms and feature composition. |
| `GitHubWorkbench.tsx` | Existing Advanced operation selector, JSON forms, schemas, preview/confirm and paged `Source` component. Preserve. |
| `GitHubBrowser.tsx` | Code/history/branches/tags/workflows/releases/Projects/Discussions browser. |
| `GitHubAction.tsx` | Reusable reviewed native action form; inspect before writing new approval UI. |
| `GitHubItemActions.tsx` | Issue/PR metadata/edit/state/reviewer forms. |
| `GitHubPullReview.tsx`, `ReviewPanel.tsx` | Shared paged PR diff/check review, exact head/base identity and review panel routing. |
| `GitHubWorkflow.tsx`, `GitHubRelease.tsx` | Existing workflow dispatch/rerun/cancel and release create/edit forms. |
| `GitHubInbox.tsx` | Account/repository notifications, filters and reviewed read-state writes. |
| `GitHubSecrets.tsx` | Existing metadata/access forms and native entry initiation. |
| `GitHubLocal.tsx` | All 15 local operations, previews, two-step pull, recovery and manual branch/SHA fields. |
| `GitHubGraphDetails.tsx`, `GitHubGraphActions.tsx` | Projects/Discussions nested pagination and mutation forms. |
| `GitHubDownload.tsx`, `GitHubUpload.tsx` | Existing bounded binary transfers and explicit save/upload UI. |
| `GitHubResult.tsx` | Agent-result links/actions into shared GitHub detail/review surfaces. |
| `bridge.ts`, `App.tsx`, `ApprovalMenu.tsx` | Client bridge, navigation/discuss/review integration and chat approval controls. |
| `redesign.css`, `review.css`, `app.css`, `dyad.css` | Existing Settings/GitHub/theme styles. `main.tsx` imports dyad → app → redesign; inspect effective cascade rather than adding conflicting overrides. |

The effective `.settings-content` rule in `redesign.css` caps width at **920px** with centered margins. Widening only an inner GitHub child will not solve the unused-canvas problem. Apply a scoped GitHub layout treatment at the appropriate parent; preserve the layout of unrelated Settings sections and user font/theme preferences.

## 6. Known limitations to preserve or address deliberately

### Authentication and account access

- Uses `gh auth login --hostname github.com --git-protocol https --web --skip-ssh-key`, or reuses an existing CLI account. AG1 removes ambient token overrides from its CLI commands.
- Tokens come from the CLI's credential store via native retrieval. Do not claim the CLI always uses a particular OS keychain; AG1 delegates storage to it.
- GitHub.com only; no AG1 account switcher, dedicated token-entry flow, scope configuration/refresh UI, Enterprise host configuration or GitHub App/JWT auth flow.
- Disconnect invalidates AG1's connection/pending work/private downloads; it does not revoke GitHub authorization or log the CLI out.
- Connected does not mean privileged scopes are granted. Projects/org/secrets can fail independently. Only display scope/permission claims actually returned or otherwise verified; unknown is not denied or allowed.
- Native identity verification adds API calls during prepare and execution. Do not remove these checks as a shortcut to faster loading.

### Remote workflows

- Hosted contents have dedicated reads; file create/replace/delete are mainly raw REST. No polished hosted-file editor. This editor is not mandatory for the first pass; preserve advanced access and prioritize the listed UI work.
- Remote commit/ref operations beyond lists largely use raw REST. Tags need clearer semantics.
- Normal merge currently hardcodes squash although backend schema supports merge/rebase/squash. Keep reviewed head SHA and use repository settings to offer applicable methods.
- Review listing/submission exists, but visual diff has no proper inline input. Review comments must bind path/side/line/range to the exact reviewed commit; missing/binary/truncated patches cannot be treated as empty reviewable files.
- Actions has broad reads/actions/downloads but no watcher or live streaming.
- Organization/team administration, variables, collaborator/permission endpoints and branch protection have raw schema support without dedicated general-purpose UI.
- Repository list search currently filters only the loaded page. True repository search and issue/PR search already exist in the backend. Do not imply remote search means an exhaustive offline inventory of every accessible repository.
- Projects browser derives owner login from the selected repository, although the backend accepts arbitrary explicit owners. Several field controls require manual IDs despite existing field metadata.
- Normal labels/assignees/milestone edits can replace current values. Preserve distinctions between adding entries and replacing all entries, and warn appropriately about concurrent state replacement.

### Bounds, refresh and context

- Standard desktop browse uses 30 rows/page and native pages 1–100. Typed REST accepts `page` 1–1000 and `per_page` 1–100 where supported. Preserve endpoint-specific schema constraints.
- GraphQL connections have their own `pageInfo` cursors; nested collections must retain independent pagination. Fixed detail reads generally use 30 nodes/page; UI history is bounded.
- Ordinary JSON responses are capped at 4 MiB. Agent remote preview size is separately bounded (24 KiB). Workbench source display is paged; keep large replies bounded through the client bridge.
- Downloads: 64 MiB/file, 128 MiB retained budget, up to 32 receipts, up to 15 minutes in memory, account/generation/conversation binding. No archive execution/extraction. Local `artifact` reads for externalized tool results cost no new GitHub request; do not confuse them with `downloads.read`, whose service path still verifies account identity.
- Uploads: 64 MiB, two retained upload plans, exact granted regular-file bytes and revision. No automatic replacement/deletion/retry of an existing or partially uploaded asset.
- Secrets: 48 KiB max private value; native desktop provider required. Ten supported destinations: `actions_repository`, `actions_environment`, `actions_organization`, `dependabot_repository`, `dependabot_organization`, `codespaces_repository`, `codespaces_organization`, `codespaces_user`, `agents_repository`, `agents_organization`.
- Local Git staging: 1–64 exact paths, 8 MiB/entry, 16 MiB total; executable/link bytes and tracked deletions supported without following symlinks. Clone checkout: 10,000 entries/256 MiB; network pack files bounded to 256 MiB. Preserve storage/recovery admission rules; never prune retained data merely to make tests succeed.
- No persistent remote repository/issue/PR cache, ETag revalidation, background refresh, webhook sync or automatic indexing. UI state and local artifacts are not a sync subsystem.
- Quota metadata and `rate_limit.get` exist. No automatic retry/backoff scheduler. Structured errors provide retry/reset guidance, but not every simple screen displays all of it.
- “Discuss with AG1” puts selected item context into the conversation compose flow; it is not background ingestion or permission to automatically send it. Large model results can be externalized to exact local artifacts. Separate `repo`/`diff` tools inspect local sources; no automatic remote issue/PR index was found.

## 7. Known bugs / strongly supported findings

Keep these distinct from requested features. Confirm with focused source/test evidence, then fix if confirmed; no live account mutation is needed.

### A. Issues view likely includes pull requests

The active `github_command` routes `issues` to `github/browse.rs`, whose `issues.list` result is returned without removing rows containing `pull_request`. GitHub's repository Issues endpoint returns both issues and PRs. `GitHubSettings.tsx` also renders the rows without filtering. The older route in `github.rs` has a filter, but is not the active desktop route.

Add a focused mixed-result fixture and correct the active projection/UI contract. Preserve pagination from the original API response, not the filtered row count: a page containing many PRs must not incorrectly disable Next or imply there are no further issues. Do not silently change the generic `issues.list` API contract unless intentionally designed and documented; a UI projection can classify results while retaining useful raw data.

### B. Tag creation is generic ref creation

`tags.create` sends POST `repos/{repo}/git/refs`. Its current compiler does not appear to require `refs/tags/...`. Decide the semantics explicitly: a tag-named operation should validate a real tag namespace, while legitimate generic ref/branch capability can remain available via the existing raw endpoint. Keep catalog/schema/examples/UI consistent and test accepted tag refs plus rejected non-tag refs. Do not conflate lightweight refs with annotated-tag object creation.

### C. Raw operations do not inherit specialized guarantees

Dedicated `pulls.merge` requires a 40-character reviewed SHA; dedicated `pulls.review` requires `commit_id`. Generic raw requests are schema-validated but do not automatically run the specialized compiler/response checks under those IDs. Advanced GraphQL is account-wide and always consequential.

Preserve exact request/account review and credential restrictions. Do not route new normal PR actions through raw APIs to evade dedicated safeguards. Do not advertise a selected repository or successful check as a blanket permission boundary.

### D. Secondary consistency findings

- Main repository pagination uses a row-count heuristic despite a next-page indicator being available. Review this with the search/pagination group, not as a separate audit project.
- Current account status presentation is sparse; public service identity contains fewer fields than the legacy status shape suggests. Do not fabricate names, scope grants or health from connection-marker existence.
- Some descriptive text about local metadata or quota-free reads is broader/narrower than the actual path. When touched, align copy with the implemented behavior without launching a general documentation rewrite.

## 8. Existing safety model — mandatory preservation

- Real credentials stay native: no real token/password in model arguments, renderer state, URLs, logs, test fixtures or this handoff. Owned fixtures may use clearly synthetic credentials. Private secret entry is outside WebKit and uses libsodium sealed-box encryption.
- Requests bind native account ID/login and credential revision, connection generation, exact operation/target, conversation scope, and expiry. Prepared plans are native-owned.
- Previews expire after five minutes; retained confirmations are one-use, including failures. Account changes/disconnect/stale scope invalidate authority.
- UI reads are a bounded dedicated route; writes require native preview/confirm. The generic workbench previews even raw reads because they are consequential.
- Agent Interactive and Autonomous modes require GitHub approval; trusted-tool lists alone do not bypass it. Planning/Headless reject account operations, apart from local discovery. The explicit user-selected chat Bypass approvals mode suppresses prompts but retains native account/target/scope/grant fences. Do not silently alter that policy in a UI patch.
- Local Git additionally binds grants, source/index/ref/config/metadata revisions, exact remote SHAs and retained transitions. Do not replace this with arbitrary shell commands.
- Cancellation does not undo effects already accepted by GitHub. Unknown outcomes must remain unknown; do not automatically retry writes, merge, dispatch, uploads or recovery actions.
- Read/write responses are untrusted content, not instructions. Preserve literal/safe rendering, bounded data and result artifact externalization.
- Ordinary REST/GraphQL uses a fixed origin with bounded methods/paths, pinned schema checks, credential-field filtering and no arbitrary redirects. Dedicated downloads have their own restricted credential-free signed redirect path; do not disable that existing capability by overgeneralizing “no redirects.”
- Raw REST/GraphQL can delete repositories, contents, refs or releases, remove access, execute destructive graph mutations or replace concurrent state when GitHub permits it. Secret deletion uses the dedicated `secrets.delete` route; ordinary raw secret/credential endpoints are blocked. `Consequential` covers destructive actions as well as publishing, configuration changes and potentially billable work; ordinary write operations can also replace data.

### No unsafe live testing

Do not create/delete real repositories, change real visibility/settings, merge PRs, alter collaborators/secrets/variables, dispatch/rerun/cancel real workflows, publish/delete releases, delete refs or perform other live account writes solely to test functionality without explicit authorization for that exact operation.

Use existing tests, mocks, owned HTTP/TLS fixtures, schema validation, isolated local repositories and narrowly useful non-destructive live reads if authorized/appropriate. Prefer isolated fixture desktop profiles to the user's live session. Do not close/relaunch the user's app or change their settings to run a test without authorization. Never interpret authorization to implement a button as authorization to invoke it against the user's account.

## 9. UI/backend mismatches and implementation priorities

Implement these as coherent workflows using existing components. Some low-level API capabilities need narrow native read wrappers to become convenient normal UI; that is not a reason to rebuild transport or downgrade raw risk.

| Priority/workflow | Existing basis | Expected improvement |
| --- | --- | --- |
| Real repository search/filtering | `repos.search`, `repos.list` parameters | Server-side search with intentional query timing (submit/debounce), clear loading/error/empty states, preserved filters and real pagination. Distinguish “Your repositories” from GitHub search. Handle late replies when account/query changes. |
| Repository configuration | `repos.get`, `repos.update` | Normal read/edit/review flow for description, visibility, default branch, feature flags, merge options and archive state as appropriate. Send only edited fields; separate risky visibility/archive changes clearly. |
| Account/connection overview | Existing status/account/read routes | Current account, useful connection errors, reconnect/disconnect semantics and permission information actually known. Do not claim token scopes from login alone. |
| API/quota status | `rate_limit.get`, reply/error metadata | Compact quota/reset/health information and meaningful limit errors, without continuous polling or a decorative dashboard. |
| Issue/PR search | `issues.search`, list filters | Explicit repository/type/state filters plus appropriate assignee/label/milestone controls. Preserve query and page state; keep issues and PRs distinct. |
| Merge methods | `pulls.merge`, repo merge settings | Offer applicable merge/squash/rebase choices, retain SHA binding and native review, handle settings/head changes without silent fallback. |
| Review history/inline review | `pulls.reviews`, `pulls.review`, shared PR diff | Readable history; properly anchored inline input/draft review using existing schema where cleanly supported. Bind to reviewed commit and invalidate stale drafts/targets appropriately. Do not auto-submit. |
| Labels/milestones | `issues.labels`, `issues.milestones`, `issues.create_milestone` | Real selectors and clear add/replace/remove behavior instead of unnecessary manual names/numbers. Keep pagination and permissions meaningful. |
| Branch/ref selectors | `repos.branches`, `repos.tags`, contents `ref`, existing local plans | Select branch/tag where applicable; carry exact reviewed SHA into Code, fetch/pull/clone/checkout/PR workflows. Fetch selected ref data if stale. A display selection does not create write authority. |
| Variables | Pinned Actions repo/environment/org raw endpoints | Scoped list/edit/create/delete forms with native request review for writes. Variables are plaintext API data; distinguish them clearly from private secret entry and do not present them as a safe secret store. |
| Collaborators/access | Pinned collaborator list/permission/add/change/remove endpoints | Readable access/permission view and reviewed target-specific access changes. Distinguish organization policy restrictions and account permissions from AG1's own approvals. |
| Secret repository selection | Existing secret access operations plus repo IDs | Select real repository names/IDs instead of forcing comma-separated IDs. Preserve explicit all/private/selected scope and deliberate empty-selection behavior. Existing simplified repo list omits IDs; extend a suitable projection or use richer existing responses. |
| Independent Projects owner | `projects.list`, `projects.user_list` | Explicit organization/user login independent of selected repository; account-wide scope stays visible and stale owner results are discarded. |
| Discussion creation | `discussions.create`, categories/repository IDs | Normal category/title/body form with reviewed exact repository/category target. Preserve existing comment/reply/edit/answer actions. |
| Tags/releases | `tags.create`, release operations | Safe standalone tag form after semantic fix; deliberate reviewed release deletion; retain existing draft/edit/assets/download/upload UI. No automatic cleanup of failed assets. |
| Repository creation/fork | `repos.create`, `repos.fork` | Add ordinary entry points if they fit the repository workflow cleanly; preserve workbench access regardless. |

The following **already exist in normal UI** and must not be reported as missing or recreated: connection/disconnection, repository browse, issue and PR create/edit, comment/review/merge, inbox, Local Git, workflow actions, release/assets, Projects, Discussion detail actions, secrets and API Workbench.

## 10. UI/UX and information architecture

**DO NOT MAKE IT LOOK LIKE AN AI-GENERATED WEBSITE.** Make GitHub feel like a native AG1 Settings/developer-tool surface.

Inspect current Settings controls and effective styles first. Use existing dark-theme tokens, typography and user font sizing, compact rows, modest control sizes, subtle separators/hover states, restrained borders and concise explanatory copy. Preserve keyboard navigation, focus, semantic labels, accessible errors, disabled/busy states and responsive behavior.

Avoid giant cards, nested rounded containers, huge padding, decorative pills/tiles, gradients, marketing layouts, bloated empty states, meaningless whitespace and narrow centered content with enormous gutters. Do not merely stretch a label and its action to opposite ends of a wide window. Use available canvas for functional lists/detail/configuration areas, with sensible readable text widths and inner margins. Collapse gracefully in narrower windows.

Evaluate a hierarchy such as **Overview / Repositories / Issues & Pull Requests / Actions / Variables & Secrets / Access / Notifications / Advanced**. This is guidance, not a required eight-tab implementation. Prefer fewer coherent sections when they fit AG1 better. Avoid unnecessary global/subnavigation duplication.

Default Overview should be useful and restrained:

- Connection: current account and meaningful status; appropriate refresh/reconnect/disconnect actions.
- Repository access: an honest summary/entry point; do not present the current page count as the total accessible inventory.
- API status: quota/reset data where loaded.
- Permissions/access: only facts actually determined, with clear repository/account scope.
- Notifications: compact inbox entry point.
- Advanced: clear route to the existing Workbench.

Do not fill Overview with every operation. Workbench/raw REST/GraphQL/schema discovery stay available under Advanced, but should not dominate the default page. Reuse the workbench, not rebuild it. Secret values must still use the dedicated private dialog even if the existing secret metadata UI is made easier to find outside Advanced.

Navigation changes must preserve selected account/repository scope, clear obsolete previews, discard late replies and maintain access to shared PR review/result links. Refresh changed details after successful mutations without automatically resubmitting unknown outcomes.

## 11. Mostly UI work versus actual backend extensions

### Existing backend / mostly UI work

Repository search and filters; repository configuration; issue/PR search; merge choice; review history and inline review input; label/milestone selectors; branch/ref selectors; quota/status; Variables UI; collaborator/permission/access UI; secret repository selection; Projects owner selection; Discussion creation; tag/release controls.

Supporting work can include a bounded native projection, a narrowly typed read operation, input validation or shared-component refactor. Keep it on the existing service/schema/approval path and update the agent contract if catalog entries change.

### Genuine new backend infrastructure — separate decision

Account switching; Enterprise hosts; OAuth scope management; persistent cache/ETag revalidation; background synchronization/watchers/webhooks; retry scheduling; automatic GitHub indexing; persistent GitHub action policies/repository allowlists/read-only enforcement.

Do not add pretend settings for these. Do not replace working authentication during a UI-first pass without architectural justification. If needed later, design credential lifecycle, account isolation and authorization checks separately. First deliver the substantial existing-backend UI work.

### GitHub-specific policy architecture

Evaluate where persistent GitHub rules would integrate with generic policy/preflight and native execution fences. Do not bolt on a renderer-only or second ad-hoc security system. Illustrative categories, **not current features or mandated defaults**:

| Action category | Candidate decisions |
| --- | --- |
| Read repository | Allow |
| Edit hosted files; push commits; create branches; create issues; create PRs | Allow / Ask / Never |
| Delete branches; merge PRs; trigger workflows; manage secrets | Ask / Never |
| Force push; delete repository | Never |

A real design needs consistent enforcement for UI and agent routes, raw REST/GraphQL, account/repository identity, unknown operations, existing explicit bypass, policy persistence/revision and in-flight preview invalidation. A repo allowlist must not be bypassable through account-wide GraphQL or raw endpoints. Explain what “read-only” actually constrains, including local effects. Do not claim to restrict separate shell tools with a GitHub-only rule.

If safely implementing this would significantly expand the first pass, deliver a concise design/defer note through existing project documentation conventions and finish the existing-backend UI milestones first. Do not let open-ended policy/auth/sync design become a reason to abandon the concrete UI work.

## 12. Suggested execution order

Adjust dependencies if necessary, explain major deviations briefly, then continue.

### Phase 0 — Orientation

Read instructions and this file, inspect dirty state, inspect the central routing/service/components/styles, and sample relevant tests. Confirm only the audit facts needed for the planned changes. Establish a coherent implementation plan and meaningful checkpoints; do not redo the full audit.

### Phase 1 — Correctness

Reproduce/fix the active Issues/PR classification and pagination problem. Decide/fix tag/ref semantics. Address similarly clear bugs in touched paths, keeping specialized safety intact.

### Phase 2 — Settings hierarchy

Build an everyday-first GitHub Settings structure and restrained Overview. Move existing Workbench access under Advanced, preserve all existing views/actions and widen the correct layout parent without disturbing other settings.

### Phase 3 — Repository/account fundamentals

Implement real repository search/filtering, normal repository configuration, connection/access/quota status, and branch/ref selection shared with Code and Local Git where practical.

### Phase 4 — Everyday Issues/PR workflow

Implement real issue/PR search, metadata selectors, merge methods and review history. Integrate inline review comments with exact reviewed diff identities where the current architecture supports a complete safe flow. If a specific advanced inline case is deferred, document that case rather than abandoning review UX entirely.

### Phase 5 — Existing-backend administration

Implement normal Variables and collaborator/access flows, improve secret repository selection, independent Projects owner selection, Discussion creation, and tag/release controls. Group related native read adapters and UI state handling. Do not duplicate existing secret/release/graph panels.

### Phase 6 — Policy architecture

Evaluate clean generic-policy integration. Implement only if bounded and architecturally coherent; otherwise record a concrete design and deferral. Authentication/sync infrastructure is not automatically part of this phase.

### Phase 7 — Integration and polish

Run the relevant accumulated GitHub tests/build checks, inspect representative UI widths/states, check preservation of advanced and agent capability, and report acceptance outcomes and material deferrals honestly.

## 13. Testing and validation strategy

Use existing tests as fixtures and conventions, not as proof of current success. Add meaningful tests for new behavior or regressions, not tests that merely restate CSS declarations or mirror implementation.

### Relevant tests already present

Frontend under `product/apps/desktop/src/`:

- `GitHubSettings.test.tsx`, `GitHubWorkbench.test.tsx`, `GitHubBrowser.test.tsx`
- `GitHubItemActions.test.tsx`, `GitHubPullReview.test.tsx`, `GitHubWorkflow.test.tsx`
- `GitHubGraphDetails.test.tsx`, `GitHubGraphActions.test.tsx`
- `GitHubSecrets.test.tsx`, `GitHubLocal.test.tsx`
- `GitHubDownload.test.tsx`, `GitHubUpload.test.tsx`, `GitHubResult.test.tsx`
- `ReviewPanel.test.tsx`, `App.test.tsx`, `bridge.test.ts` where shared integration changes

Do not assume every component has a same-name test: release/inbox cases currently also appear in `GitHubWorkflow.test.tsx`.

Native unit tests are colocated in `github/operations.rs`, `model_contract.rs`, `schemas.rs`, `browse.rs`, `service.rs`, `tool.rs`, graph/secret/transfer/local modules, and `github/acceptance_tests.rs`. Owned transport fixtures include `transport_fixture.rs`, `secret_transport_fixture.rs`, `upload_transport_fixture.rs`, `local_network_fixture.rs`.

Local integration contracts:

- `product/crates/ag1-platform/tests/github_local_contract.rs`
- `product/crates/ag1-platform/tests/client_local_git_contract.rs`

These local contracts are Linux-oriented and use the `ag1-process-host` Cargo binary plus `/usr/bin/git` and `/usr/bin/bwrap`; inspect prerequisite failures rather than weakening containment or silently counting skipped tests as passes.

### Focused command examples

From `product/apps/desktop`:

```sh
npm run typecheck
npm test -- --run src/GitHubSettings.test.tsx src/GitHubBrowser.test.tsx
npm test -- --run src/GitHubItemActions.test.tsx src/GitHubPullReview.test.tsx
```

From `product`:

```sh
cargo test --locked -p ag1-platform --lib github::browse
cargo test --locked -p ag1-platform --lib github::operations
cargo test --locked -p ag1-platform --lib github::model_contract
```

Choose the applicable group, not every command after each edit. Preserve the existing dependency/toolchain setup; do not upgrade packages or recreate caches just to test. Use an established build target/cache configuration if the environment has one, after inspecting it once.

At meaningful boundaries or completion, as appropriate to touched code:

```sh
# product/apps/desktop
npm test -- --run src/GitHub
npm run build

# product
cargo test --locked -p ag1-platform --lib github::
cargo test --locked -p ag1-platform --test github_local_contract --test client_local_git_contract
cargo check --locked -p ag1-desktop
```

Check actual selected test counts; if a runner's path filter selects nothing, correct the invocation instead of claiming coverage. Run relevant core policy/approval tests if those files change. Broaden to App/bridge/shared contracts when routing or generic infrastructure changes. These are examples based on current package scripts, not an instruction to run every expensive command unconditionally.

### What tests should prove

- Mixed issue/PR pages classify correctly and retain next-page metadata.
- Search is server-side, bounded and correctly scoped; pagination preserves filters; stale account/query/repo results are discarded.
- Forms construct exact schema-valid requests, send only intended fields, and keep native preview separate from confirmation.
- Edits, navigation, account/scope changes and expiry invalidate obsolete previews/drafts; double clicks do not republish.
- Merge choices respect known repo settings and exact head SHA; inline comments carry valid reviewed anchors and refuse stale/unavailable targets.
- Variable/access/secret destinations cannot drift between selection and execution; raw reads are not smuggled through `api_read`.
- Errors distinguish permission, not-found, rate limit, stale state and uncertain mutation results without leaking private data or auto-retrying.
- Shared navigation preserves Workbench, transfers, local recovery, graph/inbox views and agent-result links.
- Local Git selectors still produce exact native plans and cannot turn a branch label into unchecked SHA authority.

Use representative wide/narrow desktop and loading/empty/error/selected states for a final visual pass. Prefer existing owned fixture tools if needed; inspect their configuration before running them. `tools/probes/p14_native_github_*` scripts are historical evidence tooling, not a mandate to run every probe or a guarantee they are safe with a live profile. Do not start long endurance/model/live-account tests unless project instructions explicitly require them or the user authorizes them.

## 14. Acceptance criteria

Track these against actual implementation and test evidence; do not declare the task complete merely because one phase or build succeeds.

1. GitHub Settings accurately reflects the existing capability families without pretending every schema has a dedicated implementation.
2. Common workflows have normal UI where reasonable and no longer require users to construct Workbench JSON.
3. Advanced/raw REST/GraphQL/schema access remains available and functional.
4. Repository search performs real server-side GitHub search, with intentional request timing, scope and pagination.
5. Repository configuration has a usable read/edit/review interface over the existing backend.
6. Branch/ref selection avoids unnecessary manual names/SHAs while preserving exact target validation in Code/Local Git workflows.
7. Supported PR merge methods are exposed appropriately; hardcoded squash is removed from the normal choice flow without losing SHA protection.
8. Review history and review-input UX expose the existing backend more completely; implemented inline cases bind to reviewed diff identities.
9. Issues/PR search and label/milestone/filter controls are meaningfully improved.
10. Account/connection and rate-limit status are visible, truthful and useful; unknown authorization is not presented as granted.
11. Variables and collaborator/permission/access capabilities have appropriate normal UI in the planned scope, with reviewed writes.
12. Confirmed Issues/PR classification defects are fixed, including filtered-page pagination behavior.
13. Tag/ref semantics are safe and clear if the finding is confirmed; legitimate generic ref capability remains available through its proper surface.
14. Existing credential, approval, scope, stale-state, cancellation and uncertain-outcome guarantees remain intact.
15. No existing GitHub capability is accidentally removed, including local recovery, transfers, secrets, graph/inbox views and model access.
16. Workbench stays accessible under Advanced and no longer dominates the default page.
17. UI uses AG1 Settings theme/typography/control conventions, sensible density and useful responsive width; no separate SaaS/marketplace aesthetic.
18. Existing user work, unrelated changes, live app state and local recovery data are preserved.
19. Relevant focused and final checks pass, or specific pre-existing/environment failures are clearly separated from regressions; unrun checks are not claimed.
20. No destructive or consequential live GitHub testing occurred without explicit authorization.
21. Secret repository selection, independent Projects owner, Discussion creation and tag/release controls are addressed as planned or have specific justified limitations reported.
22. Any policy/auth/sync deferral is explicit and architectural; no nonfunctional settings or second permission model is introduced to imply completion.

## 15. Non-goals and scope boundaries

- Do not rebuild GitHub integration, Workbench, Git transport, authentication or generic approval from scratch.
- Do not interpret “full GitHub experience” as a requirement to implement all 1,229 endpoints as forms.
- Do not automatically add Enterprise/multi-account/OAuth scope infrastructure, webhook sync, persistent caching, live streaming or remote indexing to a UI-focused pass.
- Do not add a dedicated hosted-file editor or a new general Git client at the expense of the requested priorities; these are separate larger workflows unless source dependencies justify a bounded addition.
- Do not hide existing advanced capability solely to simplify the page.
- Do not weaken native approval, credential restrictions, exact SHA checks, filesystem grants or unknown-outcome handling for convenience/performance.
- Do not mutate live GitHub resources as development tests. Do not change the user's active account or credential store to simulate missing permissions.
- Do not modify MCP, monitor-refresh integration, model configuration or unrelated Settings merely because they are present in this dirty tree.
- Building a candidate for validation is in scope. Systemwide installation, replacing the running build, restarting the user's app, publishing, pushing or deploying require current authorization; do not inherit old historical release instructions as a fresh deployment request.
- Do not commit, stage all work or sweep unrelated changes into a PR unless requested. Avoid any broad cleanup of the pre-existing dirty workspace.

## 16. Final report from the implementing Astra

Report factually and concisely:

- What changed in normal GitHub UI/workflows.
- Confirmed bugs fixed and their resulting behavior.
- Existing backend capabilities surfaced versus genuinely new backend code/infrastructure.
- Important files modified.
- Tests/build/visual checks actually run and their outcomes; any unrun or blocked validation.
- Remaining limitations and precise deferrals, especially policy/auth/sync work and any incomplete acceptance item.
- Architectural concerns discovered and why a deferred extension needs separate design.
- Whether any live operations or installation occurred; do not imply deployment from a source build.

The desired outcome is substantial, coherent implementation with proportional evidence—not another long audit, a cosmetic-only pass, or repeated micro-verification.
