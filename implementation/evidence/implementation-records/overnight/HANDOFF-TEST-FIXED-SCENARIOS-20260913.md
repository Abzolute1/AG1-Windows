# Next instance: test the fixed harness scenarios and report back

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

## User request and intended outcome

The user asked: “Write a handoff that asks the next instance to test these scenarios, then report back when it's finished. Explain why these scenarios are to be tested as well.”

**Your task is to execute the bounded scenario checks below, review the actual results, and report back to the user when finished.** This document is an execution handoff, not evidence that these new checks have run. Its preparation did not start tests or a background run.

The scenarios come from the old-versus-new walkthrough given to the user: open MCP, inspect a valid Git repository, repair a parser, recover from incomplete work, recognize the exact verifier, change the requirement and verifier, and continue with substantial history and archived evidence. Include the related task-state and timestamp fixes.

Why test them now: the individual fixes have regression coverage and short live evidence, but the user needs confidence that the described behavior holds together in the installed harness. Verify the practical claims, especially completion, evidence freshness and responsiveness. The walkthrough was illustrative; do not require every historical failure to occur in one old-build session or present it as a captured transcript.

## Starting point

- Workspace: `/home/alex/Projects/AG1`.
- Read [the final implementation handoff](HANDOFF-OVERNIGHT-FINAL-FIXES-20260913.md) first. It links earlier fixes, protected materials and evidence.
- Recorded installed release: `/opt/ag1/releases/ef76f6b16b06446e8df7167f3b696bdf`, selected by `/opt/ag1/current`.
- Final archive: `.local/p14-packages/3d05b768dacd4a5c99f8595055dc57a2/ag1-0.1.0-rc-linux-x86_64.tar.gz`.
- Archive SHA-256: `d9fb0374c4dbf5a0077310dc1537bbd6293d50637622d813f41eb90370274e8a`.
- Source input digest: `42f029da2bc0b9000fb7b0586f1d742964b616cb6d7899c45bae4d0e2f01a548`.
- Installation receipt: `implementation/evidence/overnight-final-followup-20260913/installation-systemwide.json`.

Resolve the current release and record the actual tested binary/manifest hashes. If it has changed, identify the difference before attributing results to this build. Do not downgrade the systemwide installation to obtain a baseline.

Existing evidence includes 280 native tests before the last work-facade clarification, the complete six-test native-surface target after that clarification, three live repairs with passing original verifiers, and a successful final-binary task/evidence follow-up. Some earlier attempts failed; their failures remain recorded. This was not an eight-hour acceptance run or one clean all-workspace test invocation.

## Constraints and working method

- **No eight-hour run and no duration controller.** Run9 remains stopped; never resume, overwrite or relabel it.
- Short GPU validation is already authorized. Do not ask for that permission again. Preserve the existing model configuration and tuning.
- Discord may be closed if needed; **do not close Firefox**. Inspect current process identities rather than reusing old PIDs. Do not forcibly restart the user's AG1 app.
- Use isolated workspaces, state/config copies, owned runtime processes and a fresh evidence directory. Do not connect to external MCP servers or write to GitHub for these checks.
- Preserve the dirty repository, earlier handoffs, historical stores, original verifiers and failed results. No reset, stash, cleanup of unrelated files or unsolicited commits.
- Keep checkpoint verification small and do one appropriate combined final review. Do not rerun every previous suite, installation hash check or benchmark repeatedly. Reuse hash-bound historical baseline evidence when it answers the comparison; run an old binary only in isolation when new comparative evidence is needed.
- For live work, use the existing short-probe pattern: up to 240 seconds per prompt and a 900-second overall exercise deadline, with explicit generation limits. A deadline is a stop condition, not a reason to retry until green. The focused follow-up can be shorter. Do not change model context, offload or batch settings to make a test pass.
- Preserve `/home/alex/.ag1/agent/llamacpp-model-profiles.json`, `.local/p13-runtime-capture-20260907/configuration.json`, and the external supervisor `product/target/release/ag1-process-host`. Their expected hashes are in the prior installation verifier. Never print private launch inputs, credentials or complete private configs.
- If builds are necessary, use `.local/p14-target`, six jobs and `.local/toolchains/node-v24.14.0-linux-x64/bin`. Keep production packages separate from instrumented visual-test helpers.

If a check exposes a regression in these fixes, retain the failure, diagnose and correct it within the existing fix scope, and verify the affected path. Do not expand into speculative optimization. Identify precisely which final build was tested and installed if changes are made. Record any genuine blocker and the evidence needed to resolve it; do not silently skip a scenario.

## Scenario checks and why they matter

### 1. Open MCP after an upgrade

**Why:** the user previously got `NotFound` immediately on entering MCP, before selecting a server or Discover. A backend-only success would miss the actual failure path.

In an isolated native desktop fixture, load a saved tooling-helper path referring to a removed release from the same AG1 installation. Activate the real MCP sidebar/page. Check that the manager opens without `NotFound` and that the helper resolves to the current packaged sibling. Preserve custom helper paths and the separate model supervisor. No external server connection is required.

Evidence: capture the actual page/result and relevant helper identity. Reuse `implementation/evidence/overnight-fixrun-20260913/native-mcp-upgrade.mjs` and its harness after inspecting their assumptions and choosing fresh paths.

### 2. Inspect a valid repository with optional Git directories absent

**Why:** ordinary Git accepts these repositories; harness inspection previously rejected them. Inspection must also remain read-only and maintain its isolation.

Use disposable ordinary and linked-worktree repositories with the optional info directories absent. Exercise the native read-only inspection path and compare host metadata/file inventories before and after. Require successful inspection without creating directories in the repository. Retain a focused symlink/race rejection check from the existing coverage rather than bypassing protections or repeating the entire Git suite.

### 3. Repair the parser and review the actual completion claim

**Why:** a “tests passed” sentence does not establish that the model edited the correct file, used the original verifier, handled the specified boundaries, or completed its native command.

Start with `export function normalize(value) { return Number(value); }` in a disposable `normalize.mjs`. The contract is string input, a base-10 integer prefix with leading whitespace/optional sign and trailing text, invalid/nonpositive results mapped to zero, and positive results capped at 7. Allow changes only to that source file.

Use the preserved 12-case fixture: `"4 sec"`, `"99"`, `"bad"`, `"bad4"`, `"-2"`, `"0"`, `""`, `"Infinity"`, `"4.9"`, `"  +2tail"`, `"1e3"`, `"0x10"`. Expected results are respectively `min(4, cap)`, `cap`, 0, 0, 0, 0, 0, 0, `min(4, cap)`, 2, 1, 0. Keep verifier bytes unchanged after fixture creation.

Ask the model to inspect, repair, run `node --test verify-a.test.mjs`, and report observed results. Independently check the original verifier, changed-file inventory, tool receipts and final answer. A native completed receipt, a verifier pass, and a supported final claim are separate requirements. Retain initial coding mistakes and their repairs.

### 4. Continue after an incomplete turn; preserve real cancellation

**Why:** dropping a successful turn previously cancelled the parent command, preventing print-mode recovery. A normal successful repair might never exercise that branch.

Use the deterministic wire-level fixture to force an initial incomplete/planning-only response followed by recovery. Require the command to continue and settle completed when satisfied. Also retain the negative path: unverified recovery exhaustion must fail, and an actual cancellation/abandoned future must still stop work. Use the targeted tests in `session_commands`/`session_print`; do not depend on a live model randomly choosing an incomplete answer.

### 5. Distinguish a wrapped command from the exact verifier

**Why:** `cd … && node --test …` could print a passing result while failing the strict verifier match. Vague feedback encouraged repetition. Accepting arbitrary shell wrappers would weaken the verification boundary.

Deliberately run a wrapped verifier in the focused native fixture. Require verification to remain pending and feedback to explain the mismatch. Then run the exact recorded command: require an acknowledged pass and expiry of the mismatch explanation. In live tasks, count actual verifier calls and whether intervening mutations/failures justified them. Repeating a successful check is an observation to review, not automatically a correctness failure. Never suppress checks to achieve a desired count.

### 6. Change the requirement and verifier in the same conversation

**Why:** the prior changing verifier text invalidated the early prompt cache; a previous pass could also be mistaken for proof of the new source revision.

After scenario 3, deliberately regress the source to the original `Number(value)` implementation, change the README cap to 5, and select `node --test verify-b.test.mjs`. Continue the same native session. Require a fresh repair and verifier pass for the new revision, with the previous verifier file untouched.

Capture first-request system/prefix identities, provider-reported cached/uncached usage and request timing for each task. Confirm one current transient verifier contract, no stale contract and no contract persisted into the journal. Historical comparison: the old sample changed system text at character 2336 and reported zero cached tokens on task 2; the fixed sample retained identical system prompts and reused 4,424 tokens. Treat those numbers as observations, not thresholds or a promised speedup. Different generated transcripts invalidate a simple overall-latency comparison.

### 7. Maintain the objective and retrieve archived evidence

**Why:** the final preferred-tool exercise originally used `work update` incorrectly, leaving the old objective in place. Retrieval must also recover the actual stored output, not merely repeat remembered text.

With preferred tools selected, ask `work` to change objective/nextAction. Require a successful `set` receipt and verify the resulting task snapshot. In a focused negative case, an `update` without an entry ID must give the corrective action and make no mutation; valid entry updates must retain their meaning.

Emit the 2,200-line fixture once. Use `evidence` with its returned immutable artifact ID and a focused query to retrieve `ARCHIVE_RULE_CURRENT`: “a passing check applies to the checked source revision; later edits require a new check.” Confirm returned provenance, bytes and the final explanation. Use `inspect` for current file reads. A recorded decision is not a substitute for a successful objective update. Do not repeat source edits/verifiers just to exercise these metadata and retrieval actions.

### 8. Continue with substantial history and retained evidence

**Why:** the user-facing claims concern responsiveness and disk/CPU overhead as sessions grow. Correctness of a tiny conversation alone cannot establish these effects.

Use the retained bounded fixtures instead of a prolonged run:

- **Renderer:** use the same loaded-history fixture for any before/after comparison. Check loading earlier messages, scroll anchoring, expanded thinking, streaming/appending and loaded-text search/reveal. Measure mount/append work once with modest repeats; distinguish Chromium from native WebKit evidence. Containment retains DOM/React state and does not prove bounded memory.
- **Request preparation:** use the existing retained-snapshot probes to compare exact outputs and CPU/temporary-allocation cost. Preserve full-prefix, owner/branch and source-freshness checks. Report the measured stage, not an inferred whole-app percentage improvement.
- **Custody:** on an isolated store, check that eligible new snapshots use less physical space while logical hashes/content, retrieval and archive round-trip remain exact. Include a focused corrupt/truncated-input rejection. Preserve old raw-store compatibility and do not rewrite historical user stores. Compression reduces growth; it does not eliminate it.

Existing renderer fixtures are under `implementation/evidence/overnight-resume-20260913/`. Custody evidence is under `overnight-fixrun-20260913/`; serialization evidence is under `overnight-request-custody-20260913/`; working-set probes are under `overnight-final-followup-20260913/`. Reuse applicable existing results rather than rebuilding every benchmark by default. Clearly mark what was newly exercised versus reviewed from retained evidence.

### 9. Attribute tool evidence to the correct time

**Why:** incorrect timestamps can make an older action look current and undermine review of a long session.

Check that outputs from later tool executions carry their actual turn/execution timing rather than the initial prompt's timestamp. Prefer the existing controlled-clock regression plus inspection of the new live trace over arbitrary waits or brittle timing thresholds.

## Harness reuse and evidence discipline

Inspect `implementation/evidence/overnight-final-followup-20260913/live_checks_v4.py`, `live_work_followup.py`, `live-review.json`, and `completion.json` before adapting them. **Do not run the old scripts unchanged:** they contain fixed output paths, fixture-specific IDs and, for the follow-up, references to an existing private session. Use new destinations and newly observed IDs. The shared `retain()` helper is append-only; status updates need distinct filenames or an explicitly managed latest-status file. A previous probe failed by trying to overwrite an append-only receipt.

Capture enough evidence to distinguish model choice, native command status, tool execution, verifier result and claim correctness. Preserve unsuccessful attempts and review final answers manually. Do not auto-pass based on exit code or rewrite an earlier partial result when a follow-up later succeeds. Close only the clients, displays and processes owned by this exercise.

## Completion and report to the user

Create a new report under the fresh evidence directory with one row per scenario:

`Scenario | newly tested / retained evidence reviewed | PASS / FAIL / BLOCKED / NOT RUN | observed result | evidence | remaining limitation`

The final report must identify the actual build/model/settings, native/UI versus synthetic coverage, unchanged original verifiers, command outcomes, final-claim review, observed cache/CPU/disk/rendering measurements, any newly fixed defects, and cleanup status. Distinguish checkpoint counts from the combined final review; do not imply one all-green workspace invocation or cross-model/endurance validation.

When finished, **send the user a self-contained final reply** stating which scenarios passed, what failed or remains untested, whether any code/build changed, what the findings mean for using the harness, and a link to the report. Explicitly confirm that no eight-hour run was started and Firefox was left open. If something is blocked, explain the concrete blocker instead of claiming all scenarios passed. Do not stop at writing another plan or leave the report only on disk.
