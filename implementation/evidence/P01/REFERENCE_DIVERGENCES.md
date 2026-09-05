# Reference-suite investigation

The expanded catalog initially exposed five failures and 15 skipped tests. Four failures came from the initial isolation harness: its child-process wrapper did not preserve Node's custom promisify result shape; real fixture loopback servers were blocked; npm attempted registry resolution instead of failing offline. The harness now preserves execFile's `{stdout, stderr}`, permits connections only to servers created by that test process and sets npm offline with no fetch retries. A focused rerun passed 153 of 154 tests; only the prefix assertion remained failing.

## R01 — Old prefix test conflates request projection with persistent history

At the pinned source, `test/prefix-stability.test.ts` expects every request message to remain byte-identical across user turns. It fails because the request-local working-state block is removed from the previous user message when a new user turn becomes current. The original message text is unchanged.

`src/core/agent-session.ts:_installCacheFriendlyWorkingStateTransform` explicitly attaches that block to the latest user message and never writes it back to canonical messages/journal. Its intended cache invariant concerns stability **within the current tool loop**, while each new user instruction must receive current task state. Freezing the old request projection would violate current-state admission.

The new executable oracle `tools/probes/capture_session.mjs` records eight full request projections across successful and failing tool scenarios. It independently asserts one carrier on the latest user message, original objective preserved, latest instruction updated, stable system prompt, stable prefix within the successful tool loop, no projection in canonical messages/journal, one tool execution and correct failure outcome. It does not strip working state from captured fixtures. Timestamp/workspace-path normalization is explicit.

R01 is a stale fixture contract, not a confirmed new runtime defect. The original failing test stays unchanged and its exact failure remains recorded. A reviewed catalog exception may accept only this known assertion and requires the replacement oracle; unrelated failures still fail the gate. P08/P10 must preserve both request-local-state correctness and the scoped cache invariant.

The 15 source skips are 13 credential-dependent fork/tree integration cases and two Windows-only pipe-close cases. They are pending live/platform gates, never passing tests. Other offline branch/summary/storage/fork probes execute in P01. Their exact names and source hashes are captured in `fixtures/catalog-exceptions.json`; no blanket skip permission exists.
