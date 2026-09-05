# AG1 Desktop research handoff

Research date: 2026-09-04. This workspace contains investigation and planning inputs, not a product implementation. Read `../VISION.md`, the documents below, then `../MASTER-PROMPT.md`.

## Frozen evidence

| Source | Location | Revision |
|---|---|---|
| Authoritative AG1 | `/home/alex/Projects/Cruzible` | `e30f0b4b238ee5618928bcd17b4571eb046e7038` |
| Dyad | `/home/alex/Projects/AG1/vendor/dyad` | `fb17039bfd2adcac7d74b9c1178a3a3524df35b6` |
| Disposable AG1 test copy | `research/scratch/ag1-reference` | `git archive` of the AG1 revision, with copied dependencies and rebuilt reference packages |

Dyad was cloned from `https://github.com/dyad-sh/dyad.git` with depth 1 and switched to `research/ag1-desktop`. No remote fork was created: `gh` was unavailable and no authenticated fork facility was established. The local clone satisfies the fallback in VISION. Do not treat its `origin` as a writable personal fork.

Cruzible already had four deleted `proposal/*.md` files and untracked `Graphic_demos/` and `proposals.md`. The initial status is in `evidence/ag1-original-status.txt`. Active tracked source was clean. Research did not modify that repository. Do not restore or delete its pre-existing changes.

## Reading order

1. [AG1 architecture](AG1_ARCHITECTURE.md) and [Dyad architecture](DYAD_ARCHITECTURE.md).
2. [Parity matrix](AG1_PARITY_MATRIX.md), [UI reuse map](DYAD_UI_REUSE_MAP.md), [license audit](LICENSE_AUDIT.md).
3. [Defects and debt](BUGS_AND_TECH_DEBT.md), [invariants](INVARIANTS.md), [performance](PERFORMANCE_BASELINE.md).
4. [Target architecture](TARGET_ARCHITECTURE.md), [dependency graph](DEPENDENCY_GRAPH.md), [migration risks](MIGRATION_RISKS.md), [test strategy](TEST_STRATEGY.md).
5. [Planning prompt](../MASTER-PROMPT.md).

## Evidence conventions

Repository-relative paths in the AG1 documents resolve against Cruzible; Dyad paths resolve against `vendor/dyad`. In compact tables, `A:` means `packages/coding-agent/src/`, `G:` means `packages/agent/src/`, `M:` means `packages/ai/src/`, and `T:` means `packages/coding-agent/test/`. These prefixes denote exact filesystem paths, not conceptual modules. Named functions are more durable anchors than line numbers. Reverify when revisions change.

**Active** means reachable from a shipped entry point or an explicitly selected supported API/mode; it does not imply every user enables it. **Partial** means implemented portions lack complete production wiring or guarantees. **Experimental** means opt-in work without established general runtime adoption. **Legacy** means retained fallback/compatibility or replaced behavior. **Absent/planned** means no active implementation found. Unreferenced exported code is reported as unintegrated, not conclusively deleted/dead: external SDK consumers are not visible here.

The discovery covered all 524 TypeScript source modules in the package inventory, with deep execution tracing of the runtime, context, tools, persistence, models, native embedding and UI seams. The machine inventory is a completeness aid, not a claim that every line was formally audited. It records exports, imports, line counts and SHA-256 values. The parity matrix represents the meaningful capability families and explicitly records uncertain/optional systems. Provider interoperability, all operating systems and live long-duration model runs remain unmeasured; their closure gates are specified.

## Reproducible evidence

- `evidence/ag1-source-inventory.json`: 524 source modules and hashes.
- `evidence/research-validation.json`: final deliverable/ID checks, literal prefixed path checks, test totals, unchanged original source hashes/status and clean Dyad checkout.
- `evidence/ag1-test-inventory.txt`: 356 existing test paths in package `test` trees; other test layouts are not included in this count.
- `evidence/ag1-reference-build.log`: successful rebuild of TUI, AI, agent and coding-agent in the copy.
- `evidence/ag1-tests.json`, `evidence/ag1-tests.log`: selected coding-agent reference tests, 465 passed; not the entire monorepo suite.
- `evidence/ag1-godot-tests.json`, `evidence/ag1-godot-tests.log`: native Godot adapter suite, 26 tests passed. Combined executed total: 491 passing tests.
- `evidence/measure-ag1.mjs`, `evidence/ag1-microbench.json`, `evidence/ag1-microbench.log`: real module calls, synthetic fixtures and defect probes.
- `evidence/probe-artifact-fork.mjs`, `evidence/ag1-artifact-fork-probe.json`: branch artifact-closure defect probe.
- `evidence/ag1-cli-startup.json`: ten fresh Node CLI version processes; not full interactive startup.
- `evidence/dyad-ui-paths.txt`, `evidence/dyad-electron-node-boundaries.txt`, `evidence/dyad-pro-imports.txt`: source search inventories. Text searches are not complete transitive dependency proofs.
- `evidence/dyad-dependency-license-inventory.json`: 1,783 lockfile package records, including dev/platform/linked records; not a redistribution clearance certificate.

Scratch data is disposable. Keep evidence reports and harnesses; never redistribute `vendor/dyad` wholesale as AG1 source. No Rust/Tauri application, roadmap phases or production rewrite was created during this task.
