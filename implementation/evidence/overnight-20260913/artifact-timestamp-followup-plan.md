# Follow-up after frozen run9

Status: design only; no product source changed during the duration run.

The live observation is bound in `run9-artifact-timestamp-finding.json`.
`FileRunner` is cached per session and its construction timestamp is reused for
new tool artifacts in later turns. The existing per-turn `ToolJournal` already
exposes the appropriate logical timestamp. Approval/repetition expiry uses a
separate clock and is not implicated by this observation.

## Bounded change

In `session/files.rs::execute_one`, initialize the `ContextStore` with
`current.selection.journal.as_ref().map_or(self.timestamp.as_str(), |journal|
journal.timestamp())`. Keep the explicit runner timestamp fallback for direct
invocations without a journal. Do not rewrite any existing metadata or change
artifact identity, access checks, TTL clocks, or recovery timestamps.

Other call sites reviewed: post-execution stores for repetition/completion/
controlled observation read durable artifacts; job notices and quiescence have
no current ToolJournal, while extension discovery has separate callback
semantics. Do not mechanically alter these unrelated paths. A future change to
background-event time requires its own provenance design and evidence.

## Regression and validation milestone

Use the existing native `session_contract` fixture and `native_adapters_configured`
with a real shell and one shared runner across two `NativeSession::run` calls.
The loopback provider should issue distinct shell calls and outputs in each
turn, followed by stop responses. Give each `TurnInput` a distinct command and
explicit timestamp later than runner construction. Read the resulting
`terminal-raw-output` records through the authorized Store catalog. Assert both
timestamps match their respective turns, producers match actual call IDs, the
first record is unchanged after the second turn, and newest-first ordering is
correct. This must fail against the frozen source and pass with the fix.

Retain the existing direct-runner contracts as fallback coverage. Run the full
session contract target serially after the focused red/green check; broaden to
the relevant storage/tool targets if source scope grows. Previously passed
unchanged frontend checks need not be repeated. Repackage/install the follow-up
with exact receipts and clearly distinguish it from the eight-hour candidate.
