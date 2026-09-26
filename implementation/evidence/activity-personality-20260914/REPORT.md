# AG1 activity mark and whimsical verbs — 14 September 2026

Built and installed systemwide at `/opt/ag1/releases/ff218780597542c58cec2e5c1ba4a025`. Reopen AG1 to load the new renderer. The running user app was left open; llama.cpp, model files and tuning were not modified.

## Delivered behavior

The live status at the bottom of the working area now has an AG1 mark: three short rounded strokes around an open center. A travelling highlight and subtle radial movement share a 2.4-second cycle with a shimmer through the activity phrase. Both use the app's foreground/muted-foreground colors, without a green accent. The mark has a fixed footprint, and animation does not require JavaScript frame timers, model calls or native changes.

The vocabulary contains **254 entries / 251 distinct phrases across nine activity pools**: thinking 34, reading 32, searching 30, writing 28, editing 32, building 24, testing 26, executing 24 and general preparation 24. It retains playful choices such as Noodling, Scheming, Spelunking, Fossicking, Tinkering, Conjuring, Sending it, Cooking and Vibing. Testing includes Probing, Poking, Bug-hunting and Trying it out. Words implying fabricated answers, successful verification or an unrelated deployment/Git operation were excluded.

Selection is stable for the same turn/operation/category, independent of elapsed time and streamed token updates. A new operation or category can select a new phrase. Native status plus the first unresolved tool call selects the pool; historical completed searches and user-prompt keywords do not. Shell classification is deliberately conservative, so an `echo` mentioning tests cannot claim to be running tests.

The bottom status remains visible beneath current streamed thinking or answer text and beneath expanded or collapsed work history. The top “Working for…” timer, tool-history labels and collapsible thinking content retain their existing functions. Literal loading, verification, queueing, awaiting-model-output, stopping, approval and error messages take precedence. In particular, the existing “Waiting for model” state is not presented as measured prefill progress. Approval/queued/error marks are static, and reduced motion or forced colors disable both new animations while keeping the text readable.

## Easy iteration

[ACTIVITY-DESIGN.md](../../../product/apps/desktop/ACTIVITY-DESIGN.md) documents the independent editing surfaces:

- `ActivityMark.tsx`: SVG geometry.
- `activityMotion.css`: motion, theme and size variables, with reduced-motion/forced-color behavior.
- `workingVerbs.ts`: vocabulary and stable selection.
- `ActivityStatus.tsx`: activity mapping and accessible status.

The development-only `/activity-preview.html` page runs on the desktop Vite server. It offers light/dark themes, a cycle-speed slider, Pause/Resume, Next verbs and a simulated conversation-phase selector using the real `TurnView`. It needs no native connection, model or GPU. The studio is excluded from production assets. [Exclusion check](preview-excluded.json).

## Verification

- **284 frontend tests passed**, including vocabulary variation/stability, literal-status precedence, conservative tool classification, ignoring completed calls, streaming/stop transitions and existing conversation/history checks. [Full suite](frontend-all.log).
- TypeScript/Vite and production Rust build, bundle audit and isolated installer passed. [Frontend build](frontend-build-final.log), [package validation](package-validation.json).
- Private Chromium inspection of the actual components passed in dark/light themes and a 390px viewport. Frozen animation frames at 200ms and 1050ms had different stroke/highlight states while preserving the mark's bounding box and selected phrase. Live status stayed last in the turn; collapsing work history did not hide it. Reduced motion stopped both new animations and retained visible text. [Measurements](visual.json), [dark studio](dark-studio.png), [light studio](light-studio.png), [narrow studio](narrow-studio.png). These checks used simulated native phases and did not capture the user's screen or run a model.
- All **1,144 installed payload files** matched manifest hashes, sizes and modes; installation receipt and CLI smoke passed. All **114 protected runtime/configuration hashes** stayed unchanged, and task source hashes matched retained final inputs. [Systemwide verification](systemwide-verification.json).

An initial new test incorrectly required an explicitly present `kind: undefined` property instead of checking an absent optional property; its assertion was corrected. Initial and final logs are retained. No backend execution logic or model configuration was changed, so this work does not establish new model-performance or endurance results.

## Reproduction and rollback

[changes.patch](changes.patch), [previous versions of modified files](before.tar.gz) and [final source hashes](source-after.json) retain the two modified conversation files and nine new component, vocabulary, style, test, preview and guide files. New files can be removed and the two prior files restored from the archive when reverting this change in a separate review checkout; do not overwrite subsequent user edits.

Scratch scripts are under `.local/activity-personality-20260914/`; copies of the visual, package and installation-verification scripts are retained here. Use the repository's Node 24 toolchain and a fresh evidence directory when repeating the workflow. Animation iteration instructions and focused test commands are in the design guide.

Package: `.local/p14-packages/2b087a6025c24d65947bd89725cf86bf/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `9acd0cbbce6698d07022f5cf47d1fd12c894297eeeb96c550d370edf3f451cbf`. The installer retained previous release `0204babfa8784376801d9a746ab7a2f3` for reversible application rollback. The wider combined chat/presentation backlog remains open.
