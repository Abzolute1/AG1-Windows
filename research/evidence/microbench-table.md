| Operation (synthetic workload) | Samples | p50 ms | p95 ms |
|---|---:|---:|---:|
| `artifact_put_988000_bytes_deduplicated_blob_new_record` | 30 | 20.9524 | 21.8689 |
| `artifact_get` | 30 | 0.0094 | 0.0182 |
| `artifact_read_8192_bytes` | 30 | 0.0400 | 0.0669 |
| `artifact_search_marker` | 30 | 17.6843 | 19.2429 |
| `session_append_custom_async` | 30 | 0.2034 | 0.2716 |
| `session_reopen` | 30 | 0.1228 | 0.2104 |
| `workspace_index_uncached_120_files` | 30 | 42.0994 | 42.8661 |
| `source_context_exact_symbol` | 30 | 18.9131 | 20.1514 |
| `source_revision_hash` | 30 | 0.0610 | 0.0763 |
| `source_revision_warm_validation` | 30 | 0.0393 | 0.0494 |
| `git_status` | 30 | 3.3154 | 3.4940 |
| `git_diff` | 30 | 2.8923 | 3.0238 |
| `scheduler_uncontended` | 100 | 0.0007 | 0.0044 |
| `read_tool_dispatch` | 30 | 0.7985 | 1.2912 |
| `context_compile_201_messages_2550000_text_chars` | 10 | 83.3646 | 91.5104 |
| `context_compile_already_compiled` | 30 | 0.1722 | 0.1872 |
| `event_stream_10000_events` | 10 | 1.7310 | 4.6694 |
