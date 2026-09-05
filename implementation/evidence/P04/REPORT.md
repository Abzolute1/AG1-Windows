# P04 desktop bridge evidence

P04 delivers a Linux React/Tauri desktop backed by the actual AG1 TypeScript SDK in a separate patched reference copy. The authoritative phase result is [completion.json](completion.json); it records executed commands, input fingerprints, artifact hashes and pending gates. P01–P03 remain prerequisites. This is intermediate bridge behavior, not final Rust or complete UI parity.

## Delivered behavior and corrections

Home/project selection, chat/streaming, model selection, editable files, native Git diff, settings and approval/question dialogs use approved Dyad primitives and locally bundled assets. Monaco is lazy and messages are virtualized. A single supervised SDK worker owns agent behavior; Rust owns native services and separate preferences. Durable command intent/results, bounded queues, snapshots and full evidence support reconnect without replaying mutations.

The reported Settings “Stale revision” failure had two causes: queries unnecessarily changed the revision, and rapidly queued UI commands reused a stale React snapshot. Queries now preserve the revision; the serialized UI queue reads the current authoritative revision at dispatch. Native regression evidence covers repeated navigation and all three model selections. Reopening a workspace uses separate retained worker logs rather than attempting to recreate the same log path.

Exact-request approvals survive renderer reload and authorize a write once. Large requests expose the full persisted arguments through pages before Allow becomes available. UTF-8 and JSON escaping are included in snapshot size bounds. Native saves retain external bytes on conflict. The loading state now displays an active work indicator.

Narrow, hash-verified patches correct B01 aggregate tool-output retention/capping, B02 corrupt memory handling, B03 one-snapshot source provenance and B04 fork artifact closure. D01 guards require live owned process identity and reject foreign endpoint eviction. Original sources, profiles, binaries, libraries and weights are verified read-only. The three tuned llama.cpp configurations retain their exact settings; no automatic tuning, rebuilding or model inference was performed.

## Verification scope

The phase gate runs 19 worker regressions using the real SDK with a fixture provider, six frontend tests, three Rust desktop contracts and six gate-validator tests. It also runs TypeScript checks, frontend and production Rust builds, formatting, Clippy, generated-protocol consistency, reference reproduction, runtime preservation and license/bundle checks. The command logs and final completion record contain actual results. Additional protocol evidence records 26 cross-language/schema round trips and the existing 18 protocol contracts after the bridge DTO changes.

Native evidence includes Settings/model selection and workspace reopen, renderer reload with the same pending approval, one authorized write, full large-request review, file conflict preservation, opaque static preview isolation, and SDK streaming through a three-second renderer pause. These checks use private fixture state; they do not validate live model quality or throughput.

## Visual review

[Open the comparison sheet](visual-comparison.html) for 32 native screenshots: home, chat, dirty files, diff, settings, dialog, loading and error, in light and dark themes at 1440 × 900 and scale 1. [visual-review.json](visual-review.json) identifies the reviewed image hashes.

AG1 retains the reference's typography, colors and primitive controls, with a workspace context column and full-width file editor. Dyad shows a split preview and an empty fixture chat; AG1 shows fixture SDK messages. AG1's diff is readable native Git text rather than Dyad's side-by-side Monaco review. The dialog content reflects opening an existing workspace. Loading and error fixtures represent each app's actual workflow, rather than identical content. No pixel identity or full A01–A85 parity is claimed.

Review caught a collapsed Dyad editor pane; those captures were retaken with geometry and dirty-state assertions. AG1's ambiguous loading presentation and dim dark-theme error text were corrected and recaptured. The deliberately plain static preview fixture tests isolation and is not the app's home screen. AG1 was restored to a fresh conversation and the reference Dyad window was closed.

## Measurements

The measurements include each application's descendant processes, including the AG1 worker. PSS avoids the shared-page double counting of summed RSS. CPU is percent of one logical core; process counts and PSS below are the final sample.

| Workload | AG1 PSS / CPU / processes | Dyad PSS / CPU / processes |
| --- | --- | --- |
| Five-minute idle | 465.5 MiB / 0.97% / 5 | 578.2 MiB / 3.94% / 8 |
| Monaco and preview, 20 seconds | 541.6 MiB / 1.35% / 5 | 713.9 MiB / 0.60% / 9 |
| Renderer pause, 8-second observation | 609.2 MiB / 39.70% / 5 | 723.0 MiB / 38.42% / 9 |

Single warm-cache packaged startup samples were 392 ms for AG1 to its home heading and 2,279 ms for Dyad to its initial DOM. These have different readiness criteria and diagnostic controller overhead; they are not a precise speed ratio. Idle observations lasted 300.04 and 300.08 seconds respectively. Measurements used earlier builds in this P04 series; final visual captures include the subsequent loading and error contrast corrections. Final production staging excludes the diagnostic broker.

The paused workloads differ: AG1 streamed from its SDK fixture while Dyad retained Monaco and a running preview without an LLM stream. Builds and occasional manual interactions occurred on the same machine. GPU memory and processes that exited between samples are not fully captured. These are development observations, not controlled final-release benchmarks.

## Licensing, packaging and remaining gates

The renderer audit traces actual bundled imports, TypeScript/type dependencies, CSS build inputs, Monaco worker/lazy code, fonts and emitted assets: 1,222 files, 30 package license sets and 12 assets in the recorded build. Exact retained license files and the Dyad extraction ledger accompany the product. The Cargo manifest covers 441 locked packages, including MPL source/license obligations. No Dyad Pro code is imported. Apple SDK-derived binding distribution review remains pending for macOS.

Local staging contains the production-feature desktop and process helper plus license records and a launcher. It depends on the separate verified SDK tree, pinned Node, system WebKitGTK and preserved external runtimes. It is not a standalone installer and does not redistribute that SDK dependency tree, runtimes or weights.

Linux validation required X11 and the documented DMABUF workaround. Wayland failed on this host; Windows/macOS, live inference, interactive previews, broader mode/history surfaces, final performance targets and Rust authority cutover remain explicit gates in [limitations.json](limitations.json). Pinned Dyad's deep-route reload failure and its production inspector restriction are recorded separately; the supported E2E package supplied comparison evidence.

Rollback drains/stops only owned worker processes and retains private journals, evidence and per-launch logs. The immutable original reference remains available for supported export. Original user state and tuned runtime settings must not be overwritten. P05 may next build isolated Rust storage without changing the executing TS authority.
