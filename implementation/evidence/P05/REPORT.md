# P05 transactional storage and import/export

The P05 gate executed 28 storage contracts, six gate-validator tests, frozen SDK comparisons and original-SDK export reopening. [completion.json](completion.json) is the authoritative status and records commands, input hashes and delivered artifacts.

The new ag1-storage crate owns independent SQLite journal/branch metadata, command intent/results, committed sequence, memory and settings CAS, immutable SHA-256 evidence, origin keys, leases and restartable imports. Core ports expose domain values without database handles. No executing desktop authority changed.

B02 corruption is retained with diagnostics, never normalized to empty memory. B04 forks keep their authorized historical evidence and old cursors after parent archival/reopen. Cross-session access and tampered cursors fail. Raw source bytes and unknown payloads survive repeated export/restore. Imported process records cannot grant process ownership.

Twenty-eight contracts include 19 abrupt child exits across publication/transaction/acknowledgement boundaries, interrupted import replay, 512 deterministic malformed-journal mutations, a 1000-entry randomized branch graph, UTF-8/CRLF windows, binary tamper/eviction, catalog reconstruction, lease caps, unsafe downgrade rejection, and a destination publication race. The frozen SDK corpus covers 38 files, four journals, three memory versions, 55 byte windows and both cursor formats. [The export check](export-reference.log) reopens a Rust export through the unchanged SDK and verifies artifacts, cursors and leases.

## Measurements

| Entries | Durable import | Reopen + full resume | Branch projection |
| --- | --- | --- | --- |
| 10,000 | 132.57 ms | 8.46 ms | 7.40 ms |
| 100,000 | 1481.45 ms | 83.58 ms | 93.49 ms |

The 100 synced acknowledgement samples measured p50 0.088 ms and p95 0.102 ms. Catalog measurements cover 10/100/1000 records, warm page/search and reopened adapter reads. See [raw measurements](measurements.json) for every sample and limitations.

WAL with synchronous=FULL is configured and blob/directory synchronization precedes reference commits. This follows [SQLite’s synchronization contract](https://www.sqlite.org/pragma.html#pragma_synchronous); it cannot guarantee that every device truthfully persists a sync. The unsynced TS append baseline is therefore not used as a direct speed target.

## Dependencies, bounds and handoff

rusqlite 0.40.2 uses bundled SQLite 3.53.2 for storage only. Exact dependency licenses, the SQLite source notice and omitted WASM VFS license at its published VCS pin are retained under product/licenses and [storage-dependencies.json](storage-dependencies.json). Rust storage does not replace or rebuild any llama.cpp runtime.

See [the storage API/schema guide](../../../product/crates/ag1-storage/README.md) and [limitations.json](limitations.json). Retention preserves archive roots conservatively. P06 receives settings CAS and P07 receives journal/blob/cursor transactions; P13 retains migration authority. Rollback discards an unaccepted private destination while keeping original snapshots and the independent lossless export. P06 has not started.
