# Astra interface cleanup — surgical pass

User authorized the surgical items in the [assessment](../implementation/evidence/implementation-records/desktop/ASTRA-INTERFACE-REVIEW-20260911.md) first, followed by a report. No subagent work is authorized.

Implemented: native description/prompt deduplication; host Runtime prerequisite wording; preferred edit and optional preview/sequential edit guidance; evidence/task/memory hierarchy; existing observation projection guidance; coarse sequential todo semantics matching validation; historical exploration-board labeling; GitHub example placeholder/reuse wording; exact legacy desktop default recognition while preserving edited/custom prompts.

The old desktop default was configured verbatim as a custom prompt, bypassing the current guidance. Recognition is limited to its exact SHA-256. Native-host.json and all local model/CV settings remain unchanged.

Status: complete; installed and live verified with Luna and Qwen35B. The initial Qwen attempt was blocked by the user-owned endpoint; after the user closed AG1, the installed test passed and its runtime shut down cleanly.

Deeper items remain separate: claim-scoped lease release, model-accessible preview lifecycle/dynamic availability, unified edit/retrieval interfaces, richer task projections, and revision-aware board freshness. Historical labeling does not itself establish freshness.

Evidence and accepted package: see [the report](../implementation/evidence/implementation-records/desktop/ASTRA-SURGICAL-INTERFACE-20260911.md). MCP remains paused.
