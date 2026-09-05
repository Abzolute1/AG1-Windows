# AG1 Rust foundation

P02 introduces `ag1-core` and `ag1-protocol`. The desktop directory contains generated TypeScript only. There is no desktop application, model runner, real tool executor or data migration here yet.

`ag1-core` has distinct validated workspace/session/branch/operation/artifact IDs, domain errors, session views, asynchronous adapter ports and a bounded endpoint waiting queue. It has no external dependencies. The future platform and storage adapters implement these ports without exposing GUI or database handles in domain types.

`ag1-protocol` owns Serde DTOs, JSON Schema and TypeScript generation. Its `fake::FakeHost` is a deterministic, single-owner, in-memory contract harness. Tests control the clock and call `step()` to complete fixture operations. Unsupported operation families return a typed unavailable error.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-protocol --test protocol_contract
python3 tools/phase_gate.py P02
```

Run these from the AG1 workspace root. The gate checks formatting, Clippy, behavioral tests, generated drift, TS compilation, JSON Schema acceptance/rejection, TS-to-Rust round trips, licenses and protected reference/runtime hashes. Cargo dependencies are locked; the gate runs Cargo offline after the initial build has populated the local cache. TypeScript 5.9.3 and Ajv 8.18.0 come from the isolated P01 reference for development checks only.

All three existing tuned llama.cpp builds, their per-model selection, arguments, libraries, profiles and weights must be reused. This foundation neither launches nor updates them. The binding requirements remain L01–L08 in `../planning/LLAMA_CPP_PRESERVATION.md`. Later adapters must match the frozen launch/request fixtures before acquiring runtime authority.
