# Continued overnight fixes: request-custody allocations — 2026-09-13

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user said “good, continue” after the MCP/compression candidate was installed.
This resumes the remaining CPU-only performance follow-up. The eight-hour run
remains prohibited. Keep checkpoint verification small and use a larger final
pass. Earlier installed fixes and their receipts remain described in
`HANDOFF-OVERNIGHT-FIXRUN-20260913.md`.

Evidence: `implementation/evidence/overnight-request-custody-20260913/` (E).

## Change and evidence

Native context preparation built an owned `serde_json::Value` containing a clone
of the complete message graph merely to serialize each request-custody envelope.
The ordinary input envelope also remained in scope across journal custody and
compilation. The new `session/context/custody.rs` serializer borrows fields and
writes directly to the serialized string. The temporary owned graph is gone.
This applies to ordinary input, delegated/subagent input and compiled-request
custody. Existing context reduction, raw journal custody, access checks, cache
freshness, source rechecks and owner/branch identities are unchanged.

Fields retain the existing sorted JSON object order. The distinction between a
missing `memoryRevision` on delegated input and an explicit null on ordinary
input is preserved. Version, system prompt, tools, model, thinking, messages,
stats, nested unknown fields and escaping retain the same serialized bytes and
therefore the same logical artifact addresses. This is an allocation change,
not another storage format migration.

The focused regression compares all three production envelope variants to the
previous native expressions, including null/missing revisions, nested values,
Unicode and escaped text. It passed. The retained-snapshot benchmark compares the
actual old expression with the new production serializer on eight original
runtime-request-input artifacts, three pairs per artifact. Both outputs equal the
original retained bytes in every pair.

Median serialization time: **4.408 ms → 1.515 ms**. Median peak extra requested
Rust allocation: **7,592,571 → 2,097,152 bytes** (~72% lower). Median allocation
calls: **46,403.5 → 15**. Total requested allocation bytes: **9,951,843 →
4,194,176**. These are serialization-only measurements, not whole-request latency,
retained heap or model performance measurements. SQLite C allocation and allocator
retained memory are excluded. Historical snapshot files were read only.

`checkpoint.log`, `serialization-benchmark.json`, `benchmark-summary.json`, and
`serialization-benchmark.rs` retain the evidence. Initial standalone-probe setup
selected incompatible cached crate variants; a private Cargo probe with the
workspace's pinned dependency versions resolved that setup issue. No product
correctness failure was waived. The probe does not modify the product lockfile.

## Final validation and installation

The final native regression passed **278 tests**: all 266 session cases, nine
working-set cases and three client-guidance cases. `final-native.log` and
`final-validation.json` record that pass. The checkpoint was one byte-exact
serializer regression; no broad intermediate verification was run.

**Installed and verified.** `/opt/ag1/current` resolves to
`/opt/ag1/releases/82240dc2b29043c792454dd1a49d97ce`. All 1144 package manifest files,
sizes, hashes and modes match. The launcher, desktop entry, icons and CLI smoke
passed; protected model profiles, runtime capture and external supervisor hashes
are unchanged. The user's app was not restarted.

Archive: `.local/p14-packages/fdcbc9e71640467598d74933b51070a8/ag1-0.1.0-rc-linux-x86_64.tar.gz`.
SHA-256: `11692589d7a2ec60aec1d08b6a911651a3b4d46609ed19f902869bf0f89a3031`.
Input digest: `df65e562d9adb98324217f0153454d34f4ce2ad0dfbb06873c82208087d59f5b`.
`package.json`, `package-validation.json` and `installation-systemwide.json`
record the production build, isolated installer validation and actual activation.
Renderer/MCP UI verification was not repeated for this serializer-only change;
that upgrade flow passed in the previous candidate. Quit and reopen AG1 to run
the updated binary.

## Scope and remaining limits

Only `session/context.rs` and the new `session/context/custody.rs` change in this
follow-up. E preserves the original caller, exact patch and changed-file hashes.
The heavily dirty workspace is retained; no commit, reset or cleanup.

No model inference, GPU use, external MCP connection, duration controller, live
GitHub write, user app restart or profile changes. Prior disk compression, MCP
upgrade repair, verifier/task guidance, Git-info handling, timestamp and renderer
containment fixes remain included. Working-set prefix hashes remain necessary
freshness checks; their CPU optimization is still only a profiling hypothesis.
Renderer containment retains the loaded DOM/React state. No indefinite-memory,
live cache-hit, model-quality, P14 or eight-hour acceptance claim is made.

Use `.local/p14-target`, six jobs and the retained Node 24 toolchain. A further
change needs an observed cost/defect or explicit new task, not more repeated
verification of this completed candidate.
