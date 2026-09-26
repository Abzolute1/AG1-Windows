# GitHub usability follow-up — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user reopened the GitHub repair after another Qwen assessment. The actual session `chat-1e20e10f9825fe15bf2e7500113c2920` was inspected read-only. Sanitized call/error metadata is retained in `.local/github-usability-20260911/observed-calls.json`.

The trace distinguishes harness problems from the model's account: slash IDs resolved after the first repair; failures on those calls were body placement/validation. The model repeatedly supplied flattened request fields or a nested body inside parameters, and queried `repos/releases` eleven times without a useful recovery. Its summary mixed pre-update and post-update behavior. `issues/list` targeted repository issues; local artifact reads do not consume GitHub quota.

Implemented: unambiguous parameter/body normalization with duplicate-conflict rejection, repository shorthand inside parameters, `repos/releases` and dotted upstream aliases, native consequential releases.delete, rate_limit.get, complete REST input schemas and call examples, a compact canonical catalog with optional details and filtering, and structured large-list previews retaining status/pagination/quota data plus exact local artifact recovery. Native account binding, approval and credential guards remain enforced. Arbitrary writes are not batched or retried automatically; GraphQL can combine selected read fields.

Verification: 86 GitHub units, 17 related native integration tests, five existing core result contracts and one new GitHub preview/raw-recovery contract pass. The first follow-up unit run had one incorrect new assertion that release search would exclude related tag/download operations; the assertion was corrected to check bounded relevant search results. That failed-run log is retained. Existing five frontend model-picker failures from the previous checkpoint remain separately recorded; renderer source was not changed.

Installed and all 1,142 payload files verified: /opt/ag1/releases/d96cab4b828a43509414c6747dbe2d58. Production profile `github-usability-production-20260911` inherits release settings; previously evidenced profiles/binaries were preserved. Package/installation/verification evidence is in `.local/github-usability-20260911/`.

## First authorized live test (completed)

The user explicitly requested a fresh chat in the installed application with Qwen3.6-35B-A3B-UD-Q4_K_XL, without capturing their desktop. The application runs on authenticated private X display :600 with isolated WebKit caches, using the normal desktop state so its new test chat remains available. Every screenshot explicitly targets the private display. No screenshot of the user's live desktop or YouTube was taken. Initial private X focus setup failed before input/inference; owned processes were stopped and the mapped-window focus fix was applied in the test driver.

First test chat: `chat-52ca69e3598a640c9f2471320eea0d60`. Private driver `.local/github-usability-20260911/private_ui.py`; live prompt, process IDs, screenshots, monitoring script and observations are retained in that directory (`private-live-2/` for the current display). Live checks are read-only, use public cli/cli where possible, and inspect write schemas without remote mutation. Do not declare live acceptance until actual tool receipts and the model's final report have been reviewed. Stop/reap the owned test app/display after completion. MCP remains paused at MCP-HTTP.md.

## First installed live run findings and correction

The first installed Qwen run returned HTTP 200 for repository list/search/get/contents/commits/branches/tags, release list/get and slash alias/page two, issue list/get/comments, PR list/get/files, workflows/workflow runs and rate-limit discovery. The model recovered exact release JSON with a local artifact read. Write schemas were inspected without mutation. GraphQL was repeatedly supplied a repository scope, including an empty optional repo string; empty repo is now normalized to omission. The catalog now exposes required parameters/body fields and scope guidance, and issues.issue_labels explicitly addresses one issue while issues.labels explains its repository-wide scope. The original observations and failures remain retained; they are not final acceptance.

Final correction source and production build are under `.local/github-usability-live-fix-20260911/`, using fresh profiles `github-usability-live-fix` and `github-usability-live-production-20260911`. Targeted model-contract checks pass (9). Full relevant checks/build/install and the final fresh Qwen chat are still pending.

## Accepted final installed checkpoint

Installed `/opt/ag1/releases/d63f23faefad44c8b99e5f09ca02606f`; all 1,142 payload files, receipt and protected configuration hashes verified. Fresh profiles preserve all prior accepted binaries. 87 GitHub units pass serially and 14 native GitHub contracts pass. The first concurrent run had two local recovery conflicts (85 passed); the complete serial rerun of the same source/profile passed, with both logs retained.

Final fresh Qwen3.6-35B-A3B chat `chat-006deb4cc7ae0f9d01997ce3101af52d` ran the installed build on authenticated private display :600. All nine network calls returned HTTP 200: GraphQL with empty repo and parameters.query, release alias/pages one and two, upstream dotted content alias, all-workflow runs, repository issues, slash PR list/files and issue-specific labels. Five write schemas were discovered, and exact local artifact bytes recovered. No retry or tool error occurred. The model reported no remaining harness problems within this bounded test; live writes were not performed. Its total-call count omits catalog (actual: 9 network, 5 schema, 1 catalog, 1 artifact). Owned app/display processes were stopped and reaped; no user desktop capture occurred.

Evidence: [machine report](../../P14/github-usability-20260911.json), `.local/github-usability-live-fix-20260911/model-final-report.md`, package/installation receipts and live observations. This completes the requested GitHub checkpoint. OpenAI subscription OAuth model-picker work now follows; MCP remains paused.
