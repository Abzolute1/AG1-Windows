# AG1 storage — P05

`ag1-storage` implements independent Rust persistence. It is not connected to the executing desktop worker. TypeScript continues to own live sessions, memory, tasks, artifacts and runtime requests until P13. No automatic import, dual writes, model calls, PID adoption or llama.cpp changes occur here.

## API and authority

`Store::open` takes a new private destination or a recognized existing Rust storage database. An unrelated nonempty directory, symlink, unknown database identity/schema or second writer is rejected. Linux opening is validated; other platforms return `Unsupported` pending native storage tests. The destination root is private to the owning user.

Core ports are in `ag1_core::storage`: journal snapshot/fork, memory and metadata CAS. The existing `JournalPort` and `ArtifactPort` are implemented as well. Storage methods perform blocking disk work; a composition host must invoke them on a bounded blocking executor, rather than a renderer or inference lock. Database handles never cross a core port.

| API | Contract |
| --- | --- |
| `create_session`, `append_batch`, `snapshot`, `branch` | Preserve physical order separately from parent-linked branch order. Commit journal entries, revision, global sequence and command result atomically. |
| `record_intent`, `command_result`, `settle_intent` | Persist intent before an external action. Restart changes unresolved intents to uncertain; only explicit reconciliation evidence settles them. A command ID cannot be reused with different inputs. |
| `register_origin`, `put_artifact`, `artifact_record`, `page` | Preserve immutable record and payload hashes; enforce origin ownership and requesting-session access. Bytes served are collected during the same hash pass used to verify integrity. |
| `cursor`, `decode_cursor`, `read_cursor` | Authenticate reference `ac1` and `ac2` cursors with retained origin keys and explicit branch authorization. Generate compatible `ac2` cursors. |
| `search` | Literal case-insensitive text scan, with bounded result count and line previews. This is not P08 ranking/context selection. |
| `lease`, `leases`, `release_verified_leases` | Injected clock, 16 active leases, TTL clamped to one minute–24 hours. Preserve imported raw lease identity/unknown fields and verification-release flags. |
| `memory`, `save_memory`, `metadata`, `save_metadata` | Missing differs from corrupt. Compare expected digest before replacement, retaining previous versions as history. Imported process/job metadata is historical and cannot be overwritten through ordinary settings APIs. |
| `import_frozen`, `import_status`, `original_bytes` | Stage and verify a complete immutable manifest, checkpoint retained files, then transactionally publish typed state. Failed imports remain inspectable but unavailable as executing sessions. |
| `export`, `restore_export`, `reconcile`, `collect_orphans` | Atomic no-replace export/restore, hash verification, derived catalog rebuild and conservative orphan collection. |

## Schema and durability

Storage schema 1 has application ID `1095184723`. New databases install `schema.sql` transactionally and set SQLite `user_version=1`. Unknown versions are rejected before writer initialization. Further migrations must be explicit and tested; no speculative forward migration runs.

SQLite uses WAL, `synchronous=FULL`, foreign keys, a busy timeout and a process-wide advisory writer lock. A mutex serializes this adapter's transactions. Blob publication writes a private temporary file, syncs it, renames without clobbering an existing hash, and syncs the containing directory before any referencing database commit. Filesystem/device sync behavior remains a platform assumption; process-exit tests do not substitute for destructive power-loss testing. See [SQLite synchronization](https://www.sqlite.org/pragma.html#pragma_synchronous) and [WAL constraints](https://www.sqlite.org/wal.html).

The database contains sessions/entries, commands, committed sequence, artifact occurrence records, authorization roots, origins, leases, memory, typed metadata, import checkpoints, raw archive references and a rebuildable catalog. `blobs/<sha256>` holds immutable bytes and original artifact record bytes. `staging` holds unpublished bytes; restart moves leftovers to `quarantine`. Hash mismatches fail explicitly. Catalog rows rebuild from authoritative records.

Unacknowledged committed work is discoverable by command ID; retry does not append it twice. An accepted external-action intent is not proof of completion. The crash harness records an acknowledgement only after a successful API return and verifies reconstruction after abrupt exit on either side of that boundary.

## Frozen imports

`FrozenManifest` version 1 requires `frozen: true`, each file's relative path, size, SHA-256 and kind, with session/origin descriptors where applicable. It must enumerate the complete source directory. Symlinks, traversal, duplicate paths, unexpected files and hash/identity drift fail. The importer rechecks the snapshot before final publication. This detects changes; it does not turn a live, changing journal into a frozen snapshot. A caller must first obtain an immutable capture.

Imports target an empty destination. The only resumable existing import is the same manifest identity. Each retained file and its checkpoint commit together. All typed state publishes in the final transaction, so repeated interruption cannot expose a partial session. Recognizable artifact IDs and cursors are checked before publication; opaque encodings remain protected by the complete source archive and conservative evidence roots.

JSONL versions 1–3 preserve raw bytes, original IDs where present, parent links, unknown/custom payloads, model/thinking changes, compaction, branch summaries and labels. V1 had no IDs; deterministic source-hash/physical-position IDs make restart repeatable. V1 compaction indices become IDs and the V2 `hookMessage` role becomes `custom`, matching the captured SDK migration. A nominal V1 record that already supplied IDs is rejected for explicit migration instead of silently replacing them.

Memory versions 1–3 retain active/archive entries and unknown fields. Invalid JSON, empty files, malformed entries and invalid schemas become `corrupt` with raw bytes and diagnostics retained. Missing memory returns `None`. An ordinary memory write cannot replace quarantined input with an empty store.

Artifact records, blobs, catalogs/indexes, cursor keys, leases, checkpoint/job/runtime metadata and arbitrary extra files are retained. Legacy stores without any cursor key receive a new origin key; missing authenticated-cursor closure cannot publish. Source visual-retention records can identify already evicted binary evidence: its occurrence metadata remains available, byte reads explicitly report missing, and no payload is fabricated.

## Forks, retention and export

Forks copy the selected journal branch while sharing immutable artifact bytes through explicit authorization roots. All evidence authorized to that parent is retained, including text, binary, capsules, episodes and old cursors. This can retain more evidence than the selected branch strictly needs; it never broadens access to unrelated sessions. Parent archival hides the active session while preserving historical roots. Garbage collection removes only orphaned, unpublished blobs. Automatic pruning of archived/import roots is deliberately disabled.

Binary creation admits at most 32 authorized occurrences and 128 MiB per session, with an 8 MiB per-artifact limit and reference image-envelope bounds. When roots prevent safe retention, admission fails rather than deleting referenced evidence. Imported history can exceed current admission limits and remains retained; subsequent creation still enforces the limit.

Lossless export includes a consistent SQLite backup, every referenced blob, original source files, original-format journal/artifact/key/lease views and a hash manifest. The destination is published atomically without replacing a directory created concurrently. Restore verifies the manifest and the reconstructed database/blob graph before publishing a fresh destination.

`require_compatible=true` rejects corrupt memory, unsupported journal entry types, inherited/ambiguous origin keys, uncertain external intents and already evicted evidence. The full lossless archive remains available for those cases. A successful compatible export has been reopened by the original SDK, including artifact reads, cursor authentication and leases. It is a compatibility archive, not automatic permission to replace live TS state.

## Bounds, tests and measurements

Frozen imports allow at most 100,000 files, 4 GiB total, 256 MiB per file, 100,000 entries per journal and 4 MiB per entry. Journal batches allow 100,000 records/128 MiB total. Larger inputs fail explicitly. Pages request at most 32 KiB; matching reference UTF-8 behavior may skip continuation bytes at the start and extend by up to three bytes to finish the final character. Search returns at most 100 matches with 1,024-character line previews. Full text materialization is bounded to 256 MiB.

From the workspace root:

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-storage --test storage_contract
python3 tools/phase_gate.py P05
```

The gate runs only P05 tests plus reference comparisons, lint/format/build, preservation and licensing checks. `storage-probe` is a disposable-store benchmark/crash harness; it is not included in the desktop launcher. The gate captures 10k/100k journal imports and full resumes, 100 durable acknowledgement samples, catalog growth and warm/reopened artifact reads. “Cold” here means a reopened adapter with kernel caches retained. Unsynced TS append latency is not an equivalent durability comparison.

The fixture corpus derives from immutable P01 source and journal captures. Model/runtime actions remain zero. All L01–L08 preservation requirements remain binding. P06 can consume metadata CAS, P07 can consume journal/evidence ports, and P13 alone owns state authority cutover. Rollback discards only an unaccepted private destination and preserves its source snapshots and export.
