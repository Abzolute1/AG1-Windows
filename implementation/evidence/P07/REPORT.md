# P07 task, memory and byte-consistent evidence

The gate executed 87 Rust contracts, the 94-case unchanged SDK state corpus, strict validator tests and matched-input measurements. See [completion authority](completion.json), [coverage](coverage.json) and [the state API guide](delivered-snapshot/product/crates/ag1-core/STATE.md).

B03 tests force mutations after reading and before validation: same-size edits, replacement/rename, symlink swaps and root replacement cannot produce mismatched displayed bytes and digest. BOM, CRLF, Unicode, clipping, binary and over-limit cases remain explicit. P05 storage corruption, fork closure and crash-recovery contracts were rerun.

The phase keeps objective and steering distinct, validates claim/plan transitions, persists independent observation records, retains archived memory and enforces requester authorization for evidence pages/cursors. No executing desktop or inference authority changed.

- Shadow task/memory/evidence services only; the usable desktop remains TS-owned until P13. No model, GPU, desktop or project runtime was launched.
- Linux source capability/race and storage tests executed. Windows/macOS/WASM and actual native visual/runtime integration remain later gates.
- Source reads are limited to 8 MiB and reread full bytes for freshness. Only parsed references are cached; metadata-only shortcuts are intentionally excluded.
- Memory selection preserves the reference soft character budget, including one bounded first entry that may exceed it. The sixteen-entry cap stays hard. The shadow catalog is limited to 10000 retained memories and 4 MiB documents.
- Task and observation text caps preserve complete Unicode scalar values at UTF-16 boundaries. Runtime observation timestamps require valid RFC3339; informal date strings are not accepted as timestamps.
- Historical invalid/duplicate task identity and corrupt memory fail visibly and retain the P05 original archive. Dangling executive links cannot acquire authority during migration.
- A26 planning state remains distinct; interactive plan/question/mode policy is P10/P12. A39 prompt insertion is P08 and A45 actual native visual capture/attachment is P11.
- TS benchmarks hash the same warm-cache files but its cached validator has weaker metadata assumptions and its source revision API does not capture display text. No equivalent-correctness speed or desktop footprint claim is made.
- The initial state oracle is retained for audit. The v2 oracle explicitly parses persisted task state before mutation, matching the Rust boundary; both were generated through the unchanged SDK.
- Existing tuned llama.cpp binaries, libraries, weights and profiles were verified unchanged. No model settings or process ownership were transferred.
