# P06 providers and preserved llama.cpp

The executed gate covers all ten pinned provider families, 70 request captures, six preserved local payloads, nine exact launch plans, bounded FIFO admission, quarantine after uncertain cancellation, durable credential refresh, and owned Linux runtime lifecycle. The [completion record](completion.json) contains executed commands and immutable delivered hashes.

No desktop or live model was launched. The existing tuned binaries, libraries, weights and profile bytes were verified unchanged. Rust adapters remain shadow services until P13.

The [API guide](delivered-snapshot/product/crates/ag1-platform/PROVIDERS.md) describes provider, isolated-call, credential and process boundaries. [Coverage](coverage.json), [Rust measurements](rust-measurements.json) and [TS measurements](ts-measurements.json) record the actual run. Measurements share identical synthetic output bytes; setup costs and runtime dependencies differ.

- Rust remains a shadow provider/runtime adapter. The usable desktop retains TS scheduler and runtime authority until P13.
- All provider traffic is synthetic or loopback fixture traffic. Cloud credential canaries, interactive OAuth login and live per-model quality/TTFT/prefill/decode/VRAM are pending P13/P14; none is counted passed.
- Owned process and storage contracts ran on Linux. Windows/macOS/WASM execution and release packaging remain pending.
- Credential refresh persists through the private P05 store. This is not an OS keychain or encrypted-at-rest claim. Command-derived credentials require the later P09 broker.
- Google OAuth refresh at the source pin contains placeholder client credentials and explicitly returns unavailable; valid existing tokens and project envelopes remain supported.
- Benchmarks share exact fake bytes, fragment rate and queue depth. TS includes SDK request/Promise setup; Rust measures framing/domain reduction and admission. RSS includes different runtime dependencies. No desktop or live-model speed comparison is claimed.
- Pure versioned tuning proposals do not apply settings. No tuning, live model launch, profile rewrite, backend rebuild, port eviction or process ownership adoption occurred.
- Native HTTP uses a declared AG1 Rust user agent and host headers; Node SDK telemetry/browser-permission headers are intentionally omitted. Dynamic extension providers and user-facing auth integration belong to P09/P12.
