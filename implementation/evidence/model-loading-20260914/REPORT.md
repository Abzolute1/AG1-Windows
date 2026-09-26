# Manual local-model loading and composer progress — 14 September 2026

Built and installed systemwide at `/opt/ag1/releases/ee7da595f62146489bc06da06be93e10`. Reopen AG1 to use the new renderer and native command. The running user app was left open.

## Delivered behavior

A compact themed Load control lives inside the composer, above the model selector. It prepares the selected local model without submitting a prompt, then stays as a Loaded indicator. If another model is resident, the control says Switch & load. Remote providers do not get a loading button. Selecting a model or opening AG1 still does not automatically load it.

The same indicator follows automatic first-message loading and manual loading. It shows a progress ring and actual byte-based percentage during model-file verification, and a spinner with elapsed time during loading, initialization and warmup. Verification percentages include validated cached files and never reach 100 on a failed verification. A ready label requires successful native readiness/identity checks. The retained owned model is checked for liveness so a dead process does not retain a Loaded badge.

**Percentage limitation:** current preserved llama.cpp binaries do not expose trustworthy numerical weight-loading progress in their captured startup output. Their default progress dots are not a reliable one-dot-per-percent protocol. The UI therefore does not invent an overall loading percentage. Checking files can show 63%; Loading model and Warming up use indeterminate rings. No backend patch or launch-flag change was made to obtain telemetry.

Manual loading leaves drafts editable and does not consume attachments or send a message. Native preparation takes the same endpoint scheduler permit as generation, so a load from another chat waits behind active work and cannot replace an in-use model. Requests are recorded with durable receipts; replaying a completed command ID does not reload. A failed load proves owned-runtime stop before releasing the scheduler slot, otherwise it leaves custody quarantined. Successful loading leaves the model resident across shared conversations.

## Verification and limits

- Full frontend suite: 276 tests passed, including draft editing during manual loading, no prompt submission, remote/local controls, percentages, queued/stale ready states, retries and busy controls.
- Native: 3 manual-command checks, 9 runtime custody checks, 1 conversation-sharing check and 1 verifier-cache check passed (14 distinct checks). Covers single-GPU admission, receipt replay, sequential reuse/switching, failed-load recovery, byte progress and tamper invalidation.
- Actual App in headless Chromium with a fixture bridge: dark/light themes at 1280 and 560 px; manually exercised Load → checking percentage → loading → Loaded. Draft preserved, exactly one loading command, zero prompts. Private composer screenshots were visually inspected. No user desktop capture or live model run; native behavior is covered by the Rust fixtures, not a fresh GPU exercise or native WebKit visual run.
- Frontend TypeScript/Vite and production Rust builds, bundle audit and isolated installation passed. Vite retained its bundle-size advisory. Clippy completed with surrounding warnings; three style warnings in the new custody code were corrected afterward.
- All 1144 installed payload files matched manifest hashes, sizes and modes; installation receipt matched the archive. CLI version smoke passed. Protected configuration, model profiles and both pinned supervisor hashes stayed unchanged. No llama.cpp source, binary, model weights or tuning was edited.

An initial frontend test invocation used system Node 26 and failed in pre-existing localStorage setup; the repository's Node 24 toolchain passed. Typechecking also caught two unsupported options in the new test before the final build; they were corrected. Those logs are retained with the final passing results.

## Evidence

`changes.patch`, `source-after.json`, `before.tar.gz`, `checks.json`, `visual.json`, theme screenshots, test/build logs, `package-validation.json` and `systemwide-verification.json` retain implementation and verification evidence. Scratch scripts are under `.local/model-loading-20260914/`.

Final package: `.local/p14-packages/fa39532c8a3546558676a893000c71bc/ag1-0.1.0-rc-linux-x86_64.tar.gz`. SHA-256: `23fd6cf236a75988771cc1a1720f5e461be8e3cc63b6a542ca65f205786b267a`.

Broader chat cleanup, model comparisons and duration testing remain outside this focused delivery.
