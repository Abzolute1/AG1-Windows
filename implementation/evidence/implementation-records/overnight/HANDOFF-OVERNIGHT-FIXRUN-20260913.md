# Overnight fix run — 2026-09-13

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user reported `NotFound` immediately after opening MCP, asked to finish the
remaining fixes, authorized systemwide installation, and explicitly prohibited
an eight-hour run. They also asked for small checkpoint checks and one larger
validation at the end. This handoff supersedes the remaining implementation queue
in `HANDOFF-OVERNIGHT-FIXES-20260913.md`; both earlier handoffs retain historical
receipts. Evidence is `implementation/evidence/overnight-fixrun-20260913/` (E).

## Implemented

- **MCP after upgrade:** saved desktop tooling pointed at a removed
  `/opt/ag1/releases/<old>/bin/ag1-process-host`. Opening a workspace now resolves
  an installation-owned tooling helper to the sibling of the running packaged
  desktop, after choosing the actual workspace configuration. Custom helpers,
  helpers belonging to another installation, and the separate model supervisor
  binding remain unchanged. An incomplete current payload produces an actionable
  reinstall error. Unit coverage recreates the missing old helper, opens the real
  native MCP manager, lists its empty general-chat scope, and preserves custom
  configuration. No live configuration file is rewritten.
- **Custody disk growth:** new snapshots of `context-original-journal`,
  `runtime-request-input`, and `context-original-request` use lossless zlib when
  at least 16 KiB and at least 12.5% smaller. Existing raw blobs remain unchanged.
  Logical bytes, lengths, SHA-256, artifact records and access boundaries remain
  authoritative. Shared streaming decoding covers pages, integrity, search and
  exports, with exact length/stream termination/hash checks and a 256 MiB bound.
  Search retains cancellation checks and its final access recheck. No raw cache
  or historical pruning was added.
- **Storage compatibility:** `.z1` physical blobs have `AG1ZLIB1`, a little-endian
  u64 logical length, and one zlib stream. Schema 2 activates with the transaction
  publishing the first compressed reference; interrupted pre-commit publication
  leaves no committed reference or schema change. Schemas 1 and 2 are readable;
  schema 1 referencing compressed data is rejected. Referenced raw and compressed
  blobs survive GC; unreferenced recognized representations are collectable.
  Raw is the deterministic preference if both exist, and corrupt data never
  silently falls back. `blob_path` rejects compressed-only blobs clearly.
- **Export/recovery/tools:** lossless exports retain exact selected physical
  blobs and source schema, alongside decoded compatibility views. Restore checks
  manifest schema against the restored DB and validates the complete logical
  closure before publication. Cutover export audit accepts the new schema.
  Duration integrity tooling reads both formats without weakening logical hashes.
  Older readers refuse v2; rollback requires a compatible export/new destination,
  not opening the v2 store with an old executable.
- **Archive guidance:** new conversation-custody markers describe retrieval
  without hardcoding a legacy tool name. When `evidence` is selected, stable host
  guidance maps historical artifact read/grep/search wording to that selected
  tool. Legacy frozen compiler outputs and historical evidence stay intact.

The prior timestamp, stable verifier tail, native verification-status guidance,
selected `work` task hint, optional Git-info directory isolation, and renderer
containment fixes remain included.

## Validation

The final pass completed 26 core, 114 storage, 366 native/platform, 58 native
desktop, and 12 duration-tool tests: **576 passing tests**, plus a production
renderer build. Native tests ran serially. The additional four compression tests
passed after the final schema/reference consistency guard was added; these are
repeats, not four new distinct tests. The ignored crash-child helper is actually
spawned by its parent at each of eight physical-publication/transaction boundaries.
No long-horizon run or model request is part of this validation.

`final-validation.json` records exact commands and outcomes. Core frozen oracles,
context history/custody, provider admission, all 266 native session cases, MCP
execution/management, resume/recovery, desktop bridge and migration/downgrade
checks passed. Frontend tests were not repeated because renderer source did not
change in this batch. `final-compression-closure.log` binds the last storage guard.

A retained pre-fix storage library rejects a v2 fixture before changing its DB
or blobs (`old-reader-refusal.json`). SQLite's read-only probe can create WAL/SHM
sidecars; do not claim a byte-identical directory inventory from that inspection.

`custody-benchmark.json` measures one native raw/compressed publication and one
verified page read for each of 24 retained overnight snapshots. The complete
sample stores occupied 47,327,986 versus 12,902,533 physical file bytes (~72.7%
less). Publication medians were 24.81/20.65 ms; verified page medians 1.05/3.16 ms.
Peak extra Rust requested allocation for publication was 2,076/1,948,282 bytes,
and page reads 65,610/141,826 bytes. This bounded sample excludes SQLite C
allocations, allocator retained memory and the model. It is neither an eight-hour
result nor a full-workload growth forecast. Source snapshots were read only.

Initial fixture-only issues are retained or described in E: a benchmark record
used the wrong MIME type and was corrected; an old-reader directory comparison
observed SQLite sidecars. The first core command named a nonexistent target and
was replaced by the correct `task_guidance_contract`. No failed product test is
being labeled passed.

## Packaging / installation

**Installed and verified.** `/opt/ag1/current` resolves to
`/opt/ag1/releases/ebfa976560934e14b2188edd82fe0441`. All 1144 manifest files, sizes, hashes
and modes match; the system launcher, desktop entry, icons and CLI version smoke
passed. `E/installation-systemwide.json` is the activation receipt.

Archive: `.local/p14-packages/c0ef4b6caaff45a5a1444ee57c3de743/ag1-0.1.0-rc-linux-x86_64.tar.gz`.
SHA-256: `36eda9dc72081eda467290bac6783f9a14245835c3ecb75114d458deb07a8fe4`.
Input digest: `496d9db7c6ac0100b65cbef399db37740e15a77c52bed9f56788df864b71c08f`.
Production packaging preceded the separate visual-test helper build. Isolated
installer validation passed before systemwide activation.

**Native MCP upgrade check passed.** In a private authenticated Xvfb/WebKit
fixture, the actual desktop started with a saved helper path to a removed release.
Clicking the MCP sidebar opened the real native general-chat manager with no
NotFound or alerts. No prompts or provider requests were submitted. The private
fixture process group/display were stopped by the harness; the user's running
app was untouched. `E/native-mcp-upgrade.json` and `.png` retain the result.

`E/source-before.tar.gz` / `.json` preserve the 716-file baseline;
`E/changed-files.json` and `E/changes.patch` isolate these 15 changed/new source
files from the pre-existing dirty workspace. `E/completion.json` binds the final
receipts. No commit, reset, stash, or broad cleanup was performed.

## Limits and preserved state

No GPU inference, duration controller, eight-hour run, live GitHub write, external
MCP connection, user app restart, profile retuning, or historical store rewrite.
The user's running desktop must be quit and reopened to execute the new binary.
The protected profile, external process supervisor and runtime capture remain
separate from the upgraded desktop tooling helper.

The diagnosed implementation fixes above are complete. Live cache-hit/latency
and model repetition/quality benefits remain unmeasured. Working-set clone/hash
cost was a profiling hypothesis, not a demonstrated correctness defect; no
speculative weakening of freshness/owner/branch checks was made. Renderer
containment retains loaded DOM/React state and does not establish bounded memory
for indefinitely growing histories. Run9 remains stopped at 2.2389903826 hours,
58 verifier passes and 57/58 semantic reviews passed. No formal P14 or eight-hour
acceptance is claimed. A future live run needs separate authorization.

Continue to preserve the heavily dirty workspace, prior handoffs and receipts.
Build with `.local/p14-target`, six jobs, and the retained Node 24 toolchain.
Do not start further verification or optimization just because a handoff exists.
