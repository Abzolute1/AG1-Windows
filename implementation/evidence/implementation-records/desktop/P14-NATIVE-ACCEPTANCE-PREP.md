# P14 repaired-candidate acceptance preparation

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Date: 2026-09-09. Status: renderer fixture, isolated diagnostic native build, startup smoke and controlled native approval workflows passed. The user approved the six-file recoverable cleanup and isolated build with “yeah continue”, then continued controlled native testing. Installation, live inference and duration testing remain unauthorized. The original assessment/proposal below is retained as historical context; the completion records supersede its pending status.

## Completed without native inference

`tools/probes/p14_approval_ui_audit.mjs` launches its own loopback Vite server and headless Chromium against current renderer source. The bridge is entirely in memory, permits only the expected fixture approval commands, and rejects unexpected calls. Browser HTTP requests outside the owned server are blocked. Both owned processes are closed in `finally`.

The passing run is `.local/p14-live-fixes/approval-browser-fibZEZ/result.json`, with `review-before-expiry.png`, `review-expired.png` and `review-small.png` in that directory. It checks literal preview rendering, Enter prevention, automatic expiry with unchanged snapshots, fresh replacement requests, exact-binding approval, close-to-deny, and viewport fit at 1440×900 and 900×600. The expired and small screenshots were visually inspected. No unexpected bridge calls, page errors or blocked external requests occurred in the passing run. The first probe attempt exposed an incomplete fixture event-unsubscribe stub; that stub was corrected, not production code. Its earlier screenshots do not constitute a passing receipt.

This is current-source Chromium evidence, **not** native Tauri/WebKit or installed-application acceptance, and not a live model test. It supplements the 67 passing frontend tests recorded in the repair receipt.

## Native setup assessment before approval

- The private-display executable `.local/p11/xvfb/usr/bin/Xvfb` exists and its dynamic dependencies resolve. No private display was started here. A future native fixture should allocate an unused display, use a private Xauthority cookie, disable TCP listening, and terminate only its owned processes.
- Existing `.local/p14-target/debug/ag1-desktop` and `release/ag1-desktop` were built earlier on September 9, before the repair follow-ups. They cannot certify the repaired source. Do not use the installed pre-repair application as a substitute.
- The historical `tools/client_visual.py` expects old UI labels and binaries under protected `product/target`. It must not be run unchanged. A new candidate harness must use the agreed `.local/p14-target` binary, isolated project/configuration/state and controlled providers first. The native `visual-test` broker is feature-gated and not production UI automation parity.
- About 759 MB is free. The `.local/p14-target` debug and release trees occupy roughly 34 GB and 2 GB respectively; their incremental directories are essentially empty. A native link/rebuild with this headroom is not being attempted. The exact additional space required depends on selected build features; freeing a fixed amount is not a guarantee of build completion.

## Original recoverable space reclamation proposal — subsequently approved

Archive and verify the following six older test executables before removing their uncompressed copies. All are regular files with one hard link. Their combined uncompressed length is 2,486,396,200 bytes (about 2.32 GiB); net reclaimed space will be smaller by the archive sizes. They are test outputs, not installed binaries, model files, source or transcript artifacts. Different hashes may represent different build configurations; they are not asserted to be interchangeable.

Every path below is relative to `/home/alex/Projects/AG1/.local/p14-target/debug/deps/`.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `session_contract-2367cdaaafbbf56c` | 379344480 | `8cd914fb1639a4efd5bd3028e218756a90a0941ff276a50c11ebe7e789fa729d` |
| `session_contract-af973554555dffc9` | 376445088 | `4c2f5e2c3bcaea6c1cc2f9030116fe21f8de7007fc4cffd68c016287d43d07c7` |
| `bridge_contract-3fcdee38334372f6` | 430367688 | `5b5b48a083de811bcf684ad1dcd60e5a4b14a826480d6806ebc5dc40b9ab61c7` |
| `bridge_contract-21a285987525d368` | 431553496 | `f8e9542557f374cf76e8a314371649555721492d91a96ab0b0930ee499c465e0` |
| `cutover_contract-2109d3367c488b4f` | 433655624 | `04a736d35aeab34dbfa4f93b5827afb76ecaee57cb457871c155c0f99467737c` |
| `cutover_contract-dee7ebb496b21aca` | 435029824 | `f5bbd598ae5e23d21e48b272d9e691e432096a736f131a8499c212dabf612435` |

If authorized, process one exact file at a time: recheck identity/hash and that it is not executing; create a compressed archive in a new owned preservation directory; verify decompression reproduces the original hash and retain executable mode; only then remove that exact uncompressed file. Retain a restoration manifest and never overwrite a different future binary during restoration. Stop if identities changed or space is insufficient. Do not use `cargo clean`, recursive directory deletion, or remove dependency metadata/libraries wholesale. No cleanup has been performed yet.

Keep the current session test binary `session_contract-d5ef95905dfd106b`, all desktop/process-host binaries, and newer bridge/cutover binaries untouched by this proposal. Never touch `product/target`, `/opt/ag1`, model profiles/weights, the capture configuration, audit project, frozen transcripts or historical receipts.

## Following the space decision

1. Recheck headroom and agree on the isolated native candidate build configuration. Rust commands must use `CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target`, `CARGO_INCREMENTAL=0` and offline dependencies. Do not overwrite the installed release.
2. Exercise the rebuilt native UI with isolated state and a controlled provider on its private display. Preserve screenshots, native result receipts, build identity and honest engine/provider labels. This does not authorize live inference.
3. Request separate explicit authorization before any real-model UI test. Use the short acceptance script in the frozen handoff; do not infer permission for an eight-hour run, installation or release replacement.

## Approved cleanup and diagnostic build completed

Only the six listed uncompressed test binaries were removed after verifying their compressed archives reproduce the exact bytes. Restoration instructions, hashes and sizes are in `.local/p14-live-fixes/test-binary-archives-I9CBe8/RESTORE.md`. Archives total 580,539,204 bytes; net reclaimed space is 1,905,856,996 bytes (about 1.78 GiB). The first process-use check used unsupported `fuser --` syntax and failed to stop the first three deletions; all three already had verified archives. Work was paused and the check corrected to fail closed before processing the other three. The restoration manifest records this limitation explicitly.

The user separately deleted a game during the build; the filesystem now has about 142 GB available. That large increase is not attributed to the six-file cleanup. No further cleanup is needed.

Candidate: `.local/p14-live-fixes/native-candidate-08EBCR/`. The renderer was built into that directory, leaving existing `product/apps/desktop/dist` unchanged. The offline, locked Rust build used `.local/p14-target`, `CARGO_INCREMENTAL=0`, debug profile and `visual-test` plus default `custom-protocol`. This is a diagnostic candidate, not a production release package. Commands, binary identities and logs are recorded in the candidate README. Pre-build renderer/docs and debug desktop/process-host bytes remain in `native-build-before-20260909.tar.gz` and `native-debug-binaries-before-20260909.tar.gz` under `.local/p14-live-fixes/`.

## Native startup evidence and remaining step

`tools/probes/p14_native_startup_smoke.mjs` passed against the candidate in actual Linux Tauri/WebKit, using the feature-gated visual-test broker, private authenticated Xvfb with TCP disabled, separate AG1 state and separate XDG data/config/cache directories. It verified the current embedded renderer entry, home screen, safe approval default, fixture model metadata and Settings. Both screenshots in `native-candidate-08EBCR/startup-pYpWS1/` were visually inspected; `result.json` records zero provider requests and zero submitted prompts. Owned desktop/display processes were stopped.

Two earlier attempts failed the fixture-model assertion. Waiting longer did not fix it: isolating AG1 state alone left WebKit's saved model preference shared. The probe now isolates XDG storage as well; production code and the user's saved preferences were not edited to make this pass. Those attempts are not passing acceptance evidence.

This checks startup/navigation only, not native approval workflows, model behavior, live reliability, installation, physical display performance or full release acceptance. Next: bounded native UI/approval workflow regressions using an isolated fake provider and this diagnostic candidate. Any real-model test, eight-hour run or release replacement still requires separate explicit authorization. Protected release/process-host/profile/capture identities remain unchanged.

## Controlled native approval workflows completed

`tools/probes/p14_native_approval_audit.mjs` now reuses the startup runner and exercises the actual native host and embedded WebKit renderer. Final passing receipt: `.local/p14-live-fixes/native-candidate-08EBCR/approve-17bSfK/result.json`; native tool results reaching the scripted provider are in `provider-results.json`. Four fixture prompts produce exactly eight loopback requests, one tool call and one terminal tool result per case. No real model is contacted.

- Allow writes exactly the reviewed bytes; no write occurs before review.
- Deny and closing the dialog preserve the original bytes and report policy rejection to the provider.
- An external fixture edit during review survives Allow; native revalidation rejects the stale write and tells the provider to read/propose a fresh edit.
- Altered argument hashes and replayed consumed bindings are rejected specifically as `invalid_approval`, using fresh command-envelope revisions. These two adversarial checks invoke native IPC directly; ordinary review choices use the actual UI buttons.
- Literal script-like content is displayed without creating script elements. The real React key handler cancels a synthetic Enter event and leaves the native request pending. This is not proof of physical-keyboard default behavior. Dialog bounds pass at 1440×900 and 900×600; both sizes were visually inspected in the preceding passing run `approve-k36Bpg`, and the final 900×600 screenshot was inspected again.

The workflow process runs in a private PID/mount namespace with the host root read-only, only its owned fixture directory writable, private temporary/browser state, and a shadow native authority registry. It does not register the fixture in the user's real registry. Xvfb uses its own authenticated display with TCP disabled. The application, display and loopback server are stopped at completion.

One intermediate run (`approve-iZtE9U`) failed because the driver tried the next Send button before the renderer reflected native completion. The driver now waits boundedly for that existing UI action to become ready; it does not force state or bypass disabled controls. The whole suite was rerun and passed, then rerun with stricter exact-rejection and single-result assertions. No production repair was needed. The shared startup-only CLI was rerun successfully (`startup-SDtBjj`, zero prompts/requests), and all 11 approval-expiry React regressions passed again.

Next bounded step: native editor sizing/file-refresh checks, including preserving unsaved edits, using disposable files and controlled providers. Native wall-clock expiry/cancellation, actual model/cache behavior, physical input/display performance and release acceptance are not established by this approval suite. Live inference, installation and eight-hour testing remain separate authorization boundaries.

## Editor follow-up — 2026-09-10

Current diagnostic candidate is now `.local/p14-live-fixes/native-candidate-SKS6XR/`, with desktop SHA-256 `d72e998f25667217882e7e872e29892b1b9a4ed15344223ce1c8955b711df024` and embedded entry `/assets/index-EX6QTWms.js`. Its README records commands and logs. The old `08EBCR` candidate is preserved, not replaced.

Native testing exposed and then verified fixes for same-file reopen leaving Monaco's old model displayed and long filenames clipping the dirty marker. Both failures were reproduced before source changes; details and preservation archives are in the repair receipt. All 69 frontend tests and typecheck pass. The editor fixture passes at large/small sizes and sidebar resize, including automatic/manual file refresh without dirty-buffer loss, rejected conflicting save, external-byte preservation, and Discard/reopen/successful-save recovery. Passing receipt and inspected screenshots: `native-candidate-SKS6XR/editor-T4WBjK/` (zero prompts/requests). Prior native approval workflows also pass on this exact candidate: `approve-JxpbTd/` (scripted loopback provider only).

Next: replace raw file-conflict JSON with a clear recovery explanation while retaining revision checks and unsaved contents. This is not installed-release acceptance; no installation, live inference or duration run has been authorized or started. Native wall-clock expiry/cancellation, physical input/performance and actual model/cache behavior remain outside these receipts.

## Conflict messaging completed — 2026-09-10

Current candidate: `.local/p14-live-fixes/native-candidate-DQhmfU/`, desktop SHA-256 `4174f914774bb323007df481bbad56aba7f55992226063e75e5b9ff19179c69b`, embedded entry `/assets/index-wqSugjQ_.js`. Previous candidates are retained. The native editor-save conflict now explains safe recovery and preserved unsaved text; unrelated errors and revision checks are unchanged. The two initially failing message tests now pass, together with all 78 frontend tests and typecheck.

Native receipts on this exact build: `editor-Og4WxR/` (zero prompts/requests; message, dirty buffer, external bytes and recovery checked; small-window conflict screenshot inspected) and `approve-zKcI6M/` (prior approval suite rerun with scripted loopback provider only). Build/test logs and commands are in the candidate README. Existing installation, model profiles, capture and protected process host are unchanged.

Current confirmed repair backlog: **0**. Remaining work is validation, with possible new findings: next controlled native chat-switch/cancellation checks; separately scoped native expiry, actual model/cache, physical-input/performance and release acceptance. Zero confirmed backlog is not a bug-free or release-ready claim. No live-model test, eight-hour run or installation is authorized by this record.

## Chat/queue validation and active-cancellation limit — 2026-09-10

The unchanged `DQhmfU` candidate passes native chat switching, separate drafts/history, truthful local-slot queueing, cancellation of queued B without dispatching it or stopping A, Settings during A's stream, and a fresh B reply after A finishes normally. Final result/provider receipts: `native-candidate-DQhmfU/lifecycle-T2KT0v/`; queued and recovered screenshots were inspected. Three fixture prompts, two loopback requests, no model/tool effects. Existing exact-scope React cancellation test was rerun and passed. No production edits or rebuild occurred.

Active-stream cancellation/recovery is **not accepted**: the attempted `lifecycle-BNXO1j` case closed HTTP but timed out waiting for native completion, retaining Stopping/Conflict. The simple provider fixture lacks owned-runtime cancellation evidence; releasing a local slot on disconnect would violate the scheduler's safeguard. Before-first-output cancellation was not reached. Failed attempts and the initial incorrect parallel-slot assumption are documented in the repair receipt, not hidden by the narrower passing suite.

Next is a controlled runtime-aware fixture with real owned-process shutdown evidence, followed by those two cancellation/recovery checks. Do not weaken quarantine or manufacture an acknowledgement. Confirmed production-fix backlog remains zero pending further findings; active cancellation, native expiry, real-model/cache, physical-input/performance and release acceptance still have unverified areas. No live inference, installation or eight-hour run is authorized or started.

## Runtime-aware cancellation completed — 2026-09-10

The scoped owned-runtime fixture gap is now covered, not a live-model acceptance. A visual-test-only controller owns a fixed-response loopback child server and proves cancellation by kill/wait on its actual Child handle. It cannot choose a model/external executable; ordinary builds exclude this path. Production quarantine/cancellation logic is unchanged.

Isolated candidate `native-candidate-NFhH7R` (desktop SHA-256 `b4ab427904dc59c5ae321c3c5a0911b18d41ab6cd3ef90d87611192b7eef8bd3`) passes the new native owned-cancellation probe twice, `owned-MAUuCR` and `owned-mvdvwQ`: Stop during streaming and after dispatch before headers/output, native and renderer settlement, preserved draft, and successful same-chat request on a replacement child. Each run has four fixed prompts/four requests and actual namespace-local child reap receipts. Selected screenshots visually inspected. No physical-input/GPU/model cancellation claim.

Prior native queue/navigation, approval, and editor suites pass on the same binary (`lifecycle-axs5Ot`, `approve-fC7SrK`, `editor-E1eMB5`). Frontend 78/78, desktop library 6/6, and selected backend contracts 20/20 pass; normal-feature desktop check passes. Renderer/typecheck/native build pass. Candidate README and the main repair receipt retain exact commands, identities, limitations, and the two corrected fixture-driver failures (trailing context message selection and renderer polling timing). The earlier no-runtime-proof HTTP failure remains recorded as failed, not retroactively accepted.

Pre-edit archive: `runtime-fixture-before-20260910.tar.gz`. Previous work/candidates, existing dist, installed release and protected process-host/profile/capture identities remain unchanged. All owned test processes stopped. No installation, live inference, eight-hour run or user-project modification.

Confirmed production-fix backlog remains **0**, subject to future findings. Next safe validation is native wall-clock approval expiry/recovery using a controlled provider. Live-model/cache, physical-input/performance, cross-platform and release acceptance remain separate; prohibited runs still require explicit authorization.

## User-requested systemwide build — 2026-09-10

The user explicitly authorized building/installing for their own testing before further work. Production release package `5da43cbf768141d181b1563ffc120482` built and passed fresh isolated installer verification. Archive SHA-256: `5ddb724ea395814646396f03e05d02e26b36c75dc3623d5b5c27d9a07b2fb439`; desktop SHA-256: `ea8bb4a870f9acbb62a636543ccaa6a14681b1804acfc8887e8a3b0b84f8e92f`. Diagnostic hooks are absent. Frontend 78/78 and package-selection tests 4/4 pass; renderer/typecheck/native build, package identity/payload verification, native dependency audit and CLI smoke pass. This remains an unsigned local test candidate, not release acceptance.

**Installation is pending user authentication, not completed.** `sudo -n` required a password before the systemwide installer ran. `/opt/ag1/current` remains `releases/2ebdc491fb814b959b79378f24c62e01`; no pending operation exists and existing installation hashes are unchanged. User must authenticate in their own terminal; no password should be provided in chat.

Full build/verification/failed-install logs, installer command, old generated-output archive and preservation details: `.local/p14-live-fixes/systemwide-pYIL32/README.md`. Normal renderer dist was rebuilt with its prior bytes archived. Existing source work, user configuration, bound supervisor, model profiles/capture and installed application remain preserved. No live inference or duration run was authorized or started. Pause further validation while the user installs/tests.

### Systemwide installation completed after authentication — 2026-09-10

The user provided authentication; the exact verified archive installed successfully as `/opt/ag1/releases/a459017613ba4372a84f2dafece16dc8`, now selected by `/opt/ag1/current`. Previous release `2ebdc491fb814b959b79378f24c62e01` remains available and is bound in the installer receipt. No pending operation remains. Installed manifest and every payload hash/size/mode match the verified package; CLI smoke passes, systemwide launcher/menu/icons are published. Full receipt: `.local/p14-live-fixes/systemwide-pYIL32/installed-verification.json`.

User configuration, bound supervisor, model profiles and capture hashes are unchanged. The application was not launched; no inference or duration test ran. The user can now open AG1 normally to test the repaired production build. Further validation remains paused for that feedback; installation does not constitute full release acceptance.
