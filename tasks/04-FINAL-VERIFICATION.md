# Combined regression, visual and bounded live verification

Status: planned final feature pass. The completed GitHub audit is existing evidence, not a substitute for the final combined artifact/MCP/chat audit. Read the [task index and shared rules](README.md). This file does not resume testing or inference.

The [completed fixed-scenario exercise](../implementation/evidence/scenario-validation-20260913/REPORT.md) supplies reusable native/live evidence for selected harness fixes. It does not cover this entire combined feature audit; reuse applicable results and check actual remaining gaps.

## Prerequisites

- [ ] Reconcile completion and explicit limits from [artifact/review closeout](01-ARTIFACT-REVIEW.md), [MCP](02-MCP-INTEGRATION.md) and [chat cleanup](03-CHAT-CLEANUP.md).
- [ ] Identify the exact current source/candidate, owned runtime custody, granted scratch projects and preserved configuration. Read existing evidence before deciding what requires a fresh run.
- [ ] Preserve the seven documented frozen comparison failures and prior harness/model/product failures. Do not alter expected outputs or overwrite evidence-linked binaries to obtain a passing report.

## Automated and native checks

- [ ] Run frontend tests/typecheck, relevant native suites, affected security-boundary tests and relevant frozen comparisons. Record actual commands, terminal outcomes, counts and failure classifications.
- [ ] Run build, licence/provenance and packaging checks for changed dependencies and the final distribution.
- [ ] Build a fresh isolated native candidate with its own embedded renderer and profile; record candidate/helper hashes and verify protected files before/after.
- [ ] Check wide/narrow windows, themes, panel resizing/fullscreen/close, keyboard/focus, long names/diffs/logs, images/source/download, unsaved edits, conversation switch/reopen and approval/cancel/recovery. Inspect actual screenshots as well as assertions.
- [ ] Obtain current native non-file output-card and large-diff coverage from task 1. Confirm that result entry points preserve chat drafts and review scope.
- [ ] Exercise GitHub and MCP through owned protocol fixtures, including authentication/permission failures, cancellation, revoked or changed identities, malformed responses and uncertain writes. Reuse GitHub's current probes and extend only when touched behavior requires it.
- [ ] Confirm owned app/display/server/helper cleanup and absence of late results leaking into another conversation or identity.

## Bounded real-model harness

The former plan authorized bounded real-model cases for this final feature audit. They remain paused until this task is resumed. They do not authorize eight-hour/24-hour runs, real GitHub mutations, paid CI, or private data disclosure to external MCP servers.

- [ ] Use isolated scratch projects and the preserved configured model/runtime. Exercise create/edit/review, generation/inspection of a simple SVG, an owned MCP tool/resource, a GitHub fixture, approve/deny/cancel and same/new-conversation follow-up.
- [ ] Record model and candidate identity, actual submitted prompts, tool calls, permissions, terminal results, timing, screenshots and cleanup. Keep fixed-response fixture results separate from real inference.
- [ ] Inspect the final conversation for clear primary results without debug nesting. Compare model claims with actual file/tool/native outcomes.
- [ ] For each new failure, reproduce it, classify harness/model/UI/environment cause, fix genuine in-scope defects, then rerun the exact failed case and appropriate adjacent regressions. Do not weaken custody or rewrite frozen evidence to make a harness pass.
- [ ] State physical-input/display coverage accurately. DOM input in private WebKit is not physical keyboard control or refresh-rate/performance evidence.

## Final installed checkpoint and handoff

- [ ] Build the normal production distribution after implementation and relevant checks pass; keep diagnostic hooks out of the package.
- [ ] Verify package identity and startup, install systemwide according to the user's checkpoint instruction, then verify installed bytes/size/mode, receipt, launcher, absence of pending installer state and preserved runtime/model configuration.
- [ ] Update the task index with completed requirements, exact evidence, remaining limitations and any outstanding external prerequisites. Give the user a short remaining-work/effort summary at the installed checkpoint.
- [ ] Stop after the authorized feature scope is complete. Do not automatically begin the broader P14 release programme or a duration workload.

## Acceptance boundary

The finished artifact/review, MCP, GitHub and chat flows have current automated/native evidence and the authorized bounded real-model coverage, with truthful limits and a verified installed production checkpoint. Unavailable required coverage remains explicitly incomplete.

This is separate from the full [P14 release gate](../phases/P14-release-and-long-horizon-gates.md), its matched model/performance programme, Windows/macOS evidence, signing and elapsed-duration acceptance. Their partial-run history and authorization holds remain in the [release resume record](../implementation/evidence/implementation-records/phases/P14-RESUME-HANDOFF.md).
