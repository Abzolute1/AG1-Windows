# Astra interface — surgical pass, 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Implemented and installed systemwide at `/opt/ag1/releases/719340dfabe4460fb54c61d385e186f2`. [Machine evidence](../../P14/astra-surgical-interface-20260911.json). The installed Luna and Qwen35B live checks and focused contracts passed.

Changes:

- Built-in operational guidance now lives in native tool descriptions once, instead of being repeated in the system prompt. Reviewed descriptions replace overlapping legacy text; pinned argument schemas and historical tool semantics are preserved.
- Browser tools explicitly identify Runtime as the host UI, explain how the user supplies an owned ready URL, and distinguish installed browser capability from a running preview. The duplicated screenshot restriction is removed; readiness/grant enforcement remains intact.
- `edit` is the preferred exact editor, matching disjoint replacements against the original file. `editApply` is reserved for intentional sequential dependencies; `editPreview` is optional and explicitly returns no apply token. Fuzzy normalization is described without implying semantic matching.
- Evidence, task state and project memory have distinct roles. Optional todo/episode work is not required for every call; runtime observations explain their existing task-state projection.
- Todo is described as coarse sequential phases, with stable IDs and truthful blockers. Its old activation-before-completion wording contradicted the existing validator and has been removed from native presentation.
- Explore explains its workspace-persistent prior snapshot and retention limits. Child prompts label those summaries historical and not independently verified; this does not implement source freshness validation.
- GitHub examples now say to preserve structure and replace placeholders, and to reuse already discovered operation schemas. Existing transport/approval behavior is unchanged.
- Lease guidance accurately discloses current session-wide automatic release after a successful built-in check and the existing opt-out. Claim-scoped lease release is still deferred.

A deployment-specific issue was found: native-host.json contains an exact copy of the old 16,532-byte desktop default as a custom system prompt, bypassing newer defaults. The exact SHA-256 `e48c75aaae786f4f3c2c8baa0964967063ccb3b3c3aa72affed44ace5bba9d46` now selects current native guidance. Any edited/custom prompt remains a replacement. No configuration file is rewritten. The retained test proves both cases.

Verification: 6 prompt contracts, 1 prompt ownership/forgery contract, 1 real native exploration/board contract and 3 target ownership/receipt contracts passed. The production package passed its renderer audit and installation checks; all 1,142 payload files, receipt, and protected hashes matched. This does not assert a full Rust-suite pass; previously documented reference mismatches were outside this pass.

Luna was exercised through the production candidate and installed binary in owned scratch workspaces. It read the fixture once, used no other tools, explained the correct edit semantics and host Runtime prerequisite, avoided ceremonial records, and treated old board summaries as historical. The installed request envelopes contain the current model identity/hierarchy/browser guidance, no legacy opening and no obsolete todo sequencing. Its system text was 3,792 bytes plus 9,622 bytes of tool descriptions (argument schemas excluded). These figures are specific to the selected tool set and scratch path, not a universal token saving.

The first Qwen35B probe was rejected before generation with `Local inference endpoint already has a host owner`. The user-owned instance was left running. Its settings were not changed and no alternative launch configuration was fabricated. No screenshot or desktop capture was used anywhere in this pass.

Retained evidence is under `.local/astra-interface-surgical-20260911/`. The preliminary package is superseded by the final profile `astra-interface-final-production-20260911`; no accepted binary was overwritten. An initial test filter selected zero tests, and the corrected fully qualified native exploration test subsequently ran and passed.

Restart AG1 to load this installation. The remaining medium-sized changes are listed in the machine evidence and [original assessment](ASTRA-INTERFACE-REVIEW-20260911.md). MCP remains paused.

After the user closed AG1, the installed Qwen35B test passed in 101.68 seconds. It read the owned fixture once, made no changes, and correctly explained editing, host preview prerequisites, optional bookkeeping and historical-board limits. Its retained request envelopes confirm current native guidance. The CLI shut down and released its local endpoint; protected hashes still match. The surgical pass is complete.
