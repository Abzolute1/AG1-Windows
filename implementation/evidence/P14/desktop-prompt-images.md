# Native prompt, empty chat indexing and image input — 2026-09-09

Implementation and installation receipt: [desktop-prompt-images.json](desktop-prompt-images.json). This is a desktop correction for user review, not P14 release acceptance. No 8-hour or 24-hour run was started.

## Prompt

The previous desktop default was a captured 16,514-character prompt containing a Sep 7 Cruzible directory, old skill paths and unavailable tool guidance. The native runtime now constructs its default from the current active registry, workspace and model identity. Only the exact known captured default is retired automatically; deliberate custom system prompts remain replacement prompts. Project instructions, discovered skills, extension instructions and user append files are preserved. A host conversation appendix augments rather than masks APPEND_SYSTEM.md.

Repeated routing/catalog guidance is consolidated. Artifact readCall/nextCall handling, historical evidence freshness, verification and authorization boundaries remain explicit. Guidance distinguishes project memory, current task state, phase history, delegated deliverables and read-only exploration. Failed exact-text edits should reread their target, not guess. Ordinary conversation should receive a direct response without manufactured tool work.

Qwen's running `/tokenize` endpoint measured the old captured base at **3,738 tokens / 16,514 characters**. An offline reconstruction of the new base with the **same 28 active tool names** measured **1,171 tokens / 5,801 characters**, a **68.7% reduction**. Tool schemas were unchanged. This compares system guidance, not total request tokens or measured latency; resource suffixes vary by workspace. The old stale skill/context material is intentionally absent, while actual discovered resources remain supported. No claim of measured quality improvement or eliminated cold loading is made.

## Chat deletion

Native session catalog entries distinguish empty sessions from conversations containing user messages. Empty drafts are not newly saved in the desktop chat catalog. Existing empty metadata is hidden when refreshed. Deleting an active conversation can create a private working session while leaving no replacement sidebar row; the model host remains open. Deleted conversation metadata and its journal remain recoverable. A real conversation named New chat is not classified from its title.

## Image input and display

Pasting supported clipboard images creates removable 88×72 thumbnails. Both draft and sent images open an accessible preview with original dimensions, fit view and 1:1 scrolling. Preview rendering does not resize the bytes sent to the provider. Sent previews read bounded journal pages near the viewport, not multi-megabyte payloads in repeated view polls; object URLs are released when their components leave the conversation.

The existing limits remain eight attachments and 2 MiB of attached file bytes per message. PNG, JPEG, WebP and GIF are supported. Native image admission now accepts up to 32 Mi pixels with either axis at most 16,384 pixels, supporting ordinary 4K/8K screenshots while bounding decoded allocation. The previous 2,048-pixel axis limit rejected a 2,471×1,263 screenshot during the actual UI test.

An ephemeral Analyzing image status appears for the active image turn while it awaits the first provider output, excluding model loading, verification and approvals. It disappears on output, completion or cancellation. This is request progress, not a claim to observe the model's internal reasoning or an exact encoder-completion event.

## Restored local vision configuration

The running server inspected during this task used the non-MTP 35B model but was launched with `--no-mmproj`; `/props` reported `vision: false`, and all saved models advertised text only. Rust already includes screenshot_url, interact_then_screenshot and verify_ui. Visual tool execution checks the selected model's image capability; verify_ui returns browser evidence for caller assessment, not an implicit independent judge.

The matching existing projector was found at `/home/alex/.ag1/agent/models-disabled/lmstudio-community-nonMTP/Qwen3.6-35B-A3B-GGUF/mmproj-Qwen3.6-35B-A3B-BF16.gguf`. Its SHA-256 is `e5c205cec2fd28f66c3895e4040021ab994b860323c3db8531640305ff49b322`, exactly matching the [published Hugging Face projector](https://huggingface.co/lmstudio-community/Qwen3.6-35B-A3B-GGUF/blob/main/mmproj-Qwen3.6-35B-A3B-BF16.gguf). No download was needed.

A separate sealed desktop launch capture adds that verified projector and replaces only the 35B `--no-mmproj` argument with `--mmproj <path> --mmproj-offload`, matching legacy argument construction. The 35B model input metadata becomes text/image. Context, sampling, weight offload and the other two model plans remain unchanged. Original P13 capture, model-profile and bound process-host pins remain untouched. The native host admitted this candidate without requesting model startup or generation. Its startup identity check requires vision when the plan declares a projector.

The already open user app/server was not interrupted. The new configuration takes effect on full app restart. Actual image interpretation and GPU memory behavior with the projector loaded are **not yet live-verified**; the end-to-end UI/transport check uses a controlled vision fixture.

## Rendering and platform behavior

Unchanged Markdown blocks are memoized, so timer/progress updates do not repeatedly parse the same content. The shared conversation follows content resizing through browser requestAnimationFrame, which follows the browser/display scheduler without a fixed 60/120/165 Hz JavaScript loop. Native scrolling remains browser-owned.

On this Linux installation, WebKitGTK 2.52.5 exposed PreferPageRenderingUpdatesNear60FPS as enabled. The Linux adapter disables that preference through the public WebKitFeature API and verifies the readback. It requests hardware acceleration and smooth scrolling. Runtime lookup allows older WebKit versions without this feature API to launch normally. The adapter and its dependency are Linux-only; Windows WebView2 and macOS WKWebView retain their native schedulers. No platform is assigned a hardcoded refresh rate.

A private native WebKit/Xvfb run verified the setting was disabled and recorded zero JavaScript errors. A synthetic 160-turn DOM scrolling sample recorded a 16 ms median and 17 ms p95 over 120 animation frames on this software-backed virtual display. Experimental content visibility produced the same median/p95, so it was not shipped: this trial did not justify introducing estimated-height scroll risks. This is not proof of 165 Hz on the physical monitor, a before/after physical-display benchmark, or Windows/macOS performance acceptance. Those require platform-specific measurement.

The existing Linux X11/DMABUF launcher workaround remains because an earlier native Wayland launch failed with GDK Error 71. Disabling the WebKit preference removes one known rendering constraint; it does not guarantee the compositor and graphics driver deliver every monitor refresh.

## Validation and rollback

52 frontend tests, native prompt contracts, custom/default/migrated session request checks, provider request parity checks, and desktop bridge tests passed. Private Xvfb/WebKit tests cover original image bytes reaching the provider, draft and sent previews, transient image progress, and deletion of three conversations without replacement rows. A production launcher smoke and installed file verification are retained with the receipt.

Private logs, screenshots, tokenizer probe, configuration delta, backups and the guarded restore script are under `.local/p14-prompt-refinement/`. The restore script refuses to overwrite later configuration edits. See [desktop rollback instructions](../../DESKTOP-UI-ROLLBACK.md) for application/source rollback. Original and new runtime captures are retained separately; changing the desktop pointer does not mutate the accepted capture.
