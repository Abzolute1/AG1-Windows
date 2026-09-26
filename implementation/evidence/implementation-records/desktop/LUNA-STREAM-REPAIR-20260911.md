# Luna stream failure and cancellation repair — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed and verified: `/opt/ag1/releases/f70db4b0812f403fb8494411a0a7942f`.
The frozen previous instance (PID 1950606) was closed with SIGTERM at the user's explicit request; SIGKILL was unnecessary. Open AG1 normally to use the repair. The old turn was not replayed.

## Confirmed failure

The affected saved chat is `chat-29770224cf800e7b0e1a37e0320fa757`. Its latest assistant entry (ordinal 96) recorded `Unavailable("unknown Responses stream event")` after a successful `work` tool call, before any GitHub execution in that turn. The UI projection discarded the error and retained empty thinking. Dropping the failed generation quarantined the hosted endpoint's admission slot indefinitely, preventing settlement, Stop and clean shutdown. This was a harness failure.

The old log did not retain the unrecognized event type. A bounded replay succeeded, so we cannot identify the exact original event. The repair handles additive informational events generally, rather than guessing a single triggering name.

## Changes

- Responses decoding accepts additional informational event types and supports `response.reasoning_text.delta/done`. Empty final summaries no longer erase streamed reasoning text. Explicit provider failures/cancellations, malformed events, unsupported executable items and missing terminal evidence remain failures.
- An abandoned remote generation releases its client concurrency slot after its owning future drops. This does not claim upstream completion or retry the request. Local runtimes still quarantine until positive settlement/stop evidence; their existing cancellation tests remain passing.
- Failed/cancelled assistant messages retain a bounded error in the chat projection, suppress empty thinking, and override stale live thinking display. Original journal entries are unchanged.

## Verification

76 focused automated tests passed: 22 provider wire/HTTP contracts, 10 core provider/admission tests, 1 native error-projection test and 43 frontend tests. Production frontend and native builds passed.

A separate native-host test with a controlled HTTP server passed all four steps: unsupported response item becomes a visible error and idle; next prompt succeeds; cancellation while the next stream remains open returns idle; clean shutdown succeeds before the server finishes. This test used no model inference. Retained source and logs are in the private evidence directory.

A real `gpt-5.6-luna` low-effort test used AG1's native host in an isolated session, with a transport that rejects GitHub mutations: catalog → schema for repos.get → repos.get Abzolute1/AG1. GitHub returned HTTP 200 with outcome read, Luna returned its answer, the native host became idle in 13.28 seconds, and shutdown passed. No original user chat was modified. This is a bounded read-only test, not an exhaustive GitHub API audit.

The failure display was checked at desktop and narrow widths in an owned headless browser. No user desktop capture. All 1,144 installed files, installation receipt and protected configuration/CV/runtime hashes verified. Previous release retained.

## Resume/evidence

Private evidence: `.local/luna-stream-repair-20260911/` (implementation.patch, before/, provider-tests.log, core-tests.log, projection-tests.log, ui-tests.log, native-live.json, native-failure.json, visual.json, package.json, installation-verified.json). Compiled conversation/replay data and private native journals stay in that directory; do not paste them into reports. Temporary probe examples/preview entrypoints were removed from product sources.

Archive SHA-256: `75b591e73ba07dfdf4c60f9a9ed309a1ddcf2e1c5e59f70e767175eaf85b5e3d`.
Input SHA-256: `bdfcb496bebf987232d86c3c90ef5108eb545c7dfdcbb73af71f7db00015fb61`.
Weekly usage snapshot at handoff: 3% remaining. Broader MSP/P14 work remains paused; see tasks/README.md. This repair does not claim all historical backlog is complete.
