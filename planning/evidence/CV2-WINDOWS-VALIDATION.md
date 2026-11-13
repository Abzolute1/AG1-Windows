# CV2 Windows validation

This record covers a Windows-native source validation of commit `30c5d26` in
the separate `AG1-Windows-work` clone. The original AG1W working tree and all
local model/server state were left untouched.

## Repairs

- `context/signals.rs` accepts a rooted virtual workspace path on Windows.
  Rust distinguishes a drive-qualified absolute path from a rooted path, while
  the context fixtures use POSIX-style roots.
- `resources/package_lifecycle.rs` keeps persisted package roots in the POSIX
  spelling used by the package protocol after Windows path normalization.
- `context/projections.rs` removes an extra verifier instruction that was not
  part of the pinned context addendum.
- `clients/cli/tests/client_contract.rs` waits for the particular prompt or
  fork operation before inspecting state, avoiding a race with another idle
  operation.
- Retained web fixtures now record the DOM parser's decoded, unique-result
  behavior. Their legacy replay checks protocol fields while the newer,
  dedicated quality suites validate ranking, DOM extraction, and recovery
  presentation details.

## Verified on Windows

All commands used `cargo test --offline --locked` with generated build output
on `G:\AG1W-build-cache`.

- `-p ag1-core --test context_contract`: 12 passed.
- `-p ag1-core --test package_lifecycle_contract`: 5 passed.
- `-p ag1-core --test native_prompt_contract`: 6 passed.
- `-p ag1-core --test search_quality`: 11 passed, 1 intentionally ignored
  live-evidence replay.
- `-p ag1-core --test tool_contract`: 28 passed.
- Full `-p ag1-core` offline test suite: passed, with only its one explicit
  live-evidence replay ignored.
- `-p ag1-platform --lib`: 53 passed.
- `-p ag1-desktop --lib`: 31 passed.
- Desktop TypeScript typecheck, production web build, and Rust format check:
  passed.

## Known boundaries

- `ag1-cli` cannot build for the Windows target yet because `ag1-platform`
  still imports Linux-only modules and `std::os::unix` APIs. This is native-port
  work, not a CV2 failure.
- `cargo clippy -p ag1-core --lib -- -D warnings` reports three pre-existing
  style warnings outside this patch. No unrelated formatting or lint cleanup
  was applied.

No model, llama.cpp server, Turtle, or AG1W UI was started during this source
validation.
