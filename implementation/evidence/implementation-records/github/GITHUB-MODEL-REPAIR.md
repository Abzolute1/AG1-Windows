# GitHub model-call repair — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user reopened GitHub after Qwen could execute only `repos.list` and `repos.search`. MCP is paused at [MCP-HTTP.md](../mcp/MCP-HTTP.md); its source and evidence are retained. The earlier GitHub completion report is a historical fixture/UI checkpoint, not proof that its model-facing discovery contract worked in practice.

## Cause and change

Execution accepted AG1 dotted catalog IDs, a top-level `repo: "owner/name"`, AG1 path fields such as `number`, and a separate `body`. Discovery instead returned untranslated upstream IDs and path fields. For example, `issues.list` described repository issues but discovery of `issues/list` returned account issues. Service preparation then discarded the specific validation failure.

`github/model_contract.rs` now translates model discovery and normalizes model requests before native admission. It accepts catalog IDs, slash spellings with the same catalog meaning, and upstream IDs that map to catalog routes. Conflicting repository or numeric-target aliases fail before account access. Upstream-only schema IDs use the explicit `rest:` namespace and explain execution through consequential `api.rest`; `rest:issues/list` retains account scope. Native account binding, exact approval, risk classification, secret custody, body bounds and no-replay behavior remain enforced.

| Reported pain point | Repair |
| --- | --- |
| Most calls rejected | Regression matrix covers repository get/content, issue list/get/create, PR list/get/create/merge, Actions runs/workflows and advanced GraphQL, including slash/upstream aliases |
| Catalog/schema names differ | Model schemas return the executable catalog ID and translated AG1 parameter fields |
| Missing schema discovery | Every catalog ID and slash alias is tested; upstream-only search results are explicitly namespaced |
| Ambiguous `issues/list` | Catalog alias always means repository issues; account issue schema is `rest:issues/list` |
| Missing GraphQL schema | Required `body.query`, optional variables/operationName, executable example, account scope and pagination guidance |
| owner/repo mismatch | Top-level `owner/name` documented; paired upstream `parameters.owner`/`parameters.repo` accepted and checked for conflicts |
| Generic errors | Native service retains safe local diagnostics for missing repo/path/body fields and schema types, without echoing supplied values |
| Hidden body requirements | Catalog advertises required body fields; operation schemas include bodyRequired, body schema and AG1 commit-SHA requirements |
| Local/network Git unclear | Catalog adds a short guide distinguishing working-copy operations, network transfers and hosted GitHub API operations |
| Misleading quota drop | Replies expose `rate_resource` and `rate_limit`; catalog explains independent buckets and reset/retry handling |
| Externalized large results | Catalog explains exact local artifact recovery and that artifact reads use no GitHub quota; recommends narrow filters/pages |
| Pagination unclear | Per-operation page/per_page discovery, REST versus schema page numbering, next-page guidance and GraphQL cursor guidance |

[GitHub's rate-limit documentation](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api) confirms independent resources and the `x-ratelimit-resource` header. A core remaining count followed by a search remaining count does not measure consumption from one bucket. No claim is made that rate limits are configurable by AG1, or that arbitrary REST requests can be batched automatically.

## Verification and resume

Current source: **84 GitHub units pass**, including seven new model contract/service/owned TLS tests. **17 related native integration tests pass** (local Git, client Git, folder access and change review). The initial isolated GitHub run had 21 missing-helper failures; after building that profile's own process helper, the complete run passed. This was test setup, not an accepted regression.

The broader frontend run has 215 passes and five failures in `App.test.tsx` concerning the initial/second model selection in new chat. All 27 other frontend files pass, including all GitHub tests. No renderer source was edited by this repair. The isolated rerun reproduces the same five failures (25 pass); its log is retained to distinguish these current frontend failures from this native GitHub change; they are not the seven historical frozen comparison failures.

Working evidence and production packaging: `.local/github-model-repair-20260911/`. Test profile `github-model-repair`; production profile `github-production-20260911` inherits Cargo release settings and writes a separate output directory to preserve previously evidenced binaries. `package.py` uses the normal package/audit pipeline with that isolated release profile and isolated evidence output. No real GitHub writes, paid CI or model inference were performed.

Production package is built and verified: `.local/p14-packages/dee3565e132040119c8687e9125a7ba6/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `419bb40e03685ed29b10f0e169a79a7728a091bbbbe55ad0f3bfb7aaaa13edbf`; source digest `1fc5acd3f0858c47b4c482d7cb7be1042613513098e9c3185aaa27721e03b574`. Packaged desktop SHA-256 `c5f52478b1490c9f50a2801baa6e9615586c3c3e0c97bb1c40281fe6f35ab1b2`. Normal bundle audit/typecheck, native loader checks, CLI checks, full package installer verification and isolated production startup pass. The startup screenshot was inspected; zero provider requests and no leaked owned children. All captured protected hashes are unchanged.

**Installed and verified.** The user supplied local sudo authentication; the normal systemwide installer selected `/opt/ag1/releases/af6444ab90ec45e0be3f65056a650dc7`. All 1,142 payload files match their recorded bytes, sizes and modes; the installation receipt matches the exact archive, there is no pending installer state, and all captured protected hashes remain unchanged. The verification is retained in `.local/github-model-repair-20260911/installation-verified.json`. The earlier password-required attempt remains historical evidence. Authentication was supplied through stdin and was not written to project files.

Fully quit/reopen AG1 and begin a fresh conversation so it sees the corrected contract. Do not rebuild the accepted repair profiles in place. MCP remains paused at its saved checkpoint.

Machine evidence: [github-model-repair-20260911.json](../../P14/github-model-repair-20260911.json). No claim of a live Qwen/account test is made. The five renderer test failures are separately recorded, not fixed as part of this GitHub repair.
