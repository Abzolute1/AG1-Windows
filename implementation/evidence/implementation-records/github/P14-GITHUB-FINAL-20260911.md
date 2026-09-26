# GitHub integration — complete and installed

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

GitHub Phase 2 is complete within the documented capability boundary, verified and installed systemwide. The user authorized GitHub-only completion and checkpoint installation, then explicitly instructed stopping without starting another plan item. No GitHub implementation or installation work remains. User testing can begin after fully quitting and reopening AG1. MCP and unrelated phases were not started.

## Final audit fixes

Ordinary REST/GraphQL inputs and replies now share credential-field classification, including camel-case and token suffixes. The advanced GraphQL scanner rejects executable credential names, including direct input keys and aliases, while skipping comments and literal prose. Every advanced graph operation remains consequential; this scanner does not classify query risk or validate the server schema. Pinned REST template matching distinguishes actual private resource names from repository/owner/label/content names such as `secrets` or `keys`. Numeric-only generic path parameters cannot conceal a private route. Account Codespaces and installation JSON routes are available within the documented advanced boundary; their credential-specific endpoints remain refused.

Typed PR reviews require a valid receipt ID, the requested commit and the state corresponding to COMMENT/APPROVE/REQUEST_CHANGES or a pending review. Merge requires HTTP 200, `merged: true` and a valid merge commit SHA. Unexpected receipts are uncertain, consume confirmation and require refresh. These follow the official [review contract](https://docs.github.com/en/rest/pulls/reviews?apiVersion=2026-03-10#create-a-review-for-a-pull-request) and [merge contract](https://docs.github.com/en/rest/pulls/pulls?apiVersion=2026-03-10#merge-a-pull-request).

Native local/network/read-only Git ignores replacement refs. Private read-only overlays isolate `info` and `objects/info`; nonempty alternate object-store pointers explicitly refuse writes. Existing repository files are preserved. The storage lease explicitly unlocks at the end of owned work, so an unrelated thread's transient forked descriptor cannot extend the lock's lifetime. A retained duplicate descriptor regression covers this case. Local quota errors now use readable wording.

## Automated verification

- Full frontend: **220 tests / 28 files**, typecheck and renderer build pass.
- **77 GitHub unit tests pass**. New cross-area fixtures exercise 11 representative remote operations across repositories, issues, review/merge, Actions, releases, Discussions, Projects, notifications and advanced REST/GraphQL. Each checks permission/rate guidance, interruption, late cancel/disconnect, stale scope/account/credential/generation, early cancellation, exact once-only send and no replay/private-result disclosure. Local Git has its separate owned filesystem/HTTPS, grant, crash and recovery matrix.
- **44 relevant native contracts pass**: 9 Git publication, 14 local Git, 3 client Git/review/folder, 2 Git read/worktree, 13 change receipts, 1 client receipt and 2 crash/Undo.
- All **37** pinned public GraphQL templates validate using the retained official schema and isolated graphql-core 3.2.6 checker; no live account is involved.
- Frozen comparisons: core 20 pass / 5 fail; platform 32 pass / 2 fail. Failing names exactly match the seven preexisting baseline failures in `.local/p14-review-goal/tool-comparisons.txt`. Expectations were not modified. This is not a claim that the entire repository suite is green.

The initial final-hardening run exposed `user/tokens` being mistaken for a generic numeric user path; corrected and rerun. The initial combined final test run also caught a storage lease timing issue and two newly written fixtures using the wrong typed GraphQL argument shape; fixes and exact reruns pass. Failed logs are retained. Evidence lives in `.local/p14-github-resume-20260910/final-*`.

## Supported coverage and explicit limits

All nine requested matrix areas have typed/everyday presentation or deliberate advanced access. “Full reach” does not mean 1,229 bespoke screens or a claim that every discovered schema has received a live-account success test. Native schemas/catalogs are discoverable to both UI and model; long-tail JSON operations remain exact, consequential requests. Broad native permissions do not bypass account confirmation.

GitHub.com only. Account permissions, Projects node membership and recursive/unsupported schema details remain server-validated. General token/private-credential administration and app/JWT authentication flows are unsupported; private secret operations use only the dedicated native channel. Arbitrary binary/streaming API endpoints are unsupported; dedicated logs/artifact/release downloads and release uploads have explicit quotas. No real-account mutation, paid CI or real inference is used in acceptance.

Local Git supports ordinary in-folder repositories and reciprocal standard linked worktrees with explicit grants. Arbitrary separate Git directories, submodule metadata layouts, alternate object stores and cross-filesystem linked publication are refused for writes. Storage budgets are 4 GiB repository/retained Git metadata, 1 GiB private storage for new operations, 2 GiB for explicit recovery, and 100,000 entries. These are measured native admission/runtime budgets, not kernel quotas controlling external writers. Unknown locks/retained states may require manual intervention; no automatic pruning or unlimited recovery capacity is promised.

## Native candidate, production package and installation

Diagnostic candidate `native-candidate-Xx0YRL`, desktop SHA `f7329ea547bae8b5c6ce37df03d0bfbffb28822c043de968814e503086d02b82`, unchanged diagnostic helper SHA `eba878797415db698fccf5511a08c7bac5ae3f206cd1a43af647c5e7929fb2a4` passes all **12 actual native WebKit audits**:

| Audit | Result directory |
| --- | --- |
| PR review/merge actions | `pull-Y7mSAN` |
| Shared PR file/check review | `github-pr-review-4NAfSz` |
| Rich item changes | `github-items-tenMIq` |
| Graph details/paging | `github-graph-cp4jEL` |
| Graph changes | `graph-edit-TXIFWN` |
| Downloads | `download-ap571F` |
| Uploads | `upload-y4N4i8` |
| Private secret entry | `secret-0C1JYR` |
| Workflow/release/inbox | `flows-0Eowqf` |
| Quota/linked recovery | `quota-Hl5Q9e` |
| Ordinary Git recovery | `github-recovery-PLDtIr` |
| Baseline UI/model GitHub | `github-04uEvt` |

All run sequentially on private authenticated Xvfb displays against owned fixtures. Selected narrow merge, PR check, graph action, release edit, quota refusal and native secret-entry screenshots were inspected. The first new PR-action probe stopped before startup because its directory prefix exceeded Unix socket path limits; shortened probe-only prefix and all exact reruns pass. No product authority was changed for the probe. Native result inventory is `final-native-evidence.json`.

Production package `.local/p14-packages/488402f6b2504bfea3a7562c2330cf53/ag1-0.1.0-rc-linux-x86_64.tar.gz`, archive SHA `1e1e14b8ef7aab26e240e3ecd7d73a8a9945160df11907cd8581b49bef006bba`, source digest `5a4fe1c73c62d7fc176495f35e74a4ffa0159a2705fe6901a8a539a3878787e0`. Release build, renderer bundle audit, license/provenance and loader checks, packaged CLI checks and isolated production startup pass. Production screenshot inspected; zero provider requests and no leaked owned children.

Installed `/opt/ag1/releases/8bf70ba10402450a82054ea118f2b0b1` through the verified systemwide installer. Desktop SHA `e3d18956c55b5fca782e34ec0c23a2815103e2e9a315066c99119af1b69cc6b5`. All **1,142 payload files** match bytes, size and mode; archive receipt matches; no pending installer state. `/opt/ag1/current` and the desktop launcher select this release. The previous [storage checkpoint](P14-GITHUB-STORAGE-20260911.md) remains retained. Protected release helper, model profiles, runtime capture and native-host configuration hashes are unchanged. Authentication was supplied only through private stdin; no password was stored.

Exact verification: `.local/p14-github-resume-20260910/final-installation-verified.json`. The configured process helper remains the existing runtime helper; this desktop installation does not adopt a new model/runtime configuration. Full P14 release acceptance, other phases and real-account permission testing are not claimed by this GitHub completion.

Final checkpoint summary: **GitHub work remaining: none within this documented scope. Build/install verified; stop here.**
