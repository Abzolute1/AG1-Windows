# Cross-language protocol fixtures

`roundtrip.json` and `roundtrip.ts` are generated from real Serde DTO instances by `protocol-codegen`. `codec-check.ts` compiles against generated TypeScript, exercises negative type assertions, and checks exact absent/null/custom/decimal preservation. `tools/check_protocol.mjs` validates the resulting data against generated JSON Schema and writes it back for the Rust decoder to compare.

The fake host scenarios and controlled queue/dispatch measurements run in `product/crates/ag1-protocol/tests/protocol_contract.rs`. They cover subscription barriers, replay retention, stale and out-of-order traffic, repeated commands, approvals, cancellation, binary pages, bounded admission and parser fuzz inputs. A 30-second subscriber pause is simulated with 3,000 ticks; it is not a 30-second live desktop benchmark.

P01 fixtures remain unchanged. These new fixtures do not execute original tools, launch llama.cpp or modify persistent AG1 data.
