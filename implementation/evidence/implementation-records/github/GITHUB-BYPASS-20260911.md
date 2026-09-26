# GitHub bypass approval correction — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user reported repeated GitHub approval prompts with Bypass approvals enabled. The root cause was a GitHub-specific policy branch that required confirmation before reaching the general disabled-approval branch. This was deterministic harness policy, not a model error.

The narrow correction honors explicit `Mode::Disabled` for GitHub tool calls. Interactive and autonomous modes still require confirmation even if GitHub is in the trusted-tools list. Planning/headless restrictions and discovery behavior are preserved. Native preparation, account/credential/connection checks, request validation, granted folders, source/target revisions, execution scope and single-use authorization remain unchanged. Direct GitHub UI review/confirmation flows remain unchanged.

The bypass dialog now explicitly includes connected GitHub operations. Tool descriptions, catalog notes, GraphQL guidance and GitHub documentation reflect the same behavior. Bypass remains scoped to the current chat/app lifetime. As before, selecting it during an active response applies to the next response; this pass does not introduce mid-operation policy changes.

Validation: 88 GitHub unit/fixture tests passed, including the new policy matrix and repeated-call authorization regression; one existing UI bypass consent/backend-delivery test passed; frontend TypeScript/Vite build passed. The new tests cover read/write/merge/GraphQL/local-Git policy decisions, no pending prompt for repeated bypass calls, and rejection of changed policy revisions, targets and sessions. Existing native service and owned TLS/Git fixtures verify account, credential, connection, recovery and request boundaries.

The first broad test run lacked `ag1-process-host` beside the fresh test-profile binary, causing 21 local-Git fixture setup failures (`NotFound`). Building that helper in the isolated test profile resolved those setup failures; the rerun passed all 88 tests. Both logs are retained. The previously accepted process helper was not overwritten. No real account mutation, real model inference or user desktop capture was performed in this correction.

Evidence: `.local/github-bypass-20260911/`, including exact before files and patch. Production profile: `github-bypass-production-20260911`. The last weekly usage check showed 6% remaining at 11:50 UTC.

Status: **installed and verified**. Current release: `/opt/ag1/releases/5a3409cbc2a74ca0ac0ecd39d8582ff6`. All 1,142 installed payload files, installation receipt and protected runtime/CV hashes verified. Archive SHA256: `a54a5543f4a868d32c86ea2163e878720da237274d196c32e7038337f592f24e`. Previous release `f1f0ec5c982c4a04b2853c1213f4847e` is retained.

[Machine evidence](../../P14/github-bypass-20260911.json). Fully quit/reopen AG1, enable Bypass approvals in the intended chat, and send a new message. The user window was not interrupted.
