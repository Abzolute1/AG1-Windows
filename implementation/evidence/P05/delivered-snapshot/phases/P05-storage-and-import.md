# P05 — Transactional storage and lossless import/export

## Objective and reading

Deliver independent Rust storage with explicit crash and evidence guarantees. Read common contract, DECISIONS D03, AG1 session-manager/artifact-store/memory-store, B02/B04, storage tests and importer risks. Scope A38,A40–A44,A46,A47 and A48 storage foundations; I05,I10–I15,I27; L07.

## Entry and allowed work

Requires P02 ports/schema and P01 frozen persistence fixtures. P04 is not a dependency; storage can be built independently. Work in `product/crates/ag1-storage`, `fixtures/storage`, import/export harnesses and disposable databases. Do not read a changing live journal as if it were a snapshot or move real state ownership.

## Work and authority

Implement SQLite journal/branch and typed metadata plus immutable SHA-256 blob/record storage under DECISIONS D03. Preserve original IDs/parent links, custom/model/thinking/compaction/labels, physical versus branch order and unknown payloads. Use explicit storage schema migrations, single-writer lock, idempotent command result records and committed sequence. No live TS data writes or dual-write bridge.

Implement restartable import of JSONL v1–v3, memory v1–v3, artifact records/blobs/catalog/keys/leases and checkpoint/job metadata. Preflight manifests and source hashes; stage into a new destination; retain raw source bytes and unknown fields. Distinguish missing from corrupt; quarantine/report B02 cases without replacing them with empty memory. Imported runtime/process entries are historical only. Export includes original-format compatible records plus lossless extras/manifest; unsafe downgrade explicitly fails.

Publish and sync blob bytes before committing references; recover orphaned unpublished metadata/blobs and rebuild derived indexes. Keep semantic cursor format compatibility through origin key/store identity and access checks. Fork branches share authorized immutable evidence roots; transactionally account for every historical text/binary/capsule/episode reference and old cursor. Resolve B04 without globally relaxing session access. Retention cannot remove a parent blob still reachable by a child. Implement lease TTL/caps and binary bounds with injected clock; text retention preserves reference roots.

## Interfaces and tests

Deliver storage ports for journal transaction/snapshot/branch, artifacts/page/search/lease, memory persistence, migration/export/health. Search may initially follow reference lexical behavior; context selection comes P08. No frontend reuse. Record SQLite/storage dependency notices.

Create `storage_contract` integration suite and property/fuzz cases. Fixtures cover all formats, unknown custom records, corrupt memory, interrupted import repeated twice, fork/reopen/parent retention, authenticated old cursor, forged cross-session cursor, UTF-8/CRLF byte windows, binary tamper and catalog reconstruction. Inject failure before/after blob sync, rename, DB transaction, acknowledgment and importer checkpoint. Assert accepted durable state reconstructs and an unacknowledged external effect is not fabricated as completed.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-storage --test storage_contract
python3 tools/phase_gate.py P05
```

Tests must compare observable journal/evidence outputs against captured fixtures; byte equality is required for immutable payloads, not for the new physical database representation.

## Measurement and completion

Measure durable ack, 10k/100k-entry resume/import, artifact page/search cold/warm and growing catalogs. Compare equivalent durability contracts; TS unsynced append time is not a valid direct target for synced commits. Record this justified baseline distinction. Exit requires B02/B04 corrections, lossless import/export, crash/corruption recovery, cursor authorization and schema/version documentation. Benchmark growth must not sacrifice evidence integrity.

## Rollback and handoff

Discard an unaccepted destination; originals and executing TS sessions stay unchanged. P07 receives real transactions/blob/cursor APIs, P06 settings storage and P13 a tested migration/export protocol. Ownership remains shadow-only until P13.
