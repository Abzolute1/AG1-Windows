# Remaining custody-storage change

Status: design only. No storage format, schema, dependency, retained blob or live database was changed in this follow-up.

The overnight disk-growth finding is still open. The previous lossless-compression sample establishes a possible space saving, not a compatible storage implementation. The validated fixes in this batch do not reduce retained snapshot bytes.

## Proposed implementation boundary

Use a versioned physical zlib representation for newly published large custody artifacts, with logical SHA-256 and logical length unchanged. Keep existing raw blobs readable indefinitely. Start with the three measured snapshot kinds (`context-original-journal`, `runtime-request-input`, `context-original-request`), use a minimum size and retain raw when compression does not save enough bytes. Do not rewrite historical stores as part of publication.

A physical representation must have a distinct directory or suffix, an exact format version/codec, declared logical length, and a bounded single stream. Reject unknown versions, trailing bytes, truncated streams, lengths beyond the existing 256 MiB artifact bound and mismatched logical hashes. A byte-window read may stream the complete bounded logical payload while retaining only the requested window; it must preserve today's complete hash verification rather than verifying a different read than it returns. Cancellation/deadline checks must operate during decode and search, including highly compressible inputs.

Introduce a shared logical blob reader before enabling compressed publication. Route `verify_blob`, `blob_bytes`, `blob_window` and `artifact_catalog::scan_artifact` through it. Preserve access checks and the existing post-scan admission check. Public `Store::blob_path` currently denotes raw bytes, so it cannot quietly return a compressed path. Replace internal raw-path consumers with logical readers, document an explicit raw-export path for external consumers, and make compressed-path requests fail clearly until those callers migrate. Do not create an unbounded persistent raw-materialization cache just to preserve this API.

## Compatibility and durability

- A store that references compressed blobs requires a new storage schema version. The existing binary rejects `user_version > 1`; preserve that refusal. The new reader must accept v1 and v2. Do not silently create references an older binary would interpret as raw.
- Publish compressed physical bytes into staging with the existing file-sync, no-clobber rename and directory-sync sequence. Version activation and metadata references must be acknowledged transactionally. A failed pre-reference publication leaves collectable/quarantined physical data, not a successful artifact receipt.
- Preserve `put_artifact` immutability and logical addressing when the same logical bytes already exist in raw or compressed form. Define a single deterministic read preference and reject a corrupt selected representation instead of silently concealing it with a fallback.
- Export must retain the physical store in its versioned closure for exact recovery and produce exact logical bytes for compatibility views. Update the archive schema check and manifest so `restore_export` validates the new closure before atomic destination publication.
- Update integrity inspection, recovery, quarantine and orphan collection for both physical representations. Existing artifact, archive, memory, metadata, access and lease roots still control retention. Never prune durable history merely to improve a duration score.
- Update duration checks that read `state/store/blobs/<digest>` directly, including `tools/release_duration/workload_checks.py:109`. Keep stopped v1-run evidence readable by its original tools. A rollback from a v2 store needs a verified raw export/new v1 destination; an old binary cannot safely reopen it in place.

## Required validation before enabling writes

Read an unchanged v1 store; round-trip all original bytes and metadata; read windows at/around decode boundaries; search across those boundaries; reject corrupt/truncated/concatenated/oversized inputs; cancel a decode; inject every publication/transaction boundary; export and restore both versions; validate v2 refusal by the old reader; test lease/GC roots and orphan physical data; measure complete-workload space, CPU, latency and peak allocation.

This is a coordinated storage/reader/archive/tool migration. It was deliberately kept out of the prompt/Git/renderer candidate, whose changes need no store migration. No claim of a disk-growth fix is made.
