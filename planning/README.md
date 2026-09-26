# Planning documents

The current remaining feature work is indexed in [tasks/README.md](../tasks/README.md), with separate artifact/review, MCP, chat-cleanup and final-verification task files. GitHub is complete; this documentation cleanup does not resume the remaining implementation.

The documents in this directory retain the original architecture decisions, coverage, phase graph and runtime-preservation requirements. They are reference and gate inputs, not the current feature queue. Broader release acceptance remains governed by [P14](../phases/P14-release-and-long-horizon-gates.md).

## Historical planning-stage handoff — 2026-09-04

The following describes the original pre-implementation planning snapshot. Its statements about implementation not having begun and P01 being next are historical.

Start with [ROADMAP](../ROADMAP.md), then the [phase index](../phases/README.md). No implementation phase has run.

- [llama.cpp preservation](LLAMA_CPP_PRESERVATION.md): recorded user requirement, three configured backends, eight binding invariants and remaining identity checks.
- [Decisions](DECISIONS.md): final planning defaults for ownership, protocol, storage, clients, UI and gates.
- [Coverage](COVERAGE.md) and [machine ledger](coverage.json): all 85 capabilities, 30 invariants, four defects and eight runtime requirements.
- [Runtime fingerprints](evidence/llama-runtime-lock.json): read-only configuration and executable/adjacent library hashes; no model was run or changed.
- [Plan validation](evidence/plan-validation.json): structural coverage, dependencies, links and preservation checks. This is document validation, not product testing.

Existing research evidence remains historical. Its statement that no roadmap existed describes the preceding research stage. The current stage adds planning documents only. Source revisions remain unchanged. The next authorized work is executing a chosen phase, starting with P01 when no phase evidence exists.
