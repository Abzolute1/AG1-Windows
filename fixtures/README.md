# Executable migration fixtures

`manifest.json` maps every A01–A85 capability to pinned reference test sources and, after execution, named observed scenarios. High-risk active capabilities have a successful scenario and a failure/edge scenario. These are executable test scenarios with source and assertion evidence; they are not misrepresented as complete wire traces. `catalog-selection.json` is the exact selection. `catalog-exceptions.json` names credential/platform skips and the stale prefix assertion, with source hashes and reasons; none counts as a passing test.

`traces/session.json` captures complete request projections, emitted events, canonical messages and journal snapshots from real AgentSession with a scripted provider, for tool success and failure. It preserves the current working-state blocks. `llama/launch-plans.json` captures nine real resolver calls with fake spawn/health, covering three configured models and override/tier boundaries. `llama/request-payloads.json` captures six real provider request bodies, stopping at the payload hook before network access. Fixture environment/hardware differs deliberately from live deployment and is recorded explicitly.

The observed runtime identity is in `implementation/evidence/P01/runtime-manifest.json`: all 30 selected GGUF files (including the Flash model's 28 shards), static loader candidates and available external-process library mappings. Private launch environment and process argv stay under ignored `.local/runtime` with restrictive permissions. No model process is launched, queried by HTTP, stopped or retuned.

`child-inventory.json` refines provider, tool, persistence, client and extension families. A named child requirement is not automatically executed evidence; its linked source and pending owner remain explicit. P06/P09/P12 complete provider/tool/client-specific cross-language and live validation. P01's immutable TS reference and captured fixtures provide the input oracles.

Schemas describe the capture envelopes, not the future Rust product protocol. The runner checks actual report counts/statuses/source hashes; generated captures assert their own behavioral invariants. P02 defines the separate production wire DTO/schema.

Reproduce with `python3 tools/bootstrap_reference.py --rebuild`, `python3 tools/runtime_manifest.py`, then `python3 tools/phase_gate.py P01`. Runtime hashing reuses only a matching device/inode/size/mtime/ctime cache; `--force-hash` rereads all bytes. No filesystem hash cache is a substitute for live model-quality acceptance.
